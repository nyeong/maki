use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use crate::http;
use maki_core::html::{self, AssetMode};
use maki_core::{DatePeriod, Error as MakiError, HomeMode, Maki, MakiRoute, SearchEntry};

use super::error::Error;
use super::state::{AppState, ProjectState, ResponseCacheKey};
use super::target::{
    date_period_for_dates_request_path, has_parent_dir_component, parse_request_target, query_param,
};
use super::{
    DATES_PATH, DATES_PATH_WITH_SLASH, DIAGNOSTICS_PATH, DIAGNOSTICS_PATH_WITH_SLASH,
    LIVE_RELOAD_PATH, META_PATH, META_PATH_NO_SLASH, RECENTS_PATH, RECENTS_PATH_WITH_SLASH,
    SEARCH_INDEX_PATH, SEARCH_PAGE_RESULT_LIMIT, SEARCH_PATH,
};

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

#[derive(Clone, Copy)]
enum StaticRoute {
    LiveReload,
    RuntimeAsset(html::RuntimeAsset),
    MetaIndex,
    Recents,
    Diagnostics,
    DatesIndex,
    DatePeriod(DatePeriod),
    SearchIndex,
    Search,
}

impl StaticRoute {
    fn label(self) -> &'static str {
        match self {
            Self::LiveReload => "events",
            Self::RuntimeAsset(_) => "asset",
            Self::MetaIndex => "meta",
            Self::Recents => "recents",
            Self::Diagnostics => "diagnostics",
            Self::DatesIndex => "dates",
            Self::DatePeriod(_) => "date",
            Self::SearchIndex => "search_index",
            Self::Search => "search",
        }
    }
}

fn static_route_for_path(path: &str) -> Option<StaticRoute> {
    if path == LIVE_RELOAD_PATH {
        return Some(StaticRoute::LiveReload);
    }
    if let Some(asset) = html::runtime_asset_for_request_path(path) {
        return Some(StaticRoute::RuntimeAsset(asset));
    }
    if path == META_PATH || path == META_PATH_NO_SLASH {
        return Some(StaticRoute::MetaIndex);
    }
    if path == RECENTS_PATH || path == RECENTS_PATH_WITH_SLASH {
        return Some(StaticRoute::Recents);
    }
    if path == DIAGNOSTICS_PATH || path == DIAGNOSTICS_PATH_WITH_SLASH {
        return Some(StaticRoute::Diagnostics);
    }
    if path == DATES_PATH || path == DATES_PATH_WITH_SLASH {
        return Some(StaticRoute::DatesIndex);
    }
    if let Some(period) = date_period_for_dates_request_path(path) {
        return Some(StaticRoute::DatePeriod(period));
    }
    if path == SEARCH_INDEX_PATH {
        return Some(StaticRoute::SearchIndex);
    }
    if path == SEARCH_PATH {
        return Some(StaticRoute::Search);
    }

    None
}

pub(super) fn cacheable_response(
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

pub(super) fn handle_request(
    state: &AppState,
    request: &http::Request,
) -> Result<http::Response, Error> {
    let target = parse_request_target(request.target())?;

    if has_parent_dir_component(&target.path) {
        return Err(Error::BadRequest);
    }

    let static_route = static_route_for_path(&target.path);

    if let Some(StaticRoute::RuntimeAsset(asset)) = static_route {
        return Ok(runtime_asset_response(asset));
    }

    let project = state.project.read().map_err(|_| Error::Maki {
        source: MakiError::ReadDirectoryFailed(PathBuf::from(".")),
    })?;
    let maki = &project.maki;

    if let Some(route) = static_route {
        match route {
            StaticRoute::MetaIndex => {
                return cacheable_response(state, &project, ResponseCacheKey::MetaIndex, true);
            }
            StaticRoute::Recents => {
                return cacheable_response(state, &project, ResponseCacheKey::Recents, true);
            }
            StaticRoute::Diagnostics => {
                return cacheable_response(state, &project, ResponseCacheKey::Diagnostics, true);
            }
            StaticRoute::DatesIndex => {
                return cacheable_response(state, &project, ResponseCacheKey::DatesIndex, true);
            }
            StaticRoute::DatePeriod(period) => {
                return cacheable_response(
                    state,
                    &project,
                    ResponseCacheKey::DatePeriodPage(period),
                    true,
                );
            }
            StaticRoute::SearchIndex => {
                return cacheable_response(state, &project, ResponseCacheKey::SearchIndex, true);
            }
            StaticRoute::Search => {
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
            StaticRoute::LiveReload | StaticRoute::RuntimeAsset(_) => {}
        }
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

pub(super) fn response_for_request(
    state: &AppState,
    request: &http::Request,
) -> Result<http::Response, Error> {
    let response = handle_request(state, request)?;

    Ok(match request.method() {
        http::Method::Get => response,
        http::Method::Head => response.without_body(),
    })
}

pub(super) fn route_label_for_request(state: &AppState, request: &http::Request) -> &'static str {
    let Ok(target) = parse_request_target(request.target()) else {
        return "not_found";
    };

    if has_parent_dir_component(&target.path) {
        return "not_found";
    }

    if let Some(route) = static_route_for_path(&target.path) {
        return route.label();
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
