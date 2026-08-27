use super::MAX_REQUEST_HEAD_SIZE;
use super::cache_warmer::{response_cache_warmup_keys, warm_response_cache};
use super::error::Error;
use super::live_reload::{LiveReload, LiveReloadError, LiveReloadEvent};
use super::routes::{handle_request, response_for_request};
use super::server::read_request_head;
use super::state::AppState;
use super::watch::collect_watched_file_snapshot;
use crate::http;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::metrics::Metrics;
use maki_core::{Maki, MakiConfig};

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
    assert!(body.contains("<script src=\"/.maki/assets/maki-external-links.js\"></script>"));
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
    assert!(body.contains("<script src=\"/.maki/assets/maki-external-links.js\"></script>"));
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
fn test_search_index_returns_project_entries() {
    let maki = Maki::load(repo_path("docs")).unwrap();
    let state = AppState::new(maki);
    let request = http::Request::get("/.maki/search-index.json");

    let response = handle_request(&state, &request).unwrap();
    let body = String::from_utf8(response.body().to_vec()).unwrap();

    assert_eq!(
        response.get_header("Content-Type"),
        Some("application/json; charset=utf-8")
    );
    assert!(body.contains("\"kind\":\"note\""));
    assert!(body.contains("\"title\":\"Maki Syntax\""));
    assert!(body.contains("\"path\":\"/maki-syntax\""));
}

#[test]
fn test_search_index_includes_files_and_headings() {
    let root = std::env::temp_dir().join(format!("maki-search-entries-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("alpha.maki"),
        r#"--^ title: Alpha Note

= Overview

body"#,
    )
    .unwrap();

    let maki = Maki::load(&root).unwrap();
    let state = AppState::new(maki);
    let response = handle_request(&state, &http::Request::get("/.maki/search-index.json")).unwrap();
    let body = String::from_utf8(response.body().to_vec()).unwrap();
    fs::remove_dir_all(root).unwrap();

    assert!(body.contains("\"kind\":\"note\",\"title\":\"Alpha Note\""));
    assert!(body.contains("\"kind\":\"file\",\"title\":\"alpha.maki\""));
    assert!(body.contains("\"kind\":\"heading\",\"title\":\"Overview\""));
    assert!(body.contains("\"path\":\"/alpha#Overview\""));
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
fn test_project_title_suffixes_served_html_titles() {
    let root = std::env::temp_dir().join(format!("maki-site-title-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("maki.toml"),
        "[project]\ntitle = \"Site & Name\"\n",
    )
    .unwrap();
    fs::write(root.join("home.maki"), "--^ title: Home\n\nbody").unwrap();

    let config = MakiConfig::load_project(&root).unwrap();
    let maki = Maki::load_with_config(&root, config).unwrap();
    let state = AppState::new(maki);

    let note = handle_request(&state, &http::Request::get("/home")).unwrap();
    let note_body = String::from_utf8(note.body().to_vec()).unwrap();
    let search = handle_request(&state, &http::Request::get("/.maki/search")).unwrap();
    let search_body = String::from_utf8(search.body().to_vec()).unwrap();
    let meta = handle_request(&state, &http::Request::get("/@/")).unwrap();
    let meta_body = String::from_utf8(meta.body().to_vec()).unwrap();
    let missing = handle_request(&state, &http::Request::get("/missing")).unwrap();
    let missing_body = String::from_utf8(missing.body().to_vec()).unwrap();

    fs::remove_dir_all(root).unwrap();
    assert!(note_body.contains("<title>Home | Site &amp; Name</title>"));
    assert!(search_body.contains("<title>Search | Site &amp; Name</title>"));
    assert!(meta_body.contains("<title>Meta | Site &amp; Name</title>"));
    assert!(missing_body.contains("<title>Not Found | Site &amp; Name</title>"));
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
    assert!(text.contains("maki_response_cache_requests_total{kind=\"note\",cache=\"miss\"} 1"));
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

    let js = handle_request(&state, &http::Request::get("/.maki/assets/maki-search.js")).unwrap();
    let js_body = String::from_utf8(js.body().to_vec()).unwrap();
    assert_eq!(
        js.get_header("Content-Type"),
        Some("application/javascript; charset=utf-8")
    );
    assert_eq!(js.get_header("Cache-Control"), Some("no-cache"));
    assert!(js_body.contains("SEARCH_INDEX_PATH"));

    let external_links = handle_request(
        &state,
        &http::Request::get("/.maki/assets/maki-external-links.js"),
    )
    .unwrap();
    let external_links_body = String::from_utf8(external_links.body().to_vec()).unwrap();
    assert_eq!(
        external_links.get_header("Content-Type"),
        Some("application/javascript; charset=utf-8")
    );
    assert_eq!(external_links.get_header("Cache-Control"), Some("no-cache"));
    assert!(external_links_body.contains("faviconUrlForHref"));

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
    assert!(snapshot.contains_key(&PathBuf::from(".maki/assets/maki-external-links.js")));
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
    assert!(body.contains("<a href=\"/@/sitemap\">Sitemap</a>"));
    assert!(body.contains("<a href=\"/@/diagnostics\">Diagnostics</a>"));
    assert!(body.contains("<a href=\"/@/dates\">Dates</a>"));
    assert!(body.contains("<a href=\"/.maki/project-index.json\">Project Index JSON</a>"));
}

#[test]
fn test_sitemap_routes_list_notes() {
    let root = std::env::temp_dir().join(format!("maki-sitemap-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("alpha.maki"), "--^ title: Alpha\n").unwrap();
    fs::create_dir_all(root.join("notes")).unwrap();
    fs::write(root.join("notes/beta.maki"), "--^ title: Beta\n").unwrap();

    let maki = Maki::load(&root).unwrap();
    let state = AppState::new(maki);
    let page = handle_request(&state, &http::Request::get("/@/sitemap")).unwrap();
    let page_body = String::from_utf8(page.body().to_vec()).unwrap();
    let xml = handle_request(&state, &http::Request::get("/sitemap.xml")).unwrap();
    let xml_body = String::from_utf8(xml.body().to_vec()).unwrap();
    fs::remove_dir_all(root).unwrap();

    assert_eq!(page.status(), http::StatusCode::Ok);
    assert_eq!(
        page.get_header("Content-Type"),
        Some("text/html; charset=utf-8")
    );
    assert!(page_body.contains("<title>Sitemap</title>"));
    assert!(page_body.contains("<a href=\"/alpha\">Alpha</a>"));
    assert!(page_body.contains("<code>alpha.maki</code>"));
    assert!(page_body.contains("<a href=\"/notes/beta\">Beta</a>"));

    assert_eq!(xml.status(), http::StatusCode::Ok);
    assert_eq!(
        xml.get_header("Content-Type"),
        Some("application/xml; charset=utf-8")
    );
    assert!(xml_body.contains("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    assert!(xml_body.contains("<loc>/alpha</loc>"));
    assert!(xml_body.contains("<loc>/notes/beta</loc>"));
    assert!(!xml_body.contains("EventSource"));
}

#[test]
fn test_project_index_json_exports_analysis() {
    let root = std::env::temp_dir().join(format!("maki-project-index-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("home.maki"),
        r#"--^ title: Home
--^ scheduled: <2026-08-26>

= Intro

See [[other#Target]] on [2026-08-25]."#,
    )
    .unwrap();
    fs::write(root.join("other.maki"), "= Target\n").unwrap();

    let maki = Maki::load(&root).unwrap();
    let state = AppState::new(maki);
    let response =
        handle_request(&state, &http::Request::get("/.maki/project-index.json")).unwrap();
    let body = String::from_utf8(response.body().to_vec()).unwrap();
    fs::remove_dir_all(root).unwrap();

    assert_eq!(response.status(), http::StatusCode::Ok);
    assert_eq!(
        response.get_header("Content-Type"),
        Some("application/json; charset=utf-8")
    );
    assert!(body.contains("\"schema_version\":1"));
    assert!(body.contains("\"documents\":["));
    assert!(body.contains("\"source_path\":\"home.maki\""));
    assert!(body.contains("\"headings\":["));
    assert!(body.contains("\"title\":\"Intro\""));
    assert!(body.contains("\"links\":["));
    assert!(body.contains("\"target\":\"other#Target\""));
    assert!(body.contains("\"target_span\":"));
    assert!(body.contains("\"resolution\":{\"status\":\"found\""));
    assert!(body.contains("\"dates\":["));
    assert!(body.contains("\"kind\":\"scheduled\""));
    assert!(body.contains("\"property_key\":\"scheduled\""));
    assert!(body.contains("\"kind\":\"reference\""));
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
    assert!(year_body.contains("<h3 id=\"Months\">Months</h3>"));
    assert!(year_body.contains("<h3 id=\"Weeks\">Weeks</h3>"));
    assert!(year_body.contains("<a href=\"/@/dates/2026-08\">2026-08</a>"));

    assert_eq!(month.status(), http::StatusCode::Ok);
    assert!(month_body.contains("<title>Month 2026-08</title>"));
    assert!(month_body.contains("<a href=\"/@/dates/2026-07\">← 2026-07</a>"));
    assert!(month_body.contains("<a href=\"/@/dates/2026-09\">2026-09 →</a>"));
    assert!(month_body.contains("<a href=\"/@/dates/2026\">↑ 2026</a>"));
    assert!(month_body.contains("<h3 id=\"Backlinks\">Backlinks</h3>"));
    assert!(month_body.contains("<h3 id=\"Days\">Days</h3>"));
    assert!(month_body.contains("<h3 id=\"Weeks\">Weeks</h3>"));
    assert!(month_body.contains("<a href=\"/@/dates/2026-08-17\">2026-08-17 Mon</a>"));
    assert!(!month_body.contains("href=\"/@/dates/2026-08-18\""));
    assert!(month_body.contains("<a href=\"/@/dates/2026-08-19\">2026-08-19 Wed</a>"));
    assert!(!month_body.contains("<h3 id=\"Pages\">Pages</h3>"));
    assert!(!month_body.contains("href=\"/home#date-inline-home-maki-2\""));
    assert!(!month_body.contains("href=\"/home#date-property-home-maki-2\""));
    assert_text_order(
        &month_body,
        &[
            "href=\"/@/dates/2026-08-17\"",
            "href=\"/@/dates/2026-08-19\"",
            "href=\"/@/dates/2026-08-20\"",
        ],
    );

    let detail = handle_request(&state, &http::Request::get("/@/dates/2026-08-18")).unwrap();
    let detail_body = String::from_utf8(detail.body().to_vec()).unwrap();
    let empty_detail = handle_request(&state, &http::Request::get("/@/dates/2026-08-21")).unwrap();
    let empty_detail_body = String::from_utf8(empty_detail.body().to_vec()).unwrap();
    let property_detail =
        handle_request(&state, &http::Request::get("/@/dates/2026-08-20")).unwrap();
    let property_detail_body = String::from_utf8(property_detail.body().to_vec()).unwrap();
    let note = handle_request(&state, &http::Request::get("/home")).unwrap();
    let note_body = String::from_utf8(note.body().to_vec()).unwrap();
    fs::remove_dir_all(root).unwrap();

    assert_eq!(detail.status(), http::StatusCode::Ok);
    assert!(detail_body.contains("<title>Date 2026-08-18 Tue</title>"));
    assert!(detail_body.contains("<a href=\"/@/dates/2026-08-17\">← 2026-08-17 Mon</a>"));
    assert!(detail_body.contains("<a href=\"/@/dates/2026-08-19\">2026-08-19 Wed →</a>"));
    assert!(detail_body.contains("<a href=\"/@/dates/2026-08\">↑ 2026-08</a>"));
    assert!(detail_body.contains("[2026-08-17]--[2026-08-19]"));
    assert!(detail_body.contains("range"));
    assert!(detail_body.contains("<a href=\"/home#date-inline-home-maki-2\">Home</a>"));
    assert!(detail_body.contains("date, range, inline"));
    assert!(detail_body.contains("Plan &lt;2026-08-16&gt; and [2026-08-17]--[2026-08-19]."));

    assert_eq!(empty_detail.status(), http::StatusCode::Ok);
    assert!(empty_detail_body.contains("<title>Date 2026-08-21 Fri</title>"));
    assert!(empty_detail_body.contains("No date markers."));

    assert_eq!(property_detail.status(), http::StatusCode::Ok);
    assert!(property_detail_body.contains("<a href=\"/home#date-property-home-maki-2\">Home</a>"));
    assert!(property_detail_body.contains("event, single, property:scheduled"));
    assert!(property_detail_body.contains("scheduled: &lt;2026-08-20 15:00&gt;"));
    assert!(property_detail_body.contains("Task with property date."));

    assert_eq!(note.status(), http::StatusCode::Ok);
    assert!(note_body.contains("id=\"date-inline-home-maki-2\""));
    assert!(note_body.contains("<a class=\"maki-date-location maki-date-stamp maki-date-stamp-reference\" id=\"date-inline-home-maki-2\" href=\"/@/dates/2026-08-17#date-inline-home-maki-2\">[2026-08-17]</a>&ndash;<a class=\"maki-date-stamp maki-date-stamp-reference\" href=\"/@/dates/2026-08-19#date-inline-home-maki-2\">[2026-08-19]</a>"));
    assert!(note_body.contains("id=\"date-property-home-maki-2\""));
}

#[test]
fn test_dates_pages_list_month_and_iso_week_markers() {
    let root = std::env::temp_dir().join(format!("maki-period-dates-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("home.maki"),
        r#"--^ title: Home

Month [2026-08].
Week [2026-W23].
Specific [2026-W23-1].
Target [2026-06-01].
MonthWeek [2026-W32].
MixedTarget [2026-08-04]."#,
    )
    .unwrap();

    let maki = Maki::load(&root).unwrap();
    let state = AppState::new(maki);

    let year = handle_request(&state, &http::Request::get("/@/dates/2026")).unwrap();
    let year_body = String::from_utf8(year.body().to_vec()).unwrap();
    let month = handle_request(&state, &http::Request::get("/@/dates/2026-08")).unwrap();
    let month_body = String::from_utf8(month.body().to_vec()).unwrap();
    let week = handle_request(&state, &http::Request::get("/@/dates/2026-W23")).unwrap();
    let week_body = String::from_utf8(week.body().to_vec()).unwrap();
    let iso_weekday_alias =
        handle_request(&state, &http::Request::get("/@/dates/2026-W23-1")).unwrap();
    let iso_weekday_alias_body = String::from_utf8(iso_weekday_alias.body().to_vec()).unwrap();
    let day = handle_request(&state, &http::Request::get("/@/dates/2026-06-01")).unwrap();
    let day_body = String::from_utf8(day.body().to_vec()).unwrap();
    let month_day = handle_request(&state, &http::Request::get("/@/dates/2026-08-01")).unwrap();
    let month_day_body = String::from_utf8(month_day.body().to_vec()).unwrap();
    let contained_day = handle_request(&state, &http::Request::get("/@/dates/2026-08-04")).unwrap();
    let contained_day_body = String::from_utf8(contained_day.body().to_vec()).unwrap();
    fs::remove_dir_all(root).unwrap();

    assert_eq!(year.status(), http::StatusCode::Ok);
    assert!(year_body.contains("<h3 id=\"Weeks\">Weeks</h3>"));
    assert!(year_body.contains("<a href=\"/@/dates/2026-W23\">2026-W23</a>"));

    assert_eq!(month.status(), http::StatusCode::Ok);
    assert!(month_body.contains("<title>Month 2026-08</title>"));
    assert!(month_body.contains("date, month, inline"));
    assert!(month_body.contains("<a href=\"/@/dates/2026-W32\">2026-W32</a>"));
    assert!(!month_body.contains("href=\"/@/dates/2026-08-01\""));

    assert_eq!(week.status(), http::StatusCode::Ok);
    assert!(week_body.contains("<title>Week 2026-W23</title>"));
    assert!(week_body.contains("<a href=\"/@/dates/2026-W22\">← 2026-W22</a>"));
    assert!(week_body.contains("<a href=\"/@/dates/2026-W24\">2026-W24 →</a>"));
    assert!(week_body.contains("<a href=\"/@/dates/2026\">↑ 2026</a>"));
    assert!(week_body.contains("date, week, inline"));
    assert!(week_body.contains("<a href=\"/@/dates/2026-06-01\">2026-06-01 Mon</a>"));
    assert!(!week_body.contains("href=\"/@/dates/2026-06-07\""));

    assert_eq!(iso_weekday_alias.status(), http::StatusCode::Ok);
    assert!(iso_weekday_alias_body.contains("<title>Date 2026-06-01 Mon</title>"));

    assert_eq!(day.status(), http::StatusCode::Ok);
    assert!(day_body.contains("<a href=\"/@/dates/2026-W23\">↗ 2026-W23</a>"));
    let specific_position = day_body.find("/home#date-inline-home-maki-3").unwrap();
    let target_position = day_body.find("/home#date-inline-home-maki-4").unwrap();
    assert!(specific_position < target_position);
    assert!(day_body.contains("<h3 id=\"Containing Periods\">Containing Periods</h3>"));
    assert!(day_body.contains("<a href=\"/@/dates/2026-W23\">2026-W23</a>"));
    assert!(!day_body.contains("date, week day, inline"));

    assert_eq!(month_day.status(), http::StatusCode::Ok);
    assert!(month_day_body.contains("<title>Date 2026-08-01 Sat</title>"));
    assert!(month_day_body.contains("<h3 id=\"Containing Periods\">Containing Periods</h3>"));
    assert!(month_day_body.contains("<a href=\"/@/dates/2026-08\">2026-08</a>"));
    assert!(!month_day_body.contains("date, month day, inline"));

    assert_eq!(contained_day.status(), http::StatusCode::Ok);
    assert!(contained_day_body.contains("<title>Date 2026-08-04 Tue</title>"));
    assert!(contained_day_body.contains("<h3 id=\"Containing Periods\">Containing Periods</h3>"));
    assert_text_order(
        &contained_day_body,
        &[
            "<a href=\"/@/dates/2026-08\">2026-08</a>",
            "<a href=\"/@/dates/2026-W32\">2026-W32</a>",
        ],
    );
    assert!(contained_day_body.contains("date, single, inline"));
    assert!(!contained_day_body.contains("date, month day, inline"));
    assert!(!contained_day_body.contains("date, week day, inline"));
}

#[test]
fn test_date_pages_keep_duplicate_note_titles_unsuffixed() {
    let root =
        std::env::temp_dir().join(format!("maki-duplicate-date-links-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("alpha.maki"),
        r#"--^ title: Same

Alpha [2026-08-15]."#,
    )
    .unwrap();
    fs::write(
        root.join("beta.maki"),
        r#"--^ title: Same

Beta [2026-08-15]."#,
    )
    .unwrap();

    let maki = Maki::load(&root).unwrap();
    let state = AppState::new(maki);
    let day = handle_request(&state, &http::Request::get("/@/dates/2026-08-15")).unwrap();
    let day_body = String::from_utf8(day.body().to_vec()).unwrap();
    fs::remove_dir_all(root).unwrap();

    assert_eq!(day.status(), http::StatusCode::Ok);
    assert_eq!(day_body.matches(">Same</a>").count(), 2);
    assert!(day_body.contains("href=\"/alpha#"));
    assert!(day_body.contains("href=\"/beta#"));
    assert!(!day_body.contains("Same (2)"));
}

#[test]
fn test_date_index_hierarchy_is_ascending() {
    let root = std::env::temp_dir().join(format!("maki-ascending-dates-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("home.maki"),
        r#"Old [2025-12-31].
Months [2026-08] [2026-01-15] [2026-08-01] [2026-08-15] [2026-08-31] [2026-12-15].
Weeks [2026-W40] [2026-W03] [2026-09-28] [2026-10-01] [2026-10-04].
Future [2027-01-01]."#,
    )
    .unwrap();

    let maki = Maki::load(&root).unwrap();
    let state = AppState::new(maki);
    let body_for = |path: &str| {
        let response = handle_request(&state, &http::Request::get(path)).unwrap();
        assert_eq!(response.status(), http::StatusCode::Ok);
        String::from_utf8(response.body().to_vec()).unwrap()
    };

    let index_body = body_for("/@/dates");
    let year_body = body_for("/@/dates/2026");
    let month_body = body_for("/@/dates/2026-08");
    let week_body = body_for("/@/dates/2026-W40");
    fs::remove_dir_all(root).unwrap();

    assert_text_order(&index_body, &[">2025</a>", ">2026</a>", ">2027</a>"]);
    assert_text_order(
        &year_body,
        &[">2026-01</a>", ">2026-08</a>", ">2026-12</a>"],
    );
    assert_text_order(&year_body, &[">2026-W03</a>", ">2026-W40</a>"]);
    assert_text_order(&month_body, &[">2026-08-01", ">2026-08-15", ">2026-08-31"]);
    assert_text_order(&week_body, &[">2026-09-28", ">2026-10-01", ">2026-10-04"]);
}

fn assert_text_order(haystack: &str, needles: &[&str]) {
    let mut previous = None;
    for needle in needles {
        let position = haystack
            .find(needle)
            .unwrap_or_else(|| panic!("missing {needle:?}"));
        if let Some(previous) = previous {
            assert!(previous < position, "{needle:?} is out of order");
        }
        previous = Some(position);
    }
}

#[test]
fn test_iso_week_pages_handle_representable_year_boundaries() {
    let root = std::env::temp_dir().join(format!("maki-boundary-weeks-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("home.maki"),
        "First [0001-W01].\nLast [9999-W52].\nFriday [9999-W52-5].",
    )
    .unwrap();

    let maki = Maki::load(&root).unwrap();
    let state = AppState::new(maki);
    let first = handle_request(&state, &http::Request::get("/@/dates/0001-W01")).unwrap();
    let first_body = String::from_utf8(first.body().to_vec()).unwrap();
    let last = handle_request(&state, &http::Request::get("/@/dates/9999-W52")).unwrap();
    let last_body = String::from_utf8(last.body().to_vec()).unwrap();
    fs::remove_dir_all(root).unwrap();

    assert_eq!(first.status(), http::StatusCode::Ok);
    assert!(!first_body.contains("←"));
    assert!(first_body.contains("/@/dates/0001-W02"));

    assert_eq!(last.status(), http::StatusCode::Ok);
    assert!(last_body.contains("/@/dates/9999-12-31"));
    assert!(!last_body.contains("→"));
}

#[test]
fn test_diagnostics_page_lists_project_issues() {
    let root = std::env::temp_dir().join(format!("maki-diagnostics-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("home.maki"),
        "See [[missing]] and [Ghost].\n\n[Ghost]: ghost",
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
    assert!(body.contains("<h3 id=\"[home.maki]\"><a href=\"/home\">home.maki</a></h3>"));
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
