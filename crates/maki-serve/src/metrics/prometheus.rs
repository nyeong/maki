use std::fmt::Write as _;
use std::sync::atomic::Ordering;

use super::histogram::Histogram;
use super::{
    HTTP_DURATION_BUCKETS, MetricsInner, PROJECT_LOAD_BUCKETS, RELOAD_DURATION_BUCKETS,
    RENDER_DURATION_BUCKETS, RESPONSE_BYTES_BUCKETS, WARMUP_DURATION_BUCKETS,
};

pub(super) fn to_prometheus_text(inner: &MetricsInner) -> String {
    let mut output = String::new();
    write_counter_family(
        &mut output,
        "maki_http_requests_total",
        "HTTP application requests by low-cardinality route.",
    );
    if let Ok(values) = inner.http_requests.lock() {
        for (labels, value) in values.iter() {
            write_counter(
                &mut output,
                "maki_http_requests_total",
                &[
                    ("method", labels.method),
                    ("route", labels.route),
                    ("status", &labels.status),
                ],
                *value,
            );
        }
    }

    write_histogram_family(
        &mut output,
        "maki_http_request_duration_seconds",
        "HTTP application request duration in seconds.",
    );
    if let Ok(values) = inner.http_request_duration.lock() {
        for (labels, histogram) in values.iter() {
            write_histogram(
                &mut output,
                "maki_http_request_duration_seconds",
                &[
                    ("method", labels.method),
                    ("route", labels.route),
                    ("status", &labels.status),
                ],
                HTTP_DURATION_BUCKETS,
                histogram,
            );
        }
    }

    write_histogram_family(
        &mut output,
        "maki_http_response_bytes",
        "HTTP application response body size in bytes.",
    );
    if let Ok(values) = inner.http_response_bytes.lock() {
        for (labels, histogram) in values.iter() {
            write_histogram(
                &mut output,
                "maki_http_response_bytes",
                &[("route", labels.route), ("status", &labels.status)],
                RESPONSE_BYTES_BUCKETS,
                histogram,
            );
        }
    }

    write_gauge_family(
        &mut output,
        "maki_http_inflight_requests",
        "Current application HTTP requests in flight.",
    );
    write_gauge(
        &mut output,
        "maki_http_inflight_requests",
        &[],
        inner.http_inflight_requests.load(Ordering::Relaxed),
    );

    write_counter_family(
        &mut output,
        "maki_metrics_requests_total",
        "Metrics endpoint requests.",
    );
    if let Ok(values) = inner.metrics_requests.lock() {
        for (labels, value) in values.iter() {
            write_counter(
                &mut output,
                "maki_metrics_requests_total",
                &[("method", labels.method), ("status", &labels.status)],
                *value,
            );
        }
    }

    write_histogram_family(
        &mut output,
        "maki_metrics_request_duration_seconds",
        "Metrics endpoint request duration in seconds.",
    );
    if let Ok(values) = inner.metrics_request_duration.lock() {
        for (labels, histogram) in values.iter() {
            write_histogram(
                &mut output,
                "maki_metrics_request_duration_seconds",
                &[("method", labels.method), ("status", &labels.status)],
                HTTP_DURATION_BUCKETS,
                histogram,
            );
        }
    }

    write_counter_family(
        &mut output,
        "maki_response_cache_requests_total",
        "Response cache lookups by cacheable response kind.",
    );
    if let Ok(values) = inner.response_cache_requests.lock() {
        for (labels, value) in values.iter() {
            write_counter(
                &mut output,
                "maki_response_cache_requests_total",
                &[("kind", labels.kind), ("cache", labels.cache)],
                *value,
            );
        }
    }

    write_gauge_family(
        &mut output,
        "maki_response_cache_entries",
        "Current response cache entry count for the active project snapshot.",
    );
    write_gauge(
        &mut output,
        "maki_response_cache_entries",
        &[],
        inner.response_cache_entries.load(Ordering::Relaxed),
    );

    write_counter_family(
        &mut output,
        "maki_response_cache_warmup_items_total",
        "Response cache warmup items by kind and result.",
    );
    if let Ok(values) = inner.response_cache_warmup_items.lock() {
        for (labels, value) in values.iter() {
            write_counter(
                &mut output,
                "maki_response_cache_warmup_items_total",
                &[("kind", labels.kind), ("result", labels.result)],
                *value,
            );
        }
    }

    write_histogram_family(
        &mut output,
        "maki_response_cache_warmup_duration_seconds",
        "Response cache warmup duration in seconds.",
    );
    if let Ok(histogram) = inner.response_cache_warmup_duration.lock() {
        write_histogram(
            &mut output,
            "maki_response_cache_warmup_duration_seconds",
            &[],
            WARMUP_DURATION_BUCKETS,
            &histogram,
        );
    }

    write_histogram_family(
        &mut output,
        "maki_render_duration_seconds",
        "Render duration in seconds by rendered response kind.",
    );
    if let Ok(values) = inner.render_duration.lock() {
        for (labels, histogram) in values.iter() {
            write_histogram(
                &mut output,
                "maki_render_duration_seconds",
                &[("kind", labels.kind)],
                RENDER_DURATION_BUCKETS,
                histogram,
            );
        }
    }

    write_counter_family(
        &mut output,
        "maki_render_errors_total",
        "Render errors by rendered response kind.",
    );
    if let Ok(values) = inner.render_errors.lock() {
        for (labels, value) in values.iter() {
            write_counter(
                &mut output,
                "maki_render_errors_total",
                &[("kind", labels.kind)],
                *value,
            );
        }
    }

    write_gauge_family(
        &mut output,
        "maki_project_notes",
        "Current active project note count.",
    );
    write_gauge(
        &mut output,
        "maki_project_notes",
        &[],
        inner.project_notes.load(Ordering::Relaxed),
    );

    write_histogram_family(
        &mut output,
        "maki_project_load_duration_seconds",
        "Project load phase duration in seconds.",
    );
    if let Ok(values) = inner.project_load_duration.lock() {
        for (labels, histogram) in values.iter() {
            write_histogram(
                &mut output,
                "maki_project_load_duration_seconds",
                &[("phase", labels.phase)],
                PROJECT_LOAD_BUCKETS,
                histogram,
            );
        }
    }

    write_counter_family(
        &mut output,
        "maki_project_reload_total",
        "Project reload attempts by source and result.",
    );
    if let Ok(values) = inner.project_reload_total.lock() {
        for (labels, value) in values.iter() {
            write_counter(
                &mut output,
                "maki_project_reload_total",
                &[("source", labels.source), ("result", labels.result)],
                *value,
            );
        }
    }

    write_histogram_family(
        &mut output,
        "maki_project_reload_duration_seconds",
        "Project reload duration in seconds by source.",
    );
    if let Ok(values) = inner.project_reload_duration.lock() {
        for (labels, histogram) in values.iter() {
            write_histogram(
                &mut output,
                "maki_project_reload_duration_seconds",
                &[("source", labels.source)],
                RELOAD_DURATION_BUCKETS,
                histogram,
            );
        }
    }

    write_counter_family(
        &mut output,
        "maki_git_refresh_total",
        "Git refresh attempts by result.",
    );
    if let Ok(values) = inner.git_refresh_total.lock() {
        for (labels, value) in values.iter() {
            write_counter(
                &mut output,
                "maki_git_refresh_total",
                &[("result", labels.result)],
                *value,
            );
        }
    }

    write_histogram_family(
        &mut output,
        "maki_git_refresh_duration_seconds",
        "Git refresh duration in seconds.",
    );
    if let Ok(histogram) = inner.git_refresh_duration.lock() {
        write_histogram(
            &mut output,
            "maki_git_refresh_duration_seconds",
            &[],
            RELOAD_DURATION_BUCKETS,
            &histogram,
        );
    }

    write_gauge_family(
        &mut output,
        "maki_git_last_success_timestamp_seconds",
        "Unix timestamp for the last successful git refresh.",
    );
    write_gauge(
        &mut output,
        "maki_git_last_success_timestamp_seconds",
        &[],
        inner
            .git_last_success_timestamp_seconds
            .load(Ordering::Relaxed),
    );

    write_gauge_family(
        &mut output,
        "maki_live_reload_clients",
        "Current live reload SSE client count.",
    );
    write_gauge(
        &mut output,
        "maki_live_reload_clients",
        &[],
        inner.live_reload_clients.load(Ordering::Relaxed),
    );

    write_counter_family(
        &mut output,
        "maki_live_reload_events_total",
        "Live reload events emitted.",
    );
    write_counter(
        &mut output,
        "maki_live_reload_events_total",
        &[],
        inner.live_reload_events_total.load(Ordering::Relaxed),
    );

    output
}

fn write_counter_family(output: &mut String, name: &str, help: &str) {
    write_family(output, name, help, "counter");
}

fn write_gauge_family(output: &mut String, name: &str, help: &str) {
    write_family(output, name, help, "gauge");
}

fn write_histogram_family(output: &mut String, name: &str, help: &str) {
    write_family(output, name, help, "histogram");
}

fn write_family(output: &mut String, name: &str, help: &str, metric_type: &str) {
    let _ = writeln!(output, "# HELP {name} {}", escape_help(help));
    let _ = writeln!(output, "# TYPE {name} {metric_type}");
}

fn write_counter(output: &mut String, name: &str, labels: &[(&str, &str)], value: u64) {
    write_sample(output, name, labels, &value.to_string());
}

fn write_gauge(output: &mut String, name: &str, labels: &[(&str, &str)], value: u64) {
    write_sample(output, name, labels, &value.to_string());
}

fn write_histogram(
    output: &mut String,
    name: &str,
    labels: &[(&str, &str)],
    buckets: &[f64],
    histogram: &Histogram,
) {
    let mut cumulative = 0;
    for (index, bucket) in buckets.iter().enumerate() {
        cumulative += histogram.buckets.get(index).copied().unwrap_or_default();
        let bound = format_float(*bucket);
        let mut labels_with_le = labels.to_vec();
        labels_with_le.push(("le", &bound));
        write_sample(
            output,
            &format!("{name}_bucket"),
            &labels_with_le,
            &cumulative.to_string(),
        );
    }

    cumulative += histogram
        .buckets
        .get(buckets.len())
        .copied()
        .unwrap_or_default();
    let mut labels_with_le = labels.to_vec();
    labels_with_le.push(("le", "+Inf"));
    write_sample(
        output,
        &format!("{name}_bucket"),
        &labels_with_le,
        &cumulative.to_string(),
    );
    write_sample(
        output,
        &format!("{name}_sum"),
        labels,
        &format_float(histogram.sum),
    );
    write_sample(
        output,
        &format!("{name}_count"),
        labels,
        &histogram.count.to_string(),
    );
}

fn write_sample(output: &mut String, name: &str, labels: &[(&str, &str)], value: &str) {
    output.push_str(name);
    write_labels(output, labels);
    output.push(' ');
    output.push_str(value);
    output.push('\n');
}

fn write_labels(output: &mut String, labels: &[(&str, &str)]) {
    if labels.is_empty() {
        return;
    }

    output.push('{');
    for (index, (key, value)) in labels.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push_str(key);
        output.push_str("=\"");
        output.push_str(&escape_label_value(value));
        output.push('"');
    }
    output.push('}');
}

fn escape_help(input: &str) -> String {
    input.replace('\\', r"\\").replace('\n', r"\n")
}

fn escape_label_value(input: &str) -> String {
    let mut escaped = String::new();
    for ch in input.chars() {
        match ch {
            '\\' => escaped.push_str(r"\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str(r"\n"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn format_float(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}
