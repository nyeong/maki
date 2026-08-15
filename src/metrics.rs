use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const HTTP_DURATION_BUCKETS: &[f64] = &[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0];
const RESPONSE_BYTES_BUCKETS: &[f64] = &[
    128.0,
    512.0,
    1024.0,
    4096.0,
    16_384.0,
    65_536.0,
    262_144.0,
    1_048_576.0,
];
const RENDER_DURATION_BUCKETS: &[f64] = &[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0];
const RELOAD_DURATION_BUCKETS: &[f64] = &[0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0];
const PROJECT_LOAD_BUCKETS: &[f64] = &[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5];
const WARMUP_DURATION_BUCKETS: &[f64] = &[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0];

#[derive(Clone, Default)]
pub(crate) struct Metrics {
    inner: Option<Arc<MetricsInner>>,
}

struct MetricsInner {
    http_inflight_requests: AtomicU64,
    response_cache_entries: AtomicU64,
    project_notes: AtomicU64,
    git_last_success_timestamp_seconds: AtomicU64,
    live_reload_clients: AtomicU64,
    live_reload_events_total: AtomicU64,
    http_requests: Mutex<BTreeMap<HttpRequestLabels, u64>>,
    http_request_duration: Mutex<BTreeMap<HttpRequestLabels, Histogram>>,
    http_response_bytes: Mutex<BTreeMap<HttpResponseBytesLabels, Histogram>>,
    metrics_requests: Mutex<BTreeMap<MetricsRequestLabels, u64>>,
    metrics_request_duration: Mutex<BTreeMap<MetricsRequestLabels, Histogram>>,
    response_cache_requests: Mutex<BTreeMap<ResponseCacheLabels, u64>>,
    response_cache_warmup_items: Mutex<BTreeMap<ResponseCacheWarmupLabels, u64>>,
    response_cache_warmup_duration: Mutex<Histogram>,
    render_duration: Mutex<BTreeMap<KindLabels, Histogram>>,
    render_errors: Mutex<BTreeMap<KindLabels, u64>>,
    project_load_duration: Mutex<BTreeMap<PhaseLabels, Histogram>>,
    project_reload_total: Mutex<BTreeMap<ProjectReloadLabels, u64>>,
    project_reload_duration: Mutex<BTreeMap<SourceLabels, Histogram>>,
    git_refresh_total: Mutex<BTreeMap<ResultLabels, u64>>,
    git_refresh_duration: Mutex<Histogram>,
}

impl Default for MetricsInner {
    fn default() -> Self {
        Self {
            http_inflight_requests: AtomicU64::new(0),
            response_cache_entries: AtomicU64::new(0),
            project_notes: AtomicU64::new(0),
            git_last_success_timestamp_seconds: AtomicU64::new(0),
            live_reload_clients: AtomicU64::new(0),
            live_reload_events_total: AtomicU64::new(0),
            http_requests: Mutex::new(BTreeMap::new()),
            http_request_duration: Mutex::new(BTreeMap::new()),
            http_response_bytes: Mutex::new(BTreeMap::new()),
            metrics_requests: Mutex::new(BTreeMap::new()),
            metrics_request_duration: Mutex::new(BTreeMap::new()),
            response_cache_requests: Mutex::new(BTreeMap::new()),
            response_cache_warmup_items: Mutex::new(BTreeMap::new()),
            response_cache_warmup_duration: Mutex::new(Histogram::new(
                WARMUP_DURATION_BUCKETS.len(),
            )),
            render_duration: Mutex::new(BTreeMap::new()),
            render_errors: Mutex::new(BTreeMap::new()),
            project_load_duration: Mutex::new(BTreeMap::new()),
            project_reload_total: Mutex::new(BTreeMap::new()),
            project_reload_duration: Mutex::new(BTreeMap::new()),
            git_refresh_total: Mutex::new(BTreeMap::new()),
            git_refresh_duration: Mutex::new(Histogram::new(RELOAD_DURATION_BUCKETS.len())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct HttpRequestLabels {
    method: &'static str,
    route: &'static str,
    status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct HttpResponseBytesLabels {
    route: &'static str,
    status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct MetricsRequestLabels {
    method: &'static str,
    status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ResponseCacheLabels {
    kind: &'static str,
    cache: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ResponseCacheWarmupLabels {
    kind: &'static str,
    result: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct KindLabels {
    kind: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PhaseLabels {
    phase: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ProjectReloadLabels {
    source: &'static str,
    result: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SourceLabels {
    source: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ResultLabels {
    result: &'static str,
}

#[derive(Debug, Clone)]
struct Histogram {
    buckets: Vec<u64>,
    sum: f64,
    count: u64,
}

impl Histogram {
    fn new(bucket_count: usize) -> Self {
        Self {
            buckets: vec![0; bucket_count + 1],
            sum: 0.0,
            count: 0,
        }
    }

    fn observe(&mut self, buckets: &[f64], value: f64) {
        let bucket_index = buckets
            .iter()
            .position(|bucket| value <= *bucket)
            .unwrap_or(buckets.len());
        self.buckets[bucket_index] += 1;
        self.sum += value;
        self.count += 1;
    }
}

pub(crate) struct InflightRequest {
    metrics: Metrics,
}

impl Drop for InflightRequest {
    fn drop(&mut self) {
        if let Some(inner) = &self.metrics.inner {
            inner.http_inflight_requests.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl Metrics {
    pub(crate) fn enabled() -> Self {
        Self {
            inner: Some(Arc::new(MetricsInner::default())),
        }
    }

    pub(crate) fn disabled() -> Self {
        Self::default()
    }

    pub(crate) fn track_http_inflight_request(&self) -> InflightRequest {
        if let Some(inner) = &self.inner {
            inner.http_inflight_requests.fetch_add(1, Ordering::Relaxed);
        }

        InflightRequest {
            metrics: self.clone(),
        }
    }

    pub(crate) fn record_http_request(
        &self,
        method: &'static str,
        route: &'static str,
        status: impl Into<String>,
        duration: Duration,
        response_bytes: usize,
    ) {
        let Some(inner) = &self.inner else {
            return;
        };
        let status = status.into();
        let request_labels = HttpRequestLabels {
            method,
            route,
            status: status.clone(),
        };
        increment_counter(&inner.http_requests, request_labels.clone());
        observe_histogram_map(
            &inner.http_request_duration,
            request_labels,
            HTTP_DURATION_BUCKETS,
            duration_to_seconds(duration),
        );
        observe_histogram_map(
            &inner.http_response_bytes,
            HttpResponseBytesLabels { route, status },
            RESPONSE_BYTES_BUCKETS,
            response_bytes as f64,
        );
    }

    pub(crate) fn record_metrics_request(
        &self,
        method: &'static str,
        status: impl Into<String>,
        duration: Duration,
    ) {
        let Some(inner) = &self.inner else {
            return;
        };
        let labels = MetricsRequestLabels {
            method,
            status: status.into(),
        };
        increment_counter(&inner.metrics_requests, labels.clone());
        observe_histogram_map(
            &inner.metrics_request_duration,
            labels,
            HTTP_DURATION_BUCKETS,
            duration_to_seconds(duration),
        );
    }

    pub(crate) fn record_response_cache_request(&self, kind: &'static str, cache: &'static str) {
        let Some(inner) = &self.inner else {
            return;
        };
        increment_counter(
            &inner.response_cache_requests,
            ResponseCacheLabels { kind, cache },
        );
    }

    pub(crate) fn set_response_cache_entries(&self, entries: usize) {
        if let Some(inner) = &self.inner {
            inner
                .response_cache_entries
                .store(entries as u64, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_response_cache_warmup_item(
        &self,
        kind: &'static str,
        result: &'static str,
    ) {
        let Some(inner) = &self.inner else {
            return;
        };
        increment_counter(
            &inner.response_cache_warmup_items,
            ResponseCacheWarmupLabels { kind, result },
        );
    }

    pub(crate) fn record_response_cache_warmup_duration(&self, duration: Duration) {
        if let Some(inner) = &self.inner
            && let Ok(mut histogram) = inner.response_cache_warmup_duration.lock()
        {
            histogram.observe(WARMUP_DURATION_BUCKETS, duration_to_seconds(duration));
        }
    }

    pub(crate) fn record_render_duration(&self, kind: &'static str, duration: Duration) {
        let Some(inner) = &self.inner else {
            return;
        };
        observe_histogram_map(
            &inner.render_duration,
            KindLabels { kind },
            RENDER_DURATION_BUCKETS,
            duration_to_seconds(duration),
        );
    }

    pub(crate) fn record_render_error(&self, kind: &'static str) {
        let Some(inner) = &self.inner else {
            return;
        };
        increment_counter(&inner.render_errors, KindLabels { kind });
    }

    pub(crate) fn set_project_notes(&self, notes: usize) {
        if let Some(inner) = &self.inner {
            inner.project_notes.store(notes as u64, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_project_load_phase(&self, phase: &'static str, duration: Duration) {
        let Some(inner) = &self.inner else {
            return;
        };
        observe_histogram_map(
            &inner.project_load_duration,
            PhaseLabels { phase },
            PROJECT_LOAD_BUCKETS,
            duration_to_seconds(duration),
        );
    }

    pub(crate) fn record_project_reload(
        &self,
        source: &'static str,
        result: &'static str,
        duration: Duration,
    ) {
        let Some(inner) = &self.inner else {
            return;
        };
        increment_counter(
            &inner.project_reload_total,
            ProjectReloadLabels { source, result },
        );
        observe_histogram_map(
            &inner.project_reload_duration,
            SourceLabels { source },
            RELOAD_DURATION_BUCKETS,
            duration_to_seconds(duration),
        );
    }

    pub(crate) fn record_git_refresh(&self, result: &'static str, duration: Duration) {
        let Some(inner) = &self.inner else {
            return;
        };
        increment_counter(&inner.git_refresh_total, ResultLabels { result });
        if let Ok(mut histogram) = inner.git_refresh_duration.lock() {
            histogram.observe(RELOAD_DURATION_BUCKETS, duration_to_seconds(duration));
        }
        if result == "ok" {
            inner
                .git_last_success_timestamp_seconds
                .store(unix_timestamp(), Ordering::Relaxed);
        }
    }

    pub(crate) fn set_live_reload_clients(&self, clients: usize) {
        if let Some(inner) = &self.inner {
            inner
                .live_reload_clients
                .store(clients as u64, Ordering::Relaxed);
        }
    }

    pub(crate) fn increment_live_reload_events(&self) {
        if let Some(inner) = &self.inner {
            inner
                .live_reload_events_total
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn to_prometheus_text(&self) -> String {
        let Some(inner) = &self.inner else {
            return String::new();
        };

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
}

fn increment_counter<K>(map: &Mutex<BTreeMap<K, u64>>, key: K)
where
    K: Ord,
{
    if let Ok(mut values) = map.lock() {
        *values.entry(key).or_default() += 1;
    }
}

fn observe_histogram_map<K>(
    map: &Mutex<BTreeMap<K, Histogram>>,
    key: K,
    buckets: &[f64],
    value: f64,
) where
    K: Ord,
{
    if let Ok(mut values) = map.lock() {
        values
            .entry(key)
            .or_insert_with(|| Histogram::new(buckets.len()))
            .observe(buckets, value);
    }
}

fn duration_to_seconds(duration: Duration) -> f64 {
    duration.as_secs_f64()
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
