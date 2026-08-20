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

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::RunError;
use crate::metrics::Metrics;
use maki_core::{Maki, MakiConfigOverrides};

mod cache_warmer;
mod error;
mod live_reload;
mod routes;
mod server;
mod state;
mod target;
mod watch;

#[cfg(test)]
mod tests;

pub use state::ProjectReloader;

const MAX_REQUEST_HEAD_SIZE: usize = 16 * 1024;
const META_PATH: &str = "/@/";
const META_PATH_NO_SLASH: &str = "/@";
const RECENTS_PATH: &str = "/@/recents";
const RECENTS_PATH_WITH_SLASH: &str = "/@/recents/";
const DIAGNOSTICS_PATH: &str = "/@/diagnostics";
const DIAGNOSTICS_PATH_WITH_SLASH: &str = "/@/diagnostics/";
const DATES_PATH: &str = "/@/dates";
const DATES_PATH_WITH_SLASH: &str = "/@/dates/";
const DATES_PATH_PREFIX: &str = "/@/dates/";
const LIVE_RELOAD_PATH: &str = "/.maki/events";
const SEARCH_INDEX_PATH: &str = "/.maki/search-index.json";
const SEARCH_PATH: &str = "/.maki/search";
const SEARCH_PAGE_RESULT_LIMIT: usize = 50;
const MAX_SSE_CLIENTS: usize = 16;
const FILE_WATCH_INTERVAL: Duration = Duration::from_millis(500);
const FILE_WATCH_DEBOUNCE: Duration = Duration::from_millis(300);
const SSE_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct MetricsEndpoint {
    pub host: String,
    pub port: u16,
}

pub enum ServeRuntime {
    Development,
    Publish,
}

pub struct ServeConfig<'a> {
    pub host: &'a str,
    pub port: u16,
    pub config_overrides: MakiConfigOverrides,
    pub runtime: ServeRuntime,
    pub metrics: Metrics,
    pub metrics_endpoint: Option<MetricsEndpoint>,
}

pub fn serve_project(
    maki: Maki,
    project_root: PathBuf,
    host: &str,
    port: u16,
    config_overrides: MakiConfigOverrides,
    metrics: Metrics,
    metrics_endpoint: Option<MetricsEndpoint>,
) -> Result<(), RunError> {
    serve_with_runtime(
        maki,
        project_root,
        ServeConfig {
            host,
            port,
            config_overrides,
            runtime: ServeRuntime::Development,
            metrics,
            metrics_endpoint,
        },
        |_| {},
    )
}

pub fn serve_with_runtime<F>(
    maki: Maki,
    project_root: PathBuf,
    config: ServeConfig<'_>,
    setup: F,
) -> Result<(), RunError>
where
    F: FnOnce(ProjectReloader),
{
    let ServeConfig {
        host,
        port,
        config_overrides,
        runtime,
        metrics,
        metrics_endpoint,
    } = config;

    let listener =
        TcpListener::bind((host, port)).map_err(|source| RunError::IoError { source })?;
    let metrics_listener = metrics_endpoint
        .as_ref()
        .map(|endpoint| TcpListener::bind((endpoint.host.as_str(), endpoint.port)))
        .transpose()
        .map_err(|source| RunError::IoError { source })?;
    let live_reload = matches!(runtime, ServeRuntime::Development);
    let state = Arc::new(state::AppState::new_with_overrides(
        project_root,
        maki,
        config_overrides,
        live_reload,
        metrics.clone(),
    ));
    if let (Some(listener), Some(endpoint)) = (metrics_listener, metrics_endpoint) {
        server::spawn_metrics_listener(listener, metrics, endpoint);
    }
    if live_reload {
        watch::spawn_file_watcher(Arc::clone(&state));
    }
    setup(ProjectReloader::new(Arc::clone(&state)));
    cache_warmer::spawn_response_cache_warmer(Arc::clone(&state));

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
            if let Err(error) = server::handle_connection(&state, &mut stream) {
                eprintln!("Failed to handle connection: {}", error);
            }
        });
    }

    Ok(())
}
