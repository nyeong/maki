use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::RunError;
use crate::http::{self, Response};
use crate::metrics::Metrics;
use maki_core::Error as MakiError;

use super::error::{Error, internal_server_error, not_found};
use super::server::write_response;
use super::state::AppState;
use super::{LIVE_RELOAD_PATH, SSE_KEEPALIVE_INTERVAL};

#[derive(Debug, PartialEq, Clone)]
pub(super) enum LiveReloadEvent {
    Reload { version: u64 },
}

#[derive(Debug, PartialEq)]
pub(super) enum LiveReloadError {
    TooManyClients,
    StatePoisoned,
}

struct LiveClient {
    id: u64,
    sender: mpsc::Sender<LiveReloadEvent>,
}

pub(super) struct LiveReload {
    boot_id: u128,
    version: AtomicU64,
    next_client_id: AtomicU64,
    max_clients: usize,
    clients: Mutex<Vec<LiveClient>>,
}

impl LiveReload {
    pub(super) fn new(max_clients: usize) -> Self {
        Self {
            boot_id: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0),
            version: AtomicU64::new(0),
            next_client_id: AtomicU64::new(1),
            max_clients,
            clients: Mutex::new(Vec::new()),
        }
    }

    fn version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }

    pub(super) fn token(&self) -> String {
        self.token_for(self.version())
    }

    pub(super) fn token_for(&self, version: u64) -> String {
        format!("{}:{version}", self.boot_id)
    }

    pub(super) fn register_client(
        &self,
    ) -> Result<(u64, String, mpsc::Receiver<LiveReloadEvent>), LiveReloadError> {
        let mut clients = self
            .clients
            .lock()
            .map_err(|_| LiveReloadError::StatePoisoned)?;

        if clients.len() >= self.max_clients {
            return Err(LiveReloadError::TooManyClients);
        }

        let id = self.next_client_id.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = mpsc::channel();
        clients.push(LiveClient { id, sender });

        Ok((id, self.token(), receiver))
    }

    fn unregister_client(&self, id: u64) {
        let Ok(mut clients) = self.clients.lock() else {
            return;
        };

        clients.retain(|client| client.id != id);
    }

    pub(super) fn client_count(&self) -> usize {
        self.clients
            .lock()
            .map(|clients| clients.len())
            .unwrap_or_default()
    }

    pub(super) fn broadcast_reload(&self) {
        let version = self.version.fetch_add(1, Ordering::SeqCst) + 1;
        let Ok(mut clients) = self.clients.lock() else {
            return;
        };

        clients.retain(|client| {
            client
                .sender
                .send(LiveReloadEvent::Reload { version })
                .is_ok()
        });
    }
}

struct LiveClientRegistration<'a> {
    live_reload: &'a LiveReload,
    id: u64,
    metrics: Metrics,
}

impl Drop for LiveClientRegistration<'_> {
    fn drop(&mut self) {
        self.live_reload.unregister_client(self.id);
        self.metrics
            .set_live_reload_clients(self.live_reload.client_count());
    }
}
fn live_reload_script(token: &str) -> String {
    format!(
        r#"<script>(() => {{
const initialToken = "{token}";
const source = new EventSource("/.maki/events");
source.addEventListener("hello", event => {{
  if (event.data && event.data !== initialToken) {{
    location.reload();
  }}
}});
source.addEventListener("reload", () => location.reload());
}})();</script>"#
    )
}

pub(super) fn inject_live_reload_script(mut html: String, token: &str) -> String {
    let script = live_reload_script(token);

    if let Some(index) = html.rfind("</body>") {
        html.insert_str(index, &script);
    } else {
        html.push_str(&script);
    }

    html
}
fn service_unavailable(message: &str) -> Response {
    Response::new(http::StatusCode::ServiceUnavailable)
        .set_header("Content-Type", "text/plain; charset=utf-8")
        .set_body(message.to_string())
}
fn write_sse_event(
    stream: &mut impl Write,
    event: &str,
    data: impl std::fmt::Display,
) -> std::io::Result<()> {
    write!(stream, "event: {event}\ndata: {data}\n\n")
}

fn write_sse_keepalive(stream: &mut impl Write) -> std::io::Result<()> {
    stream.write_all(b": keepalive\n\n")
}

fn is_client_disconnect(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::BrokenPipe
            | ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionReset
            | ErrorKind::NotConnected
            | ErrorKind::UnexpectedEof
    )
}

fn handle_sse_write(
    state: &AppState,
    started: Instant,
    result: std::io::Result<()>,
) -> Result<bool, RunError> {
    match result {
        Ok(()) => Ok(true),
        Err(source) if is_client_disconnect(&source) => {
            state
                .metrics()
                .record_live_reload_disconnect("client", started.elapsed());
            Ok(false)
        }
        Err(source) => {
            state
                .metrics()
                .record_live_reload_disconnect("error", started.elapsed());
            Err(RunError::IoError { source })
        }
    }
}
pub(super) fn handle_live_reload_connection<S>(
    state: &AppState,
    stream: &mut S,
) -> Result<http::StatusCode, RunError>
where
    S: Write,
{
    let Some(live_reload) = state.live_reload() else {
        let response = not_found(&Error::Maki {
            source: MakiError::NoteNotFound(PathBuf::from(LIVE_RELOAD_PATH)),
        });
        return write_response(stream, response);
    };

    let (client_id, token, receiver) = match live_reload.register_client() {
        Ok(client) => client,
        Err(LiveReloadError::TooManyClients) => {
            let response = service_unavailable("Too many live reload clients");
            return write_response(stream, response);
        }
        Err(LiveReloadError::StatePoisoned) => {
            let response = internal_server_error(&Error::Maki {
                source: MakiError::ReadDirectoryFailed(PathBuf::from(".")),
            });
            return write_response(stream, response);
        }
    };
    state
        .metrics()
        .set_live_reload_clients(live_reload.client_count());
    let _registration = LiveClientRegistration {
        live_reload,
        id: client_id,
        metrics: state.metrics().clone(),
    };
    let started = Instant::now();

    if !handle_sse_write(
        state,
        started,
        stream.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n",
        ),
    )? {
        return Ok(http::StatusCode::Ok);
    }
    if !handle_sse_write(state, started, write_sse_event(stream, "hello", token))? {
        return Ok(http::StatusCode::Ok);
    }

    loop {
        match receiver.recv_timeout(SSE_KEEPALIVE_INTERVAL) {
            Ok(LiveReloadEvent::Reload { version }) => {
                if !handle_sse_write(
                    state,
                    started,
                    write_sse_event(stream, "reload", live_reload.token_for(version)),
                )? {
                    return Ok(http::StatusCode::Ok);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !handle_sse_write(state, started, write_sse_keepalive(stream))? {
                    return Ok(http::StatusCode::Ok);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                state
                    .metrics()
                    .record_live_reload_disconnect("server", started.elapsed());
                return Ok(http::StatusCode::Ok);
            }
        }
    }
}
