use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use crate::http;
use maki_core::html::{self, AssetMode};
use maki_core::parser::DateStampKind;
use maki_core::{
    DatePeriod, Error as MakiError, HomeMode, Maki, MakiRoute, SearchEntry, SitemapEntry,
    analysis::{
        AnalysisBlockKind, AnalysisDiagnosticKind, DateOrigin as AnalysisDateOrigin,
        LinkResolution, ProjectAnalysis, PropertyDirection,
    },
};

use super::error::Error;
use super::state::{AppState, ProjectState, ResponseCacheKey};
use super::target::{
    date_period_for_dates_request_path, has_parent_dir_component, parse_request_target, query_param,
};
use super::{
    DATES_PATH, DATES_PATH_WITH_SLASH, DIAGNOSTICS_PATH, DIAGNOSTICS_PATH_WITH_SLASH, FAVICON_PATH,
    LIVE_RELOAD_PATH, META_PATH, META_PATH_NO_SLASH, PROJECT_INDEX_PATH, RECENTS_PATH,
    RECENTS_PATH_WITH_SLASH, SEARCH_INDEX_PATH, SEARCH_PAGE_RESULT_LIMIT, SEARCH_PATH,
    SITEMAP_PATH, SITEMAP_PATH_WITH_SLASH, SITEMAP_XML_PATH,
};

fn push_json_string(output: &mut String, input: &str) {
    output.push('"');
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
    output.push('"');
}

fn search_index_json(entries: &[SearchEntry]) -> String {
    let mut json = String::from("[");

    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }

        json.push_str("{\"kind\":");
        push_json_string(&mut json, entry.kind().as_str());
        json.push_str(",\"title\":");
        push_json_string(&mut json, entry.title());
        json.push_str(",\"path\":");
        push_json_string(&mut json, entry.path());
        json.push_str(",\"source_path\":");
        push_json_string(&mut json, entry.source_path());
        json.push('}');
    }

    json.push(']');
    json
}

fn sitemap_xml(entries: &[SitemapEntry]) -> String {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
    for entry in entries {
        xml.push_str("  <url><loc>");
        push_xml_escaped(&mut xml, entry.path());
        xml.push_str("</loc></url>\n");
    }
    xml.push_str("</urlset>\n");
    xml
}

fn push_xml_escaped(output: &mut String, input: &str) {
    for ch in input.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            _ => output.push(ch),
        }
    }
}

fn source_span_json(span: maki_core::source::SourceSpan) -> String {
    format!("{{\"start\":{},\"end\":{}}}", span.start, span.end)
}

fn project_index_json(maki: &Maki) -> Result<String, MakiError> {
    let analysis = maki.published_analysis()?;

    Ok(project_analysis_json(&analysis))
}

fn project_analysis_json(analysis: &ProjectAnalysis) -> String {
    let mut json = String::from("{\"schema_version\":1,\"documents\":[");
    for (index, document) in analysis.documents.values().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push_str("{\"title\":");
        push_json_string(&mut json, &document.title);
        json.push_str(",\"path\":");
        push_json_string(&mut json, &format!("/{}", document.canonical_path));
        json.push_str(",\"source_path\":");
        push_json_string(&mut json, &document.path.display().to_string());
        json.push_str(",\"document_span\":");
        json.push_str(&source_span_json(document.document_span));

        json.push_str(",\"blocks\":[");
        for (block_index, block) in document.blocks.iter().enumerate() {
            if block_index > 0 {
                json.push(',');
            }
            json.push_str("{\"kind\":");
            push_json_string(&mut json, block_kind_label(block.kind));
            json.push_str(",\"span\":");
            json.push_str(&source_span_json(block.span));
            json.push('}');
        }
        json.push(']');

        json.push_str(",\"headings\":[");
        for (heading_index, heading) in document.headings.iter().enumerate() {
            if heading_index > 0 {
                json.push(',');
            }
            json.push_str("{\"level\":");
            let _ = write!(json, "{}", heading.level);
            json.push_str(",\"title\":");
            push_json_string(&mut json, &heading.title);
            json.push_str(",\"anchor\":");
            push_json_string(&mut json, &heading.anchor);
            json.push_str(",\"span\":");
            json.push_str(&source_span_json(heading.span));
            json.push_str(",\"title_span\":");
            json.push_str(&source_span_json(heading.title_span));
            json.push('}');
        }
        json.push(']');

        json.push_str(",\"links\":[");
        for (link_index, link) in document.note_links.iter().enumerate() {
            if link_index > 0 {
                json.push(',');
            }
            json.push_str("{\"target\":");
            push_json_string(&mut json, &link.target);
            json.push_str(",\"span\":");
            json.push_str(&source_span_json(link.span));
            json.push_str(",\"target_span\":");
            json.push_str(&source_span_json(link.target_span));
            json.push_str(",\"resolution\":");
            push_link_resolution_json(&mut json, link.resolution.as_ref());
            json.push('}');
        }
        json.push(']');

        json.push_str(",\"properties\":[");
        for (property_index, property) in document.properties.iter().enumerate() {
            if property_index > 0 {
                json.push(',');
            }
            json.push_str("{\"direction\":");
            push_json_string(&mut json, property_direction_label(property.direction));
            json.push_str(",\"key\":");
            push_json_string(&mut json, &property.key);
            json.push_str(",\"value\":");
            push_json_string(&mut json, &property.value);
            json.push_str(",\"span\":");
            json.push_str(&source_span_json(property.span));
            json.push_str(",\"key_span\":");
            json.push_str(&source_span_json(property.key_span));
            json.push_str(",\"value_span\":");
            json.push_str(&source_span_json(property.value_span));
            json.push('}');
        }
        json.push(']');

        json.push_str(",\"dates\":[");
        for (date_index, date) in document.dates.iter().enumerate() {
            if date_index > 0 {
                json.push(',');
            }
            json.push_str("{\"kind\":");
            push_json_string(&mut json, analysis_date_kind_label(date.kind, &date.origin));
            json.push_str(",\"marker_kind\":");
            push_json_string(&mut json, date_stamp_kind_label(date.kind));
            json.push_str(",\"body\":");
            push_json_string(&mut json, &date.body);
            json.push_str(",\"origin\":");
            push_json_string(&mut json, analysis_date_origin_label(&date.origin));
            if let AnalysisDateOrigin::PropertyValue { key } = &date.origin {
                json.push_str(",\"property_key\":");
                push_json_string(&mut json, key);
            }
            json.push_str(",\"span\":");
            json.push_str(&source_span_json(date.span));
            json.push('}');
        }
        json.push(']');

        json.push('}');
    }
    json.push_str("],\"diagnostics\":[");
    for (index, diagnostic) in analysis.diagnostics.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push_str("{\"path\":");
        push_json_string(&mut json, &diagnostic.path.display().to_string());
        json.push_str(",\"kind\":");
        push_json_string(&mut json, diagnostic_kind_label(diagnostic.kind));
        json.push_str(",\"message\":");
        push_json_string(&mut json, &diagnostic.message);
        json.push_str(",\"span\":");
        json.push_str(&source_span_json(diagnostic.span));
        json.push('}');
    }
    json.push_str("]}");
    json
}

fn push_link_resolution_json(output: &mut String, resolution: Option<&LinkResolution>) {
    let Some(resolution) = resolution else {
        output.push_str("null");
        return;
    };

    match resolution {
        LinkResolution::Found(target) => {
            output.push_str("{\"status\":\"found\",\"path\":");
            push_json_string(output, &target.path.display().to_string());
            output.push_str(",\"selection_span\":");
            output.push_str(&source_span_json(target.selection_span));
            if let Some(anchor) = &target.heading_anchor {
                output.push_str(",\"heading_anchor\":");
                push_json_string(output, anchor);
            }
            output.push('}');
        }
        LinkResolution::BrokenNote => output.push_str("{\"status\":\"broken_note\"}"),
        LinkResolution::AmbiguousNote => output.push_str("{\"status\":\"ambiguous_note\"}"),
        LinkResolution::BrokenHeading => output.push_str("{\"status\":\"broken_heading\"}"),
        LinkResolution::AmbiguousHeading => output.push_str("{\"status\":\"ambiguous_heading\"}"),
    }
}

fn block_kind_label(kind: AnalysisBlockKind) -> &'static str {
    match kind {
        AnalysisBlockKind::Paragraph => "paragraph",
        AnalysisBlockKind::Code => "code",
        AnalysisBlockKind::Heading => "heading",
        AnalysisBlockKind::List => "list",
        AnalysisBlockKind::Quote => "quote",
        AnalysisBlockKind::Table => "table",
        AnalysisBlockKind::Container => "container",
        AnalysisBlockKind::ReferenceDefinition => "reference_definition",
    }
}

fn property_direction_label(direction: PropertyDirection) -> &'static str {
    match direction {
        PropertyDirection::Previous => "previous",
        PropertyDirection::Next => "next",
    }
}

fn date_stamp_kind_label(kind: DateStampKind) -> &'static str {
    match kind {
        DateStampKind::Date => "date",
        DateStampKind::Event => "event",
    }
}

fn analysis_date_origin_label(origin: &AnalysisDateOrigin) -> &'static str {
    match origin {
        AnalysisDateOrigin::VisibleInline => "visible_inline",
        AnalysisDateOrigin::PropertyValue { .. } => "property_value",
    }
}

fn analysis_date_kind_label(kind: DateStampKind, origin: &AnalysisDateOrigin) -> &'static str {
    match origin {
        AnalysisDateOrigin::PropertyValue { key } if key.eq_ignore_ascii_case("scheduled") => {
            "scheduled"
        }
        AnalysisDateOrigin::PropertyValue { key } if key.eq_ignore_ascii_case("deadline") => {
            "deadline"
        }
        AnalysisDateOrigin::PropertyValue { .. } => "metadata",
        AnalysisDateOrigin::VisibleInline if kind == DateStampKind::Event => "event",
        AnalysisDateOrigin::VisibleInline => "reference",
    }
}

fn diagnostic_kind_label(kind: AnalysisDiagnosticKind) -> &'static str {
    match kind {
        AnalysisDiagnosticKind::ParseWarning => "parse_warning",
        AnalysisDiagnosticKind::BrokenNoteLink => "broken_note_link",
        AnalysisDiagnosticKind::AmbiguousNoteLink => "ambiguous_note_link",
        AnalysisDiagnosticKind::BrokenHeadingLink => "broken_heading_link",
        AnalysisDiagnosticKind::AmbiguousHeadingLink => "ambiguous_heading_link",
    }
}
fn runtime_asset_response(asset: html::RuntimeAsset) -> http::Response {
    let body =
        std::fs::read(asset.source_path()).unwrap_or_else(|_| asset.embedded().as_bytes().to_vec());

    http::Response::new(http::StatusCode::Ok)
        .set_header("Content-Type", asset.content_type())
        .set_header("Cache-Control", "no-cache")
        .set_body(body)
}

fn favicon_response(state: &AppState, maki: &Maki) -> Result<Option<http::Response>, Error> {
    let Some(path) = maki.config().favicon() else {
        return Ok(None);
    };
    let Some(content_type) = maki.config().favicon_content_type() else {
        return Ok(None);
    };

    let path = state.project_path(path);
    if !path.is_file() {
        return Ok(None);
    }

    let body = std::fs::read(path)?;
    Ok(Some(
        http::Response::new(http::StatusCode::Ok)
            .set_header("Content-Type", content_type)
            .set_header("Cache-Control", "no-cache")
            .set_body(body),
    ))
}

fn with_served_html_chrome(state: &AppState, maki: &Maki, html: String) -> String {
    let html = match maki.config().favicon_content_type() {
        Some(content_type) if maki.config().favicon().is_some() => {
            inject_favicon_link(html, content_type)
        }
        _ => html,
    };

    state.with_live_reload(html)
}

fn inject_favicon_link(html: String, content_type: &str) -> String {
    let Some(head_end) = html.find("</head>") else {
        return html;
    };

    let mut output = String::with_capacity(html.len() + 72 + content_type.len());
    output.push_str(&html[..head_end]);
    output.push_str("<link rel=\"icon\" href=\"");
    output.push_str(FAVICON_PATH);
    output.push_str("\" type=\"");
    output.push_str(content_type);
    output.push_str("\">");
    output.push_str(&html[head_end..]);
    output
}

#[derive(Clone, Copy)]
enum StaticRoute {
    LiveReload,
    RuntimeAsset(html::RuntimeAsset),
    Favicon,
    MetaIndex,
    Recents,
    Sitemap,
    SitemapXml,
    Diagnostics,
    DatesIndex,
    DatePeriod(DatePeriod),
    ProjectIndex,
    SearchIndex,
    Search,
}

impl StaticRoute {
    fn label(self) -> &'static str {
        match self {
            Self::LiveReload => "events",
            Self::RuntimeAsset(_) => "asset",
            Self::Favicon => "favicon",
            Self::MetaIndex => "meta",
            Self::Recents => "recents",
            Self::Sitemap => "sitemap",
            Self::SitemapXml => "sitemap_xml",
            Self::Diagnostics => "diagnostics",
            Self::DatesIndex => "dates",
            Self::DatePeriod(_) => "date",
            Self::ProjectIndex => "project_index",
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
    if path == FAVICON_PATH {
        return Some(StaticRoute::Favicon);
    }
    if path == META_PATH || path == META_PATH_NO_SLASH {
        return Some(StaticRoute::MetaIndex);
    }
    if path == RECENTS_PATH || path == RECENTS_PATH_WITH_SLASH {
        return Some(StaticRoute::Recents);
    }
    if path == SITEMAP_PATH || path == SITEMAP_PATH_WITH_SLASH {
        return Some(StaticRoute::Sitemap);
    }
    if path == SITEMAP_XML_PATH {
        return Some(StaticRoute::SitemapXml);
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
    if path == PROJECT_INDEX_PATH {
        return Some(StaticRoute::ProjectIndex);
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
    let site_title = maki.config().project_title();
    let site_header = maki.config().favicon().is_some();
    let result = match key {
        ResponseCacheKey::MetaIndex => {
            let html = html::render_meta_index_page(AssetMode::External, site_title, site_header);
            Ok(http::Response::new(http::StatusCode::Ok)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(with_served_html_chrome(state, maki, html)))
        }
        ResponseCacheKey::Recents => {
            let html = html::render_recents_page(
                maki.recent_entries(),
                AssetMode::External,
                site_title,
                site_header,
            );
            Ok(http::Response::new(http::StatusCode::Ok)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(with_served_html_chrome(state, maki, html)))
        }
        ResponseCacheKey::Sitemap => {
            let html = html::render_sitemap_page(
                maki.published_sitemap_entries(),
                AssetMode::External,
                site_title,
                site_header,
            );
            Ok(http::Response::new(http::StatusCode::Ok)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(with_served_html_chrome(state, maki, html)))
        }
        ResponseCacheKey::SitemapXml => Ok(http::Response::new(http::StatusCode::Ok)
            .set_header("Content-Type", "application/xml; charset=utf-8")
            .set_body(sitemap_xml(maki.published_sitemap_entries()))),
        ResponseCacheKey::Diagnostics => {
            let diagnostics = maki.diagnostics_without_external_links();
            let html = html::render_diagnostics_page(
                &diagnostics,
                maki.notes_len(),
                AssetMode::External,
                site_title,
                site_header,
            );
            Ok(http::Response::new(http::StatusCode::Ok)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(with_served_html_chrome(state, maki, html)))
        }
        ResponseCacheKey::DatesIndex => {
            let html = html::render_date_index_page(
                maki.date_index(),
                AssetMode::External,
                site_title,
                site_header,
            );
            Ok(http::Response::new(http::StatusCode::Ok)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(with_served_html_chrome(state, maki, html)))
        }
        ResponseCacheKey::DatePeriodPage(period) => {
            let html = html::render_date_period_page(
                *period,
                maki.date_index(),
                AssetMode::External,
                site_title,
                site_header,
            );
            Ok(http::Response::new(http::StatusCode::Ok)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(with_served_html_chrome(state, maki, html)))
        }
        ResponseCacheKey::SearchIndex => Ok(http::Response::new(http::StatusCode::Ok)
            .set_header("Content-Type", "application/json; charset=utf-8")
            .set_body(search_index_json(maki.published_search_entries()))),
        ResponseCacheKey::ProjectIndex => Ok(http::Response::new(http::StatusCode::Ok)
            .set_header("Content-Type", "application/json; charset=utf-8")
            .set_body(project_index_json(maki)?)),
        ResponseCacheKey::NotePage(path) => {
            let html = maki.render_html_with_site_title(path, AssetMode::External, site_title)?;
            Ok(http::Response::new(http::StatusCode::Ok)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(with_served_html_chrome(state, maki, html)))
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
            StaticRoute::Favicon => {
                return favicon_response(state, maki)?.ok_or_else(|| Error::Maki {
                    source: MakiError::NoteNotFound(PathBuf::from(FAVICON_PATH)),
                });
            }
            StaticRoute::Recents => {
                return cacheable_response(state, &project, ResponseCacheKey::Recents, true);
            }
            StaticRoute::Sitemap => {
                return cacheable_response(state, &project, ResponseCacheKey::Sitemap, true);
            }
            StaticRoute::SitemapXml => {
                return cacheable_response(state, &project, ResponseCacheKey::SitemapXml, true);
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
            StaticRoute::ProjectIndex => {
                return cacheable_response(state, &project, ResponseCacheKey::ProjectIndex, true);
            }
            StaticRoute::Search => {
                let query = query_param(target.query, "q")?.unwrap_or_default();
                let results = maki.search_titles(&query, SEARCH_PAGE_RESULT_LIMIT);
                let started = Instant::now();
                let html = html::render_search_page(
                    &query,
                    &results,
                    maki.published_search_entries().len(),
                    AssetMode::External,
                    maki.config().project_title(),
                    maki.config().favicon().is_some(),
                );
                state
                    .metrics()
                    .record_render_duration("search_page", started.elapsed());
                return Ok(http::Response::new(http::StatusCode::Ok)
                    .set_header("Content-Type", "text/html; charset=utf-8")
                    .set_body(with_served_html_chrome(state, maki, html)));
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
            let html = html::render_not_found_page(
                &target.path,
                AssetMode::External,
                maki.config().project_title(),
            );
            Ok(http::Response::new(http::StatusCode::NotFound)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(with_served_html_chrome(state, maki, html)))
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
