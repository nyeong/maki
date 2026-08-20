use super::*;

#[test]
fn metrics_endpoint_reports_serve_metrics_without_sensitive_labels() {
    let project = temp_project("serve-metrics");
    fs::write(
        project.root.join("home.maki"),
        "--^ title: Home\n\nHome body with private marker.\n",
    )
    .unwrap();

    let port = free_port();
    let metrics_port = free_port();
    let _server = start_server_with_metrics(&project.root, port, metrics_port);

    let page = http_get(port, "/home?secret=query-token");
    page.assert_status("HTTP/1.1 200 OK");
    let cached = http_get(port, "/home");
    cached.assert_status("HTTP/1.1 200 OK");

    let metrics = http_get(metrics_port, "/metrics");
    metrics.assert_status("HTTP/1.1 200 OK");
    metrics.assert_header_contains("content-type: text/plain; version=0.0.4; charset=utf-8");
    metrics.assert_body_contains("# TYPE maki_http_requests_total counter");
    metrics.assert_body_contains(
        "maki_http_requests_total{method=\"GET\",route=\"note\",status=\"200\"}",
    );
    metrics.assert_body_contains("# TYPE maki_response_cache_requests_total counter");
    metrics.assert_body_contains("# TYPE maki_render_duration_seconds histogram");
    metrics.assert_body_contains("maki_project_notes 1");
    metrics.assert_body_contains("maki_metrics_requests_total{method=\"GET\",status=\"200\"} 1");
    metrics.assert_body_excludes("query-token");
    metrics.assert_body_excludes("home.maki");
    metrics.assert_body_excludes("private marker");
}
