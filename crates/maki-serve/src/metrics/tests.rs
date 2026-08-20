use std::time::Duration;

use super::Metrics;

#[test]
fn prometheus_text_escapes_label_values() {
    let metrics = Metrics::enabled();
    metrics.record_http_request("GET", "note", "200\nbad", Duration::from_millis(1), 7);

    let text = metrics.to_prometheus_text();

    assert!(text.contains("status=\"200\\nbad\""));
    assert!(!text.contains("200\nbad"));
}

#[test]
fn disabled_metrics_are_noop() {
    let metrics = Metrics::disabled();
    metrics.record_response_cache_request("note", "hit");

    assert_eq!(metrics.to_prometheus_text(), "");
}
