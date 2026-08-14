//! Web module
//!
//! ```text
//! http::Request -> Maki -> http::Response
//! ```
//!
//! ### Error Handling
//!
//! Web errors describe failures at the HTTP/domain boundary.
//! `into_response` owns the web error -> HTTP error response policy.
//!
//! ```text
//! maki::Error ─┐
//! http::Error ─┼─> web::Error ──> http::Response
//! io::Error   ─┘
//! ```

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use percent_encoding::percent_decode_str;

use crate::http::Response;
use crate::maki;
use crate::maki::{HomeMode, Maki, MakiConfigOverrides, MakiRoute};
use crate::{RunError, http};

const MAX_REQUEST_HEAD_SIZE: usize = 16 * 1024;
const LIVE_RELOAD_PATH: &str = "/.maki/events";
const SEARCH_INDEX_PATH: &str = "/.maki/search-index.json";
const SEARCH_PATH: &str = "/.maki/search";
const SEARCH_PAGE_RESULT_LIMIT: usize = 50;
const MAX_SSE_CLIENTS: usize = 16;
const FILE_WATCH_INTERVAL: Duration = Duration::from_millis(500);
const FILE_WATCH_DEBOUNCE: Duration = Duration::from_millis(300);
const SSE_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

struct AppState {
    root: PathBuf,
    config_overrides: MakiConfigOverrides,
    maki: RwLock<Maki>,
    live_reload: LiveReload,
}

impl AppState {
    #[cfg(test)]
    fn new(maki: Maki) -> Self {
        Self::new_with_overrides(maki, MakiConfigOverrides::default())
    }

    fn new_with_overrides(maki: Maki, config_overrides: MakiConfigOverrides) -> Self {
        Self {
            root: maki.root().to_path_buf(),
            config_overrides,
            maki: RwLock::new(maki),
            live_reload: LiveReload::new(MAX_SSE_CLIENTS),
        }
    }

    fn reload(&self) -> Result<(), maki::Error> {
        let mut config = maki::MakiConfig::load_project(&self.root)?;
        self.config_overrides.apply_to(&mut config);
        let next = Maki::load_with_config(&self.root, config)?;
        let mut maki = self
            .maki
            .write()
            .map_err(|_| maki::Error::ReadDirectoryFailed(self.root.to_path_buf()))?;
        *maki = next;
        self.live_reload.broadcast_reload();
        Ok(())
    }
}

#[derive(Debug, PartialEq, Clone)]
enum LiveReloadEvent {
    Reload { version: u64 },
}

#[derive(Debug, PartialEq)]
enum LiveReloadError {
    TooManyClients,
    StatePoisoned,
}

struct LiveClient {
    id: u64,
    sender: mpsc::Sender<LiveReloadEvent>,
}

struct LiveReload {
    boot_id: u128,
    version: AtomicU64,
    next_client_id: AtomicU64,
    max_clients: usize,
    clients: Mutex<Vec<LiveClient>>,
}

impl LiveReload {
    fn new(max_clients: usize) -> Self {
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

    fn token(&self) -> String {
        self.token_for(self.version())
    }

    fn token_for(&self, version: u64) -> String {
        format!("{}:{version}", self.boot_id)
    }

    fn register_client(
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

    fn broadcast_reload(&self) {
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
}

impl Drop for LiveClientRegistration<'_> {
    fn drop(&mut self) {
        self.live_reload.unregister_client(self.id);
    }
}

#[derive(Debug)]
enum Error {
    #[allow(dead_code)]
    Io {
        source: std::io::Error,
    },
    InvalidRequest {
        #[allow(dead_code)]
        source: http::Error,
    },
    TooLongRequest,
    ZeroLengthRequest,
    BadRequest,
    Maki {
        source: maki::Error,
    },
}

fn internal_server_error(e: &Error) -> Response {
    Response::new(http::StatusCode::InternalServerError)
        .set_header("content-type", "text/plain")
        .set_body(format!("Internal Server Error: {}", e))
}

fn not_found(e: &Error) -> Response {
    Response::new(http::StatusCode::NotFound)
        .set_header("content-type", "text/plain")
        .set_body(format!("Not Found: {}", e))
}

fn bad_request(e: &Error) -> Response {
    Response::new(http::StatusCode::BadRequest)
        .set_header("content-type", "text/plain")
        .set_body(format!("Bad Request: {}", e))
}

impl Error {
    fn into_response(self) -> Response {
        match self {
            e @ Error::Maki {
                source: maki::Error::NoteNotFound(..),
            } => not_found(&e),
            e @ Error::Maki {
                source: maki::Error::InvalidNotePath(..),
            }
            | e @ Error::InvalidRequest { .. }
            | e @ Error::TooLongRequest
            | e @ Error::BadRequest
            | e @ Error::ZeroLengthRequest => bad_request(&e),
            e @ Error::Io { .. } | e @ Error::Maki { .. } => internal_server_error(&e),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io { source: error }
    }
}

impl From<maki::Error> for Error {
    fn from(error: maki::Error) -> Self {
        Self::Maki { source: error }
    }
}

impl From<http::Error> for Error {
    fn from(error: http::Error) -> Self {
        Self::InvalidRequest { source: error }
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

fn inject_live_reload_script(mut html: String, token: &str) -> String {
    let script = live_reload_script(token);

    if let Some(index) = html.rfind("</body>") {
        html.insert_str(index, &script);
    } else {
        html.push_str(&script);
    }

    html
}

struct RequestTarget<'a> {
    path: String,
    query: Option<&'a str>,
}

fn parse_request_target(target: &str) -> Result<RequestTarget<'_>, Error> {
    let (raw_path, query) = target
        .split_once('?')
        .map_or((target, None), |(path, query)| (path, Some(query)));
    let path = percent_decode_str(raw_path)
        .decode_utf8()
        .map_err(|_e| Error::BadRequest)?
        .to_string();

    Ok(RequestTarget { path, query })
}

fn decode_query_component(raw: &str) -> Result<String, Error> {
    percent_decode_str(&raw.replace('+', " "))
        .decode_utf8()
        .map(|decoded| decoded.to_string())
        .map_err(|_e| Error::BadRequest)
}

fn query_param(query: Option<&str>, name: &str) -> Result<Option<String>, Error> {
    let Some(query) = query else {
        return Ok(None);
    };

    for part in query.split('&') {
        let (raw_key, raw_value) = part.split_once('=').unwrap_or((part, ""));
        if decode_query_component(raw_key)? == name {
            return Ok(Some(decode_query_component(raw_value)?));
        }
    }

    Ok(None)
}

fn escape_json_string(input: &str) -> String {
    let mut output = String::new();

    for ch in input.chars() {
        match ch {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if ch <= '\u{1f}' => {
                let _ = write!(output, "\\u{:04x}", ch as u32);
            }
            _ => output.push(ch),
        }
    }

    output
}

fn search_index_json(entries: &[maki::SearchEntry]) -> String {
    let mut json = String::from("[");

    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }

        json.push_str("{\"title\":\"");
        json.push_str(&escape_json_string(entry.title()));
        json.push_str("\",\"path\":\"");
        json.push_str(&escape_json_string(entry.path()));
        json.push_str("\",\"source_path\":\"");
        json.push_str(&escape_json_string(entry.source_path()));
        json.push_str("\"}");
    }

    json.push(']');
    json
}

fn handle_request(state: &AppState, request: &http::Request) -> Result<http::Response, Error> {
    let target = parse_request_target(request.target())?;

    if PathBuf::from(&target.path)
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(Error::BadRequest);
    }

    let maki = state.maki.read().map_err(|_| Error::Maki {
        source: maki::Error::ReadDirectoryFailed(state.root.clone()),
    })?;

    if target.path == SEARCH_INDEX_PATH {
        return Ok(http::Response::new(http::StatusCode::Ok)
            .set_header("Content-Type", "application/json; charset=utf-8")
            .set_body(search_index_json(maki.search_entries())));
    }

    if target.path == SEARCH_PATH {
        let query = query_param(target.query, "q")?.unwrap_or_default();
        let results = maki.search_titles(&query, SEARCH_PAGE_RESULT_LIMIT);
        let html = crate::html::render_search_page(&query, &results, maki.search_entries().len());
        return Ok(http::Response::new(http::StatusCode::Ok)
            .set_header("Content-Type", "text/html; charset=utf-8")
            .set_body(inject_live_reload_script(html, &state.live_reload.token())));
    }

    match maki.resolve_route(&target.path) {
        Ok(MakiRoute::NotePage(path)) => {
            let html = maki.render_html(&path)?;
            Ok(http::Response::new(http::StatusCode::Ok)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(inject_live_reload_script(html, &state.live_reload.token())))
        }
        Ok(MakiRoute::NoteSource(path)) => Ok(http::Response::new(http::StatusCode::Ok)
            .set_header("Content-Type", "text/plain; charset=utf-8")
            .set_body(maki.get_raw_content(&path)?)),
        Ok(MakiRoute::Home) => match &maki.config().home_mode() {
            HomeMode::Redirect(path) => Ok(http::Response::new(http::StatusCode::Found)
                .set_header("Location", path)
                .set_header("Content-Type", "text/plain; charset=utf-8")
                .set_body(path.as_bytes())),
        },
        Err(e) => Err(e.into()),
    }
}

fn read_request_head(stream: &mut impl Read) -> Result<Vec<u8>, Error> {
    // TODO: 최적화 가능
    // 매 요청마다 버퍼, Vec 새로 만들지 않고 만들어진 것 쓰기
    // 단, keep-alive 지원할 경우, 그에 대해 고려해야함
    let mut request = Vec::with_capacity(4096);
    let mut buffer = [0u8; 1024];
    loop {
        let bytes_read = stream.read(&mut buffer)?;

        if bytes_read == 0 {
            return Err(Error::ZeroLengthRequest);
        }

        request.extend_from_slice(&buffer[..bytes_read]);

        // TODO: 헤더 경계 찾기 최적화 가능
        // 전체를 훑지 말고 최근에 받은 내용 중에서 훑기
        // buffer만 보면 안됨. \r\n | \r\n 이렇게 끊어서 올 수도 있으니까.
        if request.windows(4).any(|w| w == b"\r\n\r\n") {
            return Ok(request);
        }

        if request.len() > MAX_REQUEST_HEAD_SIZE {
            return Err(Error::TooLongRequest);
        }
    }
}

fn read_request(stream: &mut impl Read) -> Result<http::Request, Error> {
    let raw_request = read_request_head(stream)?;
    // TODO: header만 잘라서 먼저 utf8로 변환하기
    let request = String::from_utf8_lossy(&raw_request);
    let request = http::parse_request(&request)?;
    Ok(request)
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
) -> Result<(), RunError> {
    write!(stream, "event: {event}\ndata: {data}\n\n")
        .map_err(|source| RunError::IoError { source })
}

fn write_sse_keepalive(stream: &mut impl Write) -> Result<(), RunError> {
    stream
        .write_all(b": keepalive\n\n")
        .map_err(|source| RunError::IoError { source })
}

fn handle_live_reload_connection<S>(state: &AppState, stream: &mut S) -> Result<(), RunError>
where
    S: Write,
{
    let (client_id, token, receiver) = match state.live_reload.register_client() {
        Ok(client) => client,
        Err(LiveReloadError::TooManyClients) => {
            let response = service_unavailable("Too many live reload clients");
            return stream
                .write_all(&response.to_bytes())
                .map_err(|source| RunError::IoError { source });
        }
        Err(LiveReloadError::StatePoisoned) => {
            let response = internal_server_error(&Error::Maki {
                source: maki::Error::ReadDirectoryFailed(state.root.clone()),
            });
            return stream
                .write_all(&response.to_bytes())
                .map_err(|source| RunError::IoError { source });
        }
    };
    let _registration = LiveClientRegistration {
        live_reload: &state.live_reload,
        id: client_id,
    };

    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n",
        )
        .map_err(|source| RunError::IoError { source })?;
    write_sse_event(stream, "hello", token)?;

    loop {
        match receiver.recv_timeout(SSE_KEEPALIVE_INTERVAL) {
            Ok(LiveReloadEvent::Reload { version }) => {
                write_sse_event(stream, "reload", state.live_reload.token_for(version))?;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                write_sse_keepalive(stream)?;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

fn handle_connection<S>(state: &AppState, stream: &mut S) -> Result<(), RunError>
where
    S: Write + Read,
{
    let request = match read_request(stream) {
        Ok(request) => request,
        Err(err) => {
            let response = err.into_response();
            return stream
                .write_all(&response.to_bytes())
                .map_err(|source| RunError::IoError { source });
        }
    };

    if request.target() == LIVE_RELOAD_PATH {
        return handle_live_reload_connection(state, stream);
    }

    let response = match handle_request(state, &request) {
        Ok(response) => response,
        Err(err) => err.into_response(),
    };

    stream
        .write_all(&response.to_bytes())
        .map_err(|source| RunError::IoError { source })
}

#[derive(Debug, PartialEq, Eq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

type FileSnapshot = BTreeMap<PathBuf, FileStamp>;

fn collect_maki_file_snapshot(root: &Path) -> Result<FileSnapshot, std::io::Error> {
    fn collect(root: &Path, current: &Path, acc: &mut FileSnapshot) -> Result<(), std::io::Error> {
        for entry in std::fs::read_dir(current)? {
            let entry = entry?;
            let file_name = entry.file_name();
            if file_name.to_string_lossy().starts_with('.') {
                continue;
            }

            let path = entry.path();
            if path.is_dir() {
                collect(root, &path, acc)?;
                continue;
            }

            if !path.is_file() {
                continue;
            }

            let relative_path = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            let is_maki_note = path.extension().is_some_and(|ext| ext == "maki");
            let is_project_file = relative_path == Path::new(maki::PROJECT_FILE_NAME);
            if !is_maki_note && !is_project_file {
                continue;
            }

            let metadata = path.metadata()?;
            acc.insert(
                relative_path,
                FileStamp {
                    modified: metadata.modified().ok(),
                    len: metadata.len(),
                },
            );
        }

        Ok(())
    }

    let mut snapshot = BTreeMap::new();
    collect(root, root, &mut snapshot)?;
    Ok(snapshot)
}

fn spawn_file_watcher(state: Arc<AppState>) {
    thread::spawn(move || {
        let mut snapshot = match collect_maki_file_snapshot(&state.root) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                eprintln!("Failed to initialize file watcher: {}", error);
                FileSnapshot::new()
            }
        };

        loop {
            thread::sleep(FILE_WATCH_INTERVAL);

            let next_snapshot = match collect_maki_file_snapshot(&state.root) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    eprintln!("Failed to scan maki files: {}", error);
                    continue;
                }
            };

            if next_snapshot == snapshot {
                continue;
            }

            thread::sleep(FILE_WATCH_DEBOUNCE);

            let stable_snapshot = match collect_maki_file_snapshot(&state.root) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    eprintln!("Failed to scan maki files after debounce: {}", error);
                    continue;
                }
            };

            if stable_snapshot != next_snapshot {
                continue;
            }

            match state.reload() {
                Ok(()) => {
                    snapshot = stable_snapshot;
                }
                Err(error) => {
                    eprintln!("Failed to reload maki files: {}", error);
                }
            }
        }
    });
}

pub(crate) fn serve(
    maki: Maki,
    host: &str,
    port: u16,
    config_overrides: MakiConfigOverrides,
) -> Result<(), RunError> {
    let listener =
        TcpListener::bind((host, port)).map_err(|source| RunError::IoError { source })?;
    let state = Arc::new(AppState::new_with_overrides(maki, config_overrides));
    spawn_file_watcher(Arc::clone(&state));

    println!("Listening on http://{}:{}", host, port);

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(source) => {
                eprintln!("Failed to accept connection: {}", source);
                continue;
            }
        };

        let state = Arc::clone(&state);
        thread::spawn(move || {
            if let Err(error) = handle_connection(&state, &mut stream) {
                eprintln!("Failed to handle connection: {}", error);
            }
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::Duration;

    use crate::web::*;

    #[test]
    fn test_render_not_found_response() {
        let response = http::Response::new(http::StatusCode::NotFound)
            .set_header("Content-Type", "text/plain; charset=utf-8")
            .set_body("Not Found".to_string());
        assert_eq!(response.status(), http::StatusCode::NotFound);
        assert_eq!(response.body(), b"Not Found");
        assert_eq!(
            response.get_header("Content-Type"),
            Some("text/plain; charset=utf-8")
        );
    }

    #[test]
    fn test_read_request_with_split_header() {
        let mut input = &b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n"[..];
        let raw = read_request_head(&mut input).unwrap();
        assert!(raw.ends_with(b"\r\n\r\n"));
    }

    #[test]
    fn test_handle_unknown_path_returns_not_found() {
        let request = http::Request::get("/missing");

        let maki = Maki::load(PathBuf::from(".")).unwrap();
        let state = AppState::new(maki);

        let response = handle_request(&state, &request);

        assert!(matches!(
            response,
            Err(Error::Maki {
                source: maki::Error::NoteNotFound(..)
            })
        ));
    }

    #[test]
    fn test_rendered_note_includes_live_reload_script() {
        let maki = Maki::load(PathBuf::from("docs")).unwrap();
        let state = AppState::new(maki);
        let request = http::Request::get("/index");

        let response = handle_request(&state, &request).unwrap();
        let body = String::from_utf8(response.body().to_vec()).unwrap();

        assert!(body.contains("new EventSource(\"/.maki/events\")"));
        assert!(body.contains("</script></body>"));
    }

    #[test]
    fn test_source_note_does_not_include_live_reload_script() {
        let maki = Maki::load(PathBuf::from("docs")).unwrap();
        let state = AppState::new(maki);
        let request = http::Request::get("/index.maki");

        let response = handle_request(&state, &request).unwrap();
        let body = String::from_utf8(response.body().to_vec()).unwrap();

        assert!(!body.contains("new EventSource(\"/.maki/events\")"));
    }

    #[test]
    fn test_search_index_returns_note_titles() {
        let maki = Maki::load(PathBuf::from("docs")).unwrap();
        let state = AppState::new(maki);
        let request = http::Request::get("/.maki/search-index.json");

        let response = handle_request(&state, &request).unwrap();
        let body = String::from_utf8(response.body().to_vec()).unwrap();

        assert_eq!(
            response.get_header("Content-Type"),
            Some("application/json; charset=utf-8")
        );
        assert!(body.contains("\"title\":\"Maki Syntax\""));
        assert!(body.contains("\"path\":\"/maki-syntax\""));
    }

    #[test]
    fn test_search_page_returns_matching_titles() {
        let maki = Maki::load(PathBuf::from("docs")).unwrap();
        let state = AppState::new(maki);
        let request = http::Request::get("/.maki/search?q=syntax");

        let response = handle_request(&state, &request).unwrap();
        let body = String::from_utf8(response.body().to_vec()).unwrap();

        assert!(body.contains("<title>Search</title>"));
        assert!(body.contains("<a href=\"/maki-syntax\">Maki Syntax</a>"));
        assert!(body.contains("new EventSource(\"/.maki/events\")"));
    }

    #[test]
    fn test_search_index_escapes_json_strings() {
        let root = std::env::temp_dir().join(format!("maki-search-json-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("quote.maki"), "--^ title: Quote \"Note\"\n").unwrap();

        let maki = Maki::load(&root).unwrap();
        let state = AppState::new(maki);
        let request = http::Request::get("/.maki/search-index.json");

        let response = handle_request(&state, &request).unwrap();
        let body = String::from_utf8(response.body().to_vec()).unwrap();

        fs::remove_dir_all(root).unwrap();
        assert!(body.contains("Quote \\\"Note\\\""));
    }

    #[test]
    fn test_live_reload_rejects_clients_over_limit() {
        let live_reload = LiveReload::new(1);

        assert!(live_reload.register_client().is_ok());
        assert!(matches!(
            live_reload.register_client(),
            Err(LiveReloadError::TooManyClients)
        ));
    }

    #[test]
    fn test_live_reload_broadcasts_reload() {
        let live_reload = LiveReload::new(1);
        let (_client_id, _version, receiver) = live_reload.register_client().unwrap();

        live_reload.broadcast_reload();

        assert_eq!(
            receiver.recv_timeout(Duration::from_millis(100)).unwrap(),
            LiveReloadEvent::Reload { version: 1 }
        );
    }

    #[test]
    fn test_empty_request() {
        let mut input = &b""[..];

        assert!(matches!(
            read_request_head(&mut input),
            Err(Error::ZeroLengthRequest)
        ))
    }

    #[test]
    fn test_too_long_request() {
        let bytes = vec![b'a'; MAX_REQUEST_HEAD_SIZE + 1];
        let mut input = &bytes[..];

        assert!(matches!(
            read_request_head(&mut input),
            Err(Error::TooLongRequest)
        ))
    }
}
