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

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use percent_encoding::percent_decode_str;

use crate::http::Response;
use crate::metrics::Metrics;
use crate::{RunError, http};
use maki_core::html::{self, AssetMode};
use maki_core::{
    DatePeriod, Error as MakiError, HomeMode, Maki, MakiConfig, MakiConfigOverrides, MakiRoute,
    PROJECT_FILE_NAME, SearchEntry,
};

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

struct AppState {
    project_root: PathBuf,
    config_overrides: MakiConfigOverrides,
    project: RwLock<ProjectState>,
    live_reload: Option<LiveReload>,
    metrics: Metrics,
}

struct ProjectState {
    maki: Maki,
    response_cache: Mutex<BTreeMap<ResponseCacheKey, http::Response>>,
}

impl ProjectState {
    fn new(maki: Maki) -> Self {
        Self {
            maki,
            response_cache: Mutex::new(BTreeMap::new()),
        }
    }

    fn cached_response(&self, key: &ResponseCacheKey) -> Option<http::Response> {
        self.response_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(key).cloned())
    }

    fn insert_response(&self, key: ResponseCacheKey, response: http::Response) -> Option<usize> {
        self.response_cache.lock().ok().map(|mut cache| {
            cache.insert(key, response);
            cache.len()
        })
    }

    #[cfg(test)]
    fn cached_response_count(&self) -> usize {
        self.response_cache
            .lock()
            .map(|cache| cache.len())
            .unwrap_or_default()
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ResponseCacheKey {
    MetaIndex,
    Recents,
    Diagnostics,
    DatesIndex,
    DatePeriodPage(DatePeriod),
    SearchIndex,
    NotePage(PathBuf),
}

impl ResponseCacheKey {
    fn kind(&self) -> &'static str {
        match self {
            Self::MetaIndex => "meta",
            Self::Recents => "recents",
            Self::Diagnostics => "diagnostics",
            Self::DatesIndex => "dates",
            Self::DatePeriodPage(_) => "date",
            Self::SearchIndex => "search_index",
            Self::NotePage(_) => "note",
        }
    }
}

impl AppState {
    #[cfg(test)]
    fn new(maki: Maki) -> Self {
        Self::new_with_overrides(
            maki.root().to_path_buf(),
            maki,
            MakiConfigOverrides::default(),
            true,
            Metrics::disabled(),
        )
    }

    #[cfg(test)]
    fn new_with_metrics(maki: Maki, metrics: Metrics) -> Self {
        Self::new_with_overrides(
            maki.root().to_path_buf(),
            maki,
            MakiConfigOverrides::default(),
            true,
            metrics,
        )
    }

    fn new_with_overrides(
        project_root: PathBuf,
        maki: Maki,
        config_overrides: MakiConfigOverrides,
        live_reload: bool,
        metrics: Metrics,
    ) -> Self {
        metrics.set_project_notes(maki.notes_len());
        metrics.set_response_cache_entries(0);

        Self {
            project_root,
            config_overrides,
            project: RwLock::new(ProjectState::new(maki)),
            live_reload: live_reload.then(|| LiveReload::new(MAX_SSE_CLIENTS)),
            metrics,
        }
    }

    fn reload(&self) -> Result<(), MakiError> {
        let started = Instant::now();
        let mut config = MakiConfig::load_project(&self.project_root)?;
        self.config_overrides.apply_to(&mut config);
        let source_root = config.project_source_root(&self.project_root);
        let result = Maki::load_with_config_metered(&source_root, config, &self.metrics)
            .and_then(|next| self.replace_maki(next));
        let label = if result.is_ok() { "ok" } else { "error" };
        self.metrics
            .record_project_reload("directory", label, started.elapsed());
        result
    }

    fn current_root(&self) -> Result<PathBuf, MakiError> {
        let project = self
            .project
            .read()
            .map_err(|_| MakiError::ReadDirectoryFailed(PathBuf::from(".")))?;

        Ok(project.maki.root().to_path_buf())
    }

    fn replace_maki(&self, next: Maki) -> Result<(), MakiError> {
        let root = next.root().to_path_buf();
        let notes_len = next.notes_len();
        {
            let mut project = self
                .project
                .write()
                .map_err(|_| MakiError::ReadDirectoryFailed(root))?;
            *project = ProjectState::new(next);
        }
        self.metrics.set_project_notes(notes_len);
        self.metrics.set_response_cache_entries(0);
        if let Some(live_reload) = &self.live_reload {
            live_reload.broadcast_reload();
            self.metrics.increment_live_reload_events();
        }
        Ok(())
    }

    fn with_live_reload(&self, html: String) -> String {
        match &self.live_reload {
            Some(live_reload) => inject_live_reload_script(html, &live_reload.token()),
            None => html,
        }
    }

    fn live_reload(&self) -> Option<&LiveReload> {
        self.live_reload.as_ref()
    }

    fn watched_snapshot(&self) -> Result<FileSnapshot, std::io::Error> {
        let source_root = self
            .current_root()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        collect_watched_project_snapshot(&self.project_root, &source_root)
    }

    #[cfg(test)]
    fn cached_response_count(&self) -> usize {
        self.project
            .read()
            .map(|project| project.cached_response_count())
            .unwrap_or_default()
    }

    fn metrics(&self) -> &Metrics {
        &self.metrics
    }
}

#[derive(Clone)]
pub struct ProjectReloader {
    state: Arc<AppState>,
}

impl ProjectReloader {
    pub fn replace_maki(&self, next: Maki) -> Result<(), MakiError> {
        self.state.replace_maki(next)
    }

    pub fn metrics(&self) -> &Metrics {
        self.state.metrics()
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

    fn client_count(&self) -> usize {
        self.clients
            .lock()
            .map(|clients| clients.len())
            .unwrap_or_default()
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
    metrics: Metrics,
}

impl Drop for LiveClientRegistration<'_> {
    fn drop(&mut self) {
        self.live_reload.unregister_client(self.id);
        self.metrics
            .set_live_reload_clients(self.live_reload.client_count());
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
        source: MakiError,
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
                source: MakiError::NoteNotFound(..),
            } => not_found(&e),
            e @ Error::Maki {
                source: MakiError::InvalidNotePath(..),
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

impl From<MakiError> for Error {
    fn from(error: MakiError) -> Self {
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

fn date_period_for_dates_request_path(path: &str) -> Option<DatePeriod> {
    let raw = path.strip_prefix(DATES_PATH_PREFIX)?;

    if raw.is_empty() || raw.contains('/') {
        return None;
    }

    DatePeriod::parse_path_segment(raw)
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

fn search_index_json(entries: &[SearchEntry]) -> String {
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

fn runtime_asset_response(asset: html::RuntimeAsset) -> http::Response {
    let body =
        std::fs::read(asset.source_path()).unwrap_or_else(|_| asset.embedded().as_bytes().to_vec());

    http::Response::new(http::StatusCode::Ok)
        .set_header("Content-Type", asset.content_type())
        .set_header("Cache-Control", "no-cache")
        .set_body(body)
}

fn has_parent_dir_component(path: &str) -> bool {
    PathBuf::from(path)
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}

fn cacheable_response(
    state: &AppState,
    project: &ProjectState,
    key: ResponseCacheKey,
    observe_cache_request: bool,
) -> Result<http::Response, Error> {
    let kind = key.kind();
    if let Some(response) = project.cached_response(&key) {
        if observe_cache_request {
            state.metrics().record_response_cache_request(kind, "hit");
        }
        return Ok(response);
    }

    if observe_cache_request {
        state.metrics().record_response_cache_request(kind, "miss");
    }
    let response = render_cacheable_response(state, &project.maki, &key)?;
    if let Some(entries) = project.insert_response(key, response.clone()) {
        state.metrics().set_response_cache_entries(entries);
    }
    Ok(response)
}

fn render_cacheable_response(
    state: &AppState,
    maki: &Maki,
    key: &ResponseCacheKey,
) -> Result<http::Response, Error> {
    let kind = key.kind();
    let started = Instant::now();
    let result = match key {
        ResponseCacheKey::MetaIndex => {
            let html = html::render_meta_index_page(AssetMode::External);
            Ok(http::Response::new(http::StatusCode::Ok)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(state.with_live_reload(html)))
        }
        ResponseCacheKey::Recents => {
            let html = html::render_recents_page(maki.recent_entries(), AssetMode::External);
            Ok(http::Response::new(http::StatusCode::Ok)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(state.with_live_reload(html)))
        }
        ResponseCacheKey::Diagnostics => {
            let diagnostics = maki.diagnostics_without_external_links();
            let html =
                html::render_diagnostics_page(&diagnostics, maki.notes_len(), AssetMode::External);
            Ok(http::Response::new(http::StatusCode::Ok)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(state.with_live_reload(html)))
        }
        ResponseCacheKey::DatesIndex => {
            let html = html::render_date_index_page(maki.date_index(), AssetMode::External);
            Ok(http::Response::new(http::StatusCode::Ok)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(state.with_live_reload(html)))
        }
        ResponseCacheKey::DatePeriodPage(period) => {
            let html =
                html::render_date_period_page(*period, maki.date_index(), AssetMode::External);
            Ok(http::Response::new(http::StatusCode::Ok)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(state.with_live_reload(html)))
        }
        ResponseCacheKey::SearchIndex => Ok(http::Response::new(http::StatusCode::Ok)
            .set_header("Content-Type", "application/json; charset=utf-8")
            .set_body(search_index_json(maki.search_entries()))),
        ResponseCacheKey::NotePage(path) => {
            let html = maki.render_html_with_asset_mode(path, AssetMode::External)?;
            Ok(http::Response::new(http::StatusCode::Ok)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(state.with_live_reload(html)))
        }
    };

    state
        .metrics()
        .record_render_duration(kind, started.elapsed());
    if result.is_err() {
        state.metrics().record_render_error(kind);
    }
    result
}

fn handle_request(state: &AppState, request: &http::Request) -> Result<http::Response, Error> {
    let target = parse_request_target(request.target())?;

    if has_parent_dir_component(&target.path) {
        return Err(Error::BadRequest);
    }

    if let Some(asset) = html::runtime_asset_for_request_path(&target.path) {
        return Ok(runtime_asset_response(asset));
    }

    let project = state.project.read().map_err(|_| Error::Maki {
        source: MakiError::ReadDirectoryFailed(PathBuf::from(".")),
    })?;
    let maki = &project.maki;

    if target.path == META_PATH || target.path == META_PATH_NO_SLASH {
        return cacheable_response(state, &project, ResponseCacheKey::MetaIndex, true);
    }

    if target.path == RECENTS_PATH || target.path == RECENTS_PATH_WITH_SLASH {
        return cacheable_response(state, &project, ResponseCacheKey::Recents, true);
    }

    if target.path == DIAGNOSTICS_PATH || target.path == DIAGNOSTICS_PATH_WITH_SLASH {
        return cacheable_response(state, &project, ResponseCacheKey::Diagnostics, true);
    }

    if target.path == DATES_PATH || target.path == DATES_PATH_WITH_SLASH {
        return cacheable_response(state, &project, ResponseCacheKey::DatesIndex, true);
    }

    if let Some(period) = date_period_for_dates_request_path(&target.path) {
        return cacheable_response(
            state,
            &project,
            ResponseCacheKey::DatePeriodPage(period),
            true,
        );
    }

    if target.path == SEARCH_INDEX_PATH {
        return cacheable_response(state, &project, ResponseCacheKey::SearchIndex, true);
    }

    if target.path == SEARCH_PATH {
        let query = query_param(target.query, "q")?.unwrap_or_default();
        let results = maki.search_titles(&query, SEARCH_PAGE_RESULT_LIMIT);
        let started = Instant::now();
        let html = html::render_search_page(
            &query,
            &results,
            maki.search_entries().len(),
            AssetMode::External,
        );
        state
            .metrics()
            .record_render_duration("search_page", started.elapsed());
        return Ok(http::Response::new(http::StatusCode::Ok)
            .set_header("Content-Type", "text/html; charset=utf-8")
            .set_body(state.with_live_reload(html)));
    }

    match maki.resolve_route(&target.path) {
        Ok(MakiRoute::NotePage(path)) => {
            cacheable_response(state, &project, ResponseCacheKey::NotePage(path), true)
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
        Err(MakiError::NoteNotFound(_path)) => {
            let html = html::render_not_found_page(&target.path, AssetMode::External);
            Ok(http::Response::new(http::StatusCode::NotFound)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(state.with_live_reload(html)))
        }
        Err(e) => Err(e.into()),
    }
}

fn response_for_request(
    state: &AppState,
    request: &http::Request,
) -> Result<http::Response, Error> {
    let response = handle_request(state, request)?;

    Ok(match request.method() {
        http::Method::Get => response,
        http::Method::Head => response.without_body(),
    })
}

fn route_label_for_request(state: &AppState, request: &http::Request) -> &'static str {
    let Ok(target) = parse_request_target(request.target()) else {
        return "not_found";
    };

    if has_parent_dir_component(&target.path) {
        return "not_found";
    }

    if target.path == LIVE_RELOAD_PATH {
        return "events";
    }
    if html::runtime_asset_for_request_path(&target.path).is_some() {
        return "asset";
    }
    if target.path == META_PATH || target.path == META_PATH_NO_SLASH {
        return "meta";
    }
    if target.path == RECENTS_PATH || target.path == RECENTS_PATH_WITH_SLASH {
        return "recents";
    }
    if target.path == DIAGNOSTICS_PATH || target.path == DIAGNOSTICS_PATH_WITH_SLASH {
        return "diagnostics";
    }
    if target.path == DATES_PATH || target.path == DATES_PATH_WITH_SLASH {
        return "dates";
    }
    if date_period_for_dates_request_path(&target.path).is_some() {
        return "date";
    }
    if target.path == SEARCH_INDEX_PATH {
        return "search_index";
    }
    if target.path == SEARCH_PATH {
        return "search";
    }

    let Ok(project) = state.project.read() else {
        return "not_found";
    };

    match project.maki.resolve_route(&target.path) {
        Ok(MakiRoute::Home) => "home",
        Ok(MakiRoute::NotePage(_)) => "note",
        Ok(MakiRoute::NoteSource(_)) => "source",
        Err(_) => "not_found",
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

fn write_response(
    stream: &mut impl Write,
    response: Response,
) -> Result<http::StatusCode, RunError> {
    let status = response.status();
    stream
        .write_all(&response.to_bytes())
        .map_err(|source| RunError::IoError { source })?;
    Ok(status)
}

fn handle_live_reload_connection<S>(
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

    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n",
        )
        .map_err(|source| RunError::IoError { source })?;
    write_sse_event(stream, "hello", token)?;

    loop {
        match receiver.recv_timeout(SSE_KEEPALIVE_INTERVAL) {
            Ok(LiveReloadEvent::Reload { version }) => {
                write_sse_event(stream, "reload", live_reload.token_for(version))?;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                write_sse_keepalive(stream)?;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(http::StatusCode::Ok),
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

    let method = request.method().as_str();
    let route = route_label_for_request(state, &request);
    let started = Instant::now();
    let _inflight = state.metrics().track_http_inflight_request();

    if request.method() == http::Method::Get && request.target() == LIVE_RELOAD_PATH {
        let result = handle_live_reload_connection(state, stream);
        let status = result
            .as_ref()
            .map(|status| status.code())
            .unwrap_or(http::StatusCode::InternalServerError.code())
            .to_string();
        state
            .metrics()
            .record_http_request(method, route, status, started.elapsed(), 0);
        return result.map(|_| ());
    }

    let response = match response_for_request(state, &request) {
        Ok(response) => response,
        Err(err) => err.into_response(),
    };
    let status = response.status().code().to_string();
    let response_bytes = response.body().len();

    let result = stream
        .write_all(&response.to_bytes())
        .map_err(|source| RunError::IoError { source });
    state
        .metrics()
        .record_http_request(method, route, status, started.elapsed(), response_bytes);
    result
}

fn metrics_response(metrics: &Metrics) -> Response {
    Response::new(http::StatusCode::Ok)
        .set_header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
        .set_body(metrics.to_prometheus_text())
}

fn handle_metrics_connection<S>(metrics: &Metrics, stream: &mut S) -> Result<(), RunError>
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

    let started = Instant::now();
    let response = if request.method() == http::Method::Get && request.target() == "/metrics" {
        let status = http::StatusCode::Ok.code().to_string();
        metrics.record_metrics_request(request.method().as_str(), status, started.elapsed());
        metrics_response(metrics)
    } else {
        let response = not_found(&Error::Maki {
            source: MakiError::NoteNotFound(PathBuf::from("/metrics")),
        });
        let status = response.status().code().to_string();
        metrics.record_metrics_request(request.method().as_str(), status, started.elapsed());
        response
    };

    stream
        .write_all(&response.to_bytes())
        .map_err(|source| RunError::IoError { source })
}

fn spawn_metrics_listener(listener: TcpListener, metrics: Metrics, endpoint: MetricsEndpoint) {
    thread::spawn(move || {
        println!(
            "Metrics listening on http://{}:{}/metrics",
            endpoint.host, endpoint.port
        );

        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(stream) => stream,
                Err(source) => {
                    eprintln!("Failed to accept metrics connection: {}", source);
                    continue;
                }
            };

            let metrics = metrics.clone();
            thread::spawn(move || {
                if let Err(error) = handle_metrics_connection(&metrics, &mut stream) {
                    eprintln!("Failed to handle metrics connection: {}", error);
                }
            });
        }
    });
}

#[derive(Debug, PartialEq, Eq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

type FileSnapshot = BTreeMap<PathBuf, FileStamp>;

fn insert_file_stamp(
    snapshot: &mut FileSnapshot,
    key: PathBuf,
    path: &Path,
) -> Result<(), std::io::Error> {
    let metadata = path.metadata()?;
    snapshot.insert(
        key,
        FileStamp {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        },
    );
    Ok(())
}

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
            let is_project_file = relative_path == Path::new(PROJECT_FILE_NAME);
            if !is_maki_note && !is_project_file {
                continue;
            }

            insert_file_stamp(acc, relative_path, &path)?;
        }

        Ok(())
    }

    let mut snapshot = BTreeMap::new();
    collect(root, root, &mut snapshot)?;
    Ok(snapshot)
}

fn collect_runtime_asset_snapshot(snapshot: &mut FileSnapshot) -> Result<(), std::io::Error> {
    for asset in html::runtime_assets() {
        let source_path = asset.source_path();
        if !source_path.is_file() {
            continue;
        }

        let key = PathBuf::from(asset.request_path().trim_start_matches('/'));
        insert_file_stamp(snapshot, key, &source_path)?;
    }

    Ok(())
}

fn collect_watched_file_snapshot(root: &Path) -> Result<FileSnapshot, std::io::Error> {
    let mut snapshot = collect_maki_file_snapshot(root)?;
    collect_runtime_asset_snapshot(&mut snapshot)?;
    Ok(snapshot)
}

fn collect_watched_project_snapshot(
    project_root: &Path,
    source_root: &Path,
) -> Result<FileSnapshot, std::io::Error> {
    let mut snapshot = collect_watched_file_snapshot(source_root)?;

    if project_root != source_root {
        let project_file = project_root.join(PROJECT_FILE_NAME);
        if project_file.is_file() {
            insert_file_stamp(
                &mut snapshot,
                PathBuf::from("__project__").join(PROJECT_FILE_NAME),
                &project_file,
            )?;
        }
    }

    Ok(snapshot)
}

fn spawn_file_watcher(state: Arc<AppState>) {
    thread::spawn(move || {
        let mut snapshot = match state.watched_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                eprintln!("Failed to initialize file watcher: {}", error);
                FileSnapshot::new()
            }
        };

        loop {
            thread::sleep(FILE_WATCH_INTERVAL);

            let next_snapshot = match state.watched_snapshot() {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    eprintln!("Failed to scan watched files: {}", error);
                    continue;
                }
            };

            if next_snapshot == snapshot {
                continue;
            }

            thread::sleep(FILE_WATCH_DEBOUNCE);

            let stable_snapshot = match state.watched_snapshot() {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    eprintln!("Failed to scan watched files after debounce: {}", error);
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

fn response_cache_warmup_keys(maki: &Maki) -> Vec<ResponseCacheKey> {
    let mut keys = Vec::with_capacity(maki.notes_len() + 4);
    keys.push(ResponseCacheKey::MetaIndex);
    keys.push(ResponseCacheKey::Recents);
    keys.push(ResponseCacheKey::SearchIndex);
    keys.push(ResponseCacheKey::Diagnostics);
    keys.push(ResponseCacheKey::DatesIndex);
    let mut date_periods = BTreeSet::new();
    for (date, _backlinks) in maki.date_index().dates() {
        date_periods.insert(DatePeriod::Year(date.year()));
        date_periods.insert(DatePeriod::Month {
            year: date.year(),
            month: date.month(),
        });
        date_periods.insert(DatePeriod::Day(*date));
    }
    keys.extend(
        date_periods
            .into_iter()
            .map(ResponseCacheKey::DatePeriodPage),
    );
    keys.extend(
        maki.notes()
            .map(|note| ResponseCacheKey::NotePage(note.source_path().to_path_buf())),
    );
    keys
}

fn warm_response_cache(state: &AppState) -> Result<(), Error> {
    let started = Instant::now();
    let keys = {
        let project = state.project.read().map_err(|_| Error::Maki {
            source: MakiError::ReadDirectoryFailed(PathBuf::from(".")),
        })?;
        response_cache_warmup_keys(&project.maki)
    };

    for key in keys {
        let kind = key.kind();
        let project = state.project.read().map_err(|_| Error::Maki {
            source: MakiError::ReadDirectoryFailed(PathBuf::from(".")),
        })?;

        match cacheable_response(state, &project, key, false) {
            Ok(_) => state
                .metrics()
                .record_response_cache_warmup_item(kind, "ok"),
            Err(error) => {
                state
                    .metrics()
                    .record_response_cache_warmup_item(kind, "error");
                eprintln!("Failed to warm response cache: {}", error);
            }
        }
    }

    state
        .metrics()
        .record_response_cache_warmup_duration(started.elapsed());
    Ok(())
}

fn spawn_response_cache_warmer(state: Arc<AppState>) {
    thread::spawn(move || {
        if let Err(error) = warm_response_cache(&state) {
            eprintln!("Failed to warm response cache: {}", error);
        }
    });
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
    let state = Arc::new(AppState::new_with_overrides(
        project_root,
        maki,
        config_overrides,
        live_reload,
        metrics.clone(),
    ));
    if let (Some(listener), Some(endpoint)) = (metrics_listener, metrics_endpoint) {
        spawn_metrics_listener(listener, metrics, endpoint);
    }
    if live_reload {
        spawn_file_watcher(Arc::clone(&state));
    }
    setup(ProjectReloader {
        state: Arc::clone(&state),
    });
    spawn_response_cache_warmer(Arc::clone(&state));

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
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use crate::metrics::Metrics;
    use crate::web::*;
    use maki_core::MakiConfig;

    fn repo_path(path: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }

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

        let maki = Maki::load(repo_path(".")).unwrap();
        let state = AppState::new(maki);

        let response = handle_request(&state, &request).unwrap();
        let body = String::from_utf8(response.body().to_vec()).unwrap();

        assert_eq!(response.status(), http::StatusCode::NotFound);
        assert_eq!(
            response.get_header("Content-Type"),
            Some("text/html; charset=utf-8")
        );
        assert!(body.contains("<title>Not Found</title>"));
        assert!(body.contains("<header class=\"maki-nav\">"));
        assert!(body.contains("<link rel=\"stylesheet\" href=\"/.maki/assets/maki.css\">"));
        assert!(body.contains("<script src=\"/.maki/assets/maki-search.js\"></script>"));
        assert!(body.contains("<script src=\"/.maki/assets/maki-toc.js\"></script>"));
        assert!(body.contains("<code>/missing</code>"));
        assert!(body.contains("new EventSource(\"/.maki/events\")"));
    }

    #[test]
    fn test_rendered_note_includes_live_reload_script() {
        let maki = Maki::load(repo_path("docs")).unwrap();
        let state = AppState::new(maki);
        let request = http::Request::get("/index");

        let response = handle_request(&state, &request).unwrap();
        let body = String::from_utf8(response.body().to_vec()).unwrap();

        assert!(body.contains("<link rel=\"stylesheet\" href=\"/.maki/assets/maki.css\">"));
        assert!(body.contains("<script src=\"/.maki/assets/maki-search.js\"></script>"));
        assert!(body.contains("<script src=\"/.maki/assets/maki-toc.js\"></script>"));
        assert!(body.contains("new EventSource(\"/.maki/events\")"));
        assert!(body.contains("</script></body>"));
        assert!(!body.contains("<style>:root"));
    }

    #[test]
    fn test_source_note_does_not_include_live_reload_script() {
        let maki = Maki::load(repo_path("docs")).unwrap();
        let state = AppState::new(maki);
        let request = http::Request::get("/index.maki");

        let response = handle_request(&state, &request).unwrap();
        let body = String::from_utf8(response.body().to_vec()).unwrap();

        assert!(!body.contains("new EventSource(\"/.maki/events\")"));
    }

    #[test]
    fn test_search_index_returns_note_titles() {
        let maki = Maki::load(repo_path("docs")).unwrap();
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
        let maki = Maki::load(repo_path("docs")).unwrap();
        let state = AppState::new(maki);
        let request = http::Request::get("/.maki/search?q=syntax");

        let response = handle_request(&state, &request).unwrap();
        let body = String::from_utf8(response.body().to_vec()).unwrap();

        assert!(body.contains("<title>Search</title>"));
        assert!(body.contains("<a href=\"/maki-syntax\">Maki Syntax</a>"));
        assert!(body.contains("<link rel=\"stylesheet\" href=\"/.maki/assets/maki.css\">"));
        assert!(body.contains("<script src=\"/.maki/assets/maki-search.js\"></script>"));
        assert!(body.contains("<script src=\"/.maki/assets/maki-toc.js\"></script>"));
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
    fn test_note_page_response_cache_is_replaced_with_project() {
        let root = std::env::temp_dir().join(format!("maki-response-cache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("home.maki"), "Cache marker: first generation").unwrap();

        let maki = Maki::load(&root).unwrap();
        let state = AppState::new(maki);
        let request = http::Request::get("/home");

        let first = handle_request(&state, &request).unwrap();
        let first_body = String::from_utf8(first.body().to_vec()).unwrap();
        assert!(first_body.contains("Cache marker: first generation"));
        assert_eq!(state.cached_response_count(), 1);

        fs::write(root.join("home.maki"), "Cache marker: second generation").unwrap();

        let cached = handle_request(&state, &request).unwrap();
        let cached_body = String::from_utf8(cached.body().to_vec()).unwrap();
        assert!(cached_body.contains("Cache marker: first generation"));
        assert!(!cached_body.contains("Cache marker: second generation"));

        state.reload().unwrap();
        assert_eq!(state.cached_response_count(), 0);

        let reloaded = handle_request(&state, &request).unwrap();
        let reloaded_body = String::from_utf8(reloaded.body().to_vec()).unwrap();
        assert!(reloaded_body.contains("Cache marker: second generation"));
        assert_eq!(state.cached_response_count(), 1);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn test_response_cache_hit_miss_metrics_follow_requests() {
        let maki = Maki::load(repo_path("docs")).unwrap();
        let metrics = Metrics::enabled();
        let state = AppState::new_with_metrics(maki, metrics.clone());
        let request = http::Request::get("/index");

        let first = handle_request(&state, &request).unwrap();
        assert_eq!(first.status(), http::StatusCode::Ok);

        let second = handle_request(&state, &request).unwrap();
        assert_eq!(second.status(), http::StatusCode::Ok);

        let text = metrics.to_prometheus_text();
        assert!(text.contains("maki_response_cache_requests_total{kind=\"note\",cache=\"hit\"} 1"));
        assert!(
            text.contains("maki_response_cache_requests_total{kind=\"note\",cache=\"miss\"} 1")
        );
    }

    #[test]
    fn test_reload_updates_project_and_cache_gauges() {
        let root = std::env::temp_dir().join(format!("maki-metrics-reload-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("home.maki"), "Home generation one").unwrap();

        let metrics = Metrics::enabled();
        let maki = Maki::load_with_config_metered(&root, MakiConfig::default(), &metrics).unwrap();
        let state = AppState::new_with_metrics(maki, metrics.clone());

        handle_request(&state, &http::Request::get("/home")).unwrap();
        let before = metrics.to_prometheus_text();
        assert!(before.contains("maki_project_notes 1"));
        assert!(before.contains("maki_response_cache_entries 1"));

        fs::write(root.join("next.maki"), "Next generation").unwrap();
        state.reload().unwrap();

        let after = metrics.to_prometheus_text();
        fs::remove_dir_all(root).unwrap();
        assert!(after.contains("maki_project_notes 2"));
        assert!(after.contains("maki_response_cache_entries 0"));
        assert!(after.contains("maki_project_reload_total{source=\"directory\",result=\"ok\"} 1"));
    }

    #[test]
    fn test_warm_response_cache_populates_cacheable_project_routes() {
        let maki = Maki::load(repo_path("docs")).unwrap();
        let expected_entries = response_cache_warmup_keys(&maki).len();
        let state = AppState::new(maki);

        assert_eq!(state.cached_response_count(), 0);

        warm_response_cache(&state).unwrap();

        assert_eq!(state.cached_response_count(), expected_entries);
    }

    #[test]
    fn test_runtime_asset_routes_return_source_assets() {
        let maki = Maki::load(repo_path("docs")).unwrap();
        let state = AppState::new(maki);

        let css = handle_request(&state, &http::Request::get("/.maki/assets/maki.css")).unwrap();
        let css_body = String::from_utf8(css.body().to_vec()).unwrap();
        assert_eq!(
            css.get_header("Content-Type"),
            Some("text/css; charset=utf-8")
        );
        assert_eq!(css.get_header("Cache-Control"), Some("no-cache"));
        assert!(css_body.contains(":root"));

        let js =
            handle_request(&state, &http::Request::get("/.maki/assets/maki-search.js")).unwrap();
        let js_body = String::from_utf8(js.body().to_vec()).unwrap();
        assert_eq!(
            js.get_header("Content-Type"),
            Some("application/javascript; charset=utf-8")
        );
        assert_eq!(js.get_header("Cache-Control"), Some("no-cache"));
        assert!(js_body.contains("SEARCH_INDEX_PATH"));

        let toc = handle_request(&state, &http::Request::get("/.maki/assets/maki-toc.js")).unwrap();
        let toc_body = String::from_utf8(toc.body().to_vec()).unwrap();
        assert_eq!(
            toc.get_header("Content-Type"),
            Some("application/javascript; charset=utf-8")
        );
        assert_eq!(toc.get_header("Cache-Control"), Some("no-cache"));
        assert!(toc_body.contains("HEADING_SELECTOR"));
    }

    #[test]
    fn test_watched_file_snapshot_includes_runtime_assets() {
        let root = repo_path("docs");
        let snapshot = collect_watched_file_snapshot(&root).unwrap();

        assert!(snapshot.contains_key(&PathBuf::from(".maki/assets/maki.css")));
        assert!(snapshot.contains_key(&PathBuf::from(".maki/assets/maki-search.js")));
        assert!(snapshot.contains_key(&PathBuf::from(".maki/assets/maki-toc.js")));
    }

    #[test]
    fn test_meta_index_links_internal_indexes() {
        let maki = Maki::load(repo_path("docs")).unwrap();
        let state = AppState::new(maki);
        let response = handle_request(&state, &http::Request::get("/@/")).unwrap();
        let body = String::from_utf8(response.body().to_vec()).unwrap();

        assert_eq!(response.status(), http::StatusCode::Ok);
        assert!(body.contains("<title>Meta</title>"));
        assert!(body.contains("<a href=\"/@/recents\">Recents</a>"));
        assert!(body.contains("<a href=\"/@/diagnostics\">Diagnostics</a>"));
        assert!(body.contains("<a href=\"/@/dates\">Dates</a>"));
    }

    #[test]
    fn test_recents_page_lists_recent_notes() {
        let root = std::env::temp_dir().join(format!("maki-recents-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("alpha.maki"), "--^ title: Alpha\n").unwrap();
        fs::create_dir_all(root.join("notes")).unwrap();
        fs::write(root.join("notes/beta.maki"), "--^ title: Beta\n").unwrap();

        let maki = Maki::load(&root).unwrap();
        let state = AppState::new(maki);
        let response = handle_request(&state, &http::Request::get("/@/recents")).unwrap();
        let body = String::from_utf8(response.body().to_vec()).unwrap();
        fs::remove_dir_all(root).unwrap();

        assert_eq!(response.status(), http::StatusCode::Ok);
        assert!(body.contains("<title>Recents</title>"));
        assert!(body.contains("KST <a href=\"/alpha\">Alpha</a></li>"));
        assert!(body.contains("<a href=\"/alpha\">Alpha</a>"));
        assert!(body.contains("<a href=\"/notes/beta\">Beta</a>"));
        assert!(!body.contains("alpha.maki"));
        assert!(!body.contains("notes/beta.maki"));
        assert!(!body.contains("UTC"));
    }

    #[test]
    fn test_dates_pages_list_dates_and_backlinks() {
        let root = std::env::temp_dir().join(format!("maki-dates-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("home.maki"),
            r#"--^ title: Home
--^ date: [2026-08-15]

Plan <2026-08-16> and [2026-08-17]--[2026-08-19].

Task with property date.
--^ scheduled: <2026-08-20 15:00>"#,
        )
        .unwrap();

        let maki = Maki::load(&root).unwrap();
        let state = AppState::new(maki);

        let index = handle_request(&state, &http::Request::get("/@/dates")).unwrap();
        let index_body = String::from_utf8(index.body().to_vec()).unwrap();
        let year = handle_request(&state, &http::Request::get("/@/dates/2026")).unwrap();
        let year_body = String::from_utf8(year.body().to_vec()).unwrap();
        let month = handle_request(&state, &http::Request::get("/@/dates/2026-08")).unwrap();
        let month_body = String::from_utf8(month.body().to_vec()).unwrap();
        assert_eq!(index.status(), http::StatusCode::Ok);
        assert!(index_body.contains("<title>Dates</title>"));
        assert!(index_body.contains("<a href=\"/@/dates/2026\">2026</a>"));
        assert!(!index_body.contains("<a href=\"/@/dates/2026-08\">2026-08</a>"));

        assert_eq!(year.status(), http::StatusCode::Ok);
        assert!(year_body.contains("<title>2026</title>"));
        assert!(year_body.contains("<a href=\"/@/dates/2025\">← 2025</a>"));
        assert!(year_body.contains("<a href=\"/@/dates/2027\">2027 →</a>"));
        assert!(year_body.contains("<a href=\"/@/dates\">↑ Dates</a>"));
        assert!(year_body.contains("<a href=\"/@/dates/2026-08\">2026-08</a>"));

        assert_eq!(month.status(), http::StatusCode::Ok);
        assert!(month_body.contains("<title>2026-08</title>"));
        assert!(month_body.contains("<a href=\"/@/dates/2026-07\">← 2026-07</a>"));
        assert!(month_body.contains("<a href=\"/@/dates/2026-09\">2026-09 →</a>"));
        assert!(month_body.contains("<a href=\"/@/dates/2026\">↑ 2026</a>"));
        assert!(month_body.contains("<h3 id=\"Days\">Days</h3>"));
        assert!(month_body.contains(
            "<h4 id=\"[2026-08-17 Mon](/@/dates/2026-08-17)\"><a href=\"/@/dates/2026-08-17\">2026-08-17 Mon</a></h4>"
        ));
        assert!(!month_body.contains("href=\"/@/dates/2026-08-18\""));
        assert!(month_body.contains(
            "<h4 id=\"[2026-08-19 Wed](/@/dates/2026-08-19)\"><a href=\"/@/dates/2026-08-19\">2026-08-19 Wed</a></h4>"
        ));
        assert!(!month_body.contains("<h3 id=\"Pages\">Pages</h3>"));
        assert!(month_body.contains(
            "<li><a href=\"/home#date-inline-home-maki-2\">Home</a> date, range start, inline<pre><code>Plan &lt;2026-08-16&gt; and [2026-08-17]--[2026-08-19].</code></pre></li>"
        ));
        assert!(month_body.contains(
            "<li><a href=\"/home#date-property-home-maki-2\">Home</a> event, single, property:scheduled<pre><code>scheduled: &lt;2026-08-20 15:00&gt;\nTask with property date.</code></pre></li>"
        ));
        let date_heading_position = |date: &str| {
            month_body
                .find(&format!("href=\"/@/dates/{date}\""))
                .unwrap()
        };
        assert!(date_heading_position("2026-08-20") < date_heading_position("2026-08-19"));
        assert!(date_heading_position("2026-08-19") < date_heading_position("2026-08-17"));

        let detail = handle_request(&state, &http::Request::get("/@/dates/2026-08-18")).unwrap();
        let detail_body = String::from_utf8(detail.body().to_vec()).unwrap();
        let empty_detail =
            handle_request(&state, &http::Request::get("/@/dates/2026-08-21")).unwrap();
        let empty_detail_body = String::from_utf8(empty_detail.body().to_vec()).unwrap();
        let property_detail =
            handle_request(&state, &http::Request::get("/@/dates/2026-08-20")).unwrap();
        let property_detail_body = String::from_utf8(property_detail.body().to_vec()).unwrap();
        let note = handle_request(&state, &http::Request::get("/home")).unwrap();
        let note_body = String::from_utf8(note.body().to_vec()).unwrap();
        fs::remove_dir_all(root).unwrap();

        assert_eq!(detail.status(), http::StatusCode::Ok);
        assert!(detail_body.contains("<title>2026-08-18 Tue</title>"));
        assert!(detail_body.contains("<a href=\"/@/dates/2026-08-17\">← 2026-08-17 Mon</a>"));
        assert!(detail_body.contains("<a href=\"/@/dates/2026-08-19\">2026-08-19 Wed →</a>"));
        assert!(detail_body.contains("<a href=\"/@/dates/2026-08\">↑ 2026-08</a>"));
        assert!(detail_body.contains("[2026-08-17]--[2026-08-19]"));
        assert!(detail_body.contains("range"));
        assert!(detail_body.contains(
            "<li><a href=\"/home#date-inline-home-maki-2\">Home</a> date, range, inline<pre><code>Plan &lt;2026-08-16&gt; and [2026-08-17]--[2026-08-19].</code></pre></li>"
        ));
        assert!(detail_body.contains("Plan &lt;2026-08-16&gt; and [2026-08-17]--[2026-08-19]."));

        assert_eq!(empty_detail.status(), http::StatusCode::Ok);
        assert!(empty_detail_body.contains("<title>2026-08-21 Fri</title>"));
        assert!(empty_detail_body.contains("No date markers."));

        assert_eq!(property_detail.status(), http::StatusCode::Ok);
        assert!(
            property_detail_body.contains("<a href=\"/home#date-property-home-maki-2\">Home</a>")
        );
        assert!(property_detail_body.contains("event, single, property:scheduled"));
        assert!(property_detail_body.contains("scheduled: &lt;2026-08-20 15:00&gt;"));
        assert!(property_detail_body.contains("Task with property date."));

        assert_eq!(note.status(), http::StatusCode::Ok);
        assert!(note_body.contains("id=\"date-inline-home-maki-2\""));
        assert!(note_body.contains("<a class=\"maki-date-stamp maki-date-stamp-reference\" href=\"/@/dates/2026-08-17#date-inline-home-maki-2\">[2026-08-17]</a>&ndash;<a class=\"maki-date-stamp maki-date-stamp-reference\" href=\"/@/dates/2026-08-19#date-inline-home-maki-2\">[2026-08-19]</a>"));
        assert!(note_body.contains("id=\"date-property-home-maki-2\""));
    }

    #[test]
    fn test_diagnostics_page_lists_project_issues() {
        let root = std::env::temp_dir().join(format!("maki-diagnostics-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("home.maki"),
            "See [[missing]] and [Ghost](ghost).",
        )
        .unwrap();

        let maki = Maki::load(&root).unwrap();
        let state = AppState::new(maki);
        let request = http::Request::get("/@/diagnostics");

        let response = handle_request(&state, &request).unwrap();
        let body = String::from_utf8(response.body().to_vec()).unwrap();

        fs::remove_dir_all(root).unwrap();
        assert_eq!(
            response.get_header("Content-Type"),
            Some("text/html; charset=utf-8")
        );
        assert!(body.contains("<title>Diagnostics</title>"));
        assert!(body.contains("<link rel=\"stylesheet\" href=\"/.maki/assets/maki.css\">"));
        assert!(body.contains("<script src=\"/.maki/assets/maki-search.js\"></script>"));
        assert!(body.contains("<script src=\"/.maki/assets/maki-toc.js\"></script>"));
        assert!(body.contains("2 issue(s)"));
        assert!(
            body.contains("<h3 id=\"[home.maki](/home)\"><a href=\"/home\">home.maki</a></h3>")
        );
        assert!(body.contains("broken link: missing"));
        assert!(body.contains("broken link: ghost"));
        assert!(!body.contains("maki-diagnostics-table"));
    }

    #[test]
    fn test_head_diagnostics_page_returns_headers_without_body() {
        let maki = Maki::load(repo_path("docs")).unwrap();
        let state = AppState::new(maki);
        let request = http::Request::new(http::Method::Head, "/@/diagnostics");

        let response = response_for_request(&state, &request).unwrap();

        assert_eq!(response.status(), http::StatusCode::Ok);
        assert_eq!(
            response.get_header("Content-Type"),
            Some("text/html; charset=utf-8")
        );
        assert!(
            response
                .get_header("Content-Length")
                .is_some_and(|length| length.parse::<usize>().unwrap() > 0)
        );
        assert_eq!(response.body(), b"");
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
