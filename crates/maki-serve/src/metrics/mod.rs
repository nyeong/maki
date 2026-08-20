use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use maki_core::ProjectLoadMeter;

mod histogram;
mod labels;
mod prometheus;

#[cfg(test)]
mod tests;

use histogram::Histogram;
use labels::{
    HttpRequestLabels, HttpResponseBytesLabels, KindLabels, MetricsRequestLabels, PhaseLabels,
    ProjectReloadLabels, ResponseCacheLabels, ResponseCacheWarmupLabels, ResultLabels,
    SourceLabels,
};

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
pub struct Metrics {
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
pub struct InflightRequest {
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
    pub fn enabled() -> Self {
        Self {
            inner: Some(Arc::new(MetricsInner::default())),
        }
    }

    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn track_http_inflight_request(&self) -> InflightRequest {
        if let Some(inner) = &self.inner {
            inner.http_inflight_requests.fetch_add(1, Ordering::Relaxed);
        }

        InflightRequest {
            metrics: self.clone(),
        }
    }

    pub fn record_http_request(
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

    pub fn record_metrics_request(
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

    pub fn record_response_cache_request(&self, kind: &'static str, cache: &'static str) {
        let Some(inner) = &self.inner else {
            return;
        };
        increment_counter(
            &inner.response_cache_requests,
            ResponseCacheLabels { kind, cache },
        );
    }

    pub fn set_response_cache_entries(&self, entries: usize) {
        if let Some(inner) = &self.inner {
            inner
                .response_cache_entries
                .store(entries as u64, Ordering::Relaxed);
        }
    }

    pub fn record_response_cache_warmup_item(&self, kind: &'static str, result: &'static str) {
        let Some(inner) = &self.inner else {
            return;
        };
        increment_counter(
            &inner.response_cache_warmup_items,
            ResponseCacheWarmupLabels { kind, result },
        );
    }

    pub fn record_response_cache_warmup_duration(&self, duration: Duration) {
        if let Some(inner) = &self.inner
            && let Ok(mut histogram) = inner.response_cache_warmup_duration.lock()
        {
            histogram.observe(WARMUP_DURATION_BUCKETS, duration_to_seconds(duration));
        }
    }

    pub fn record_render_duration(&self, kind: &'static str, duration: Duration) {
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

    pub fn record_render_error(&self, kind: &'static str) {
        let Some(inner) = &self.inner else {
            return;
        };
        increment_counter(&inner.render_errors, KindLabels { kind });
    }

    pub fn set_project_notes(&self, notes: usize) {
        if let Some(inner) = &self.inner {
            inner.project_notes.store(notes as u64, Ordering::Relaxed);
        }
    }

    pub fn record_project_load_phase(&self, phase: &'static str, duration: Duration) {
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

    pub fn record_project_reload(
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

    pub fn record_git_refresh(&self, result: &'static str, duration: Duration) {
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

    pub fn set_live_reload_clients(&self, clients: usize) {
        if let Some(inner) = &self.inner {
            inner
                .live_reload_clients
                .store(clients as u64, Ordering::Relaxed);
        }
    }

    pub fn increment_live_reload_events(&self) {
        if let Some(inner) = &self.inner {
            inner
                .live_reload_events_total
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn to_prometheus_text(&self) -> String {
        let Some(inner) = &self.inner else {
            return String::new();
        };

        prometheus::to_prometheus_text(inner)
    }
}

impl ProjectLoadMeter for Metrics {
    fn record_project_load_phase(&self, phase: &'static str, duration: Duration) {
        Metrics::record_project_load_phase(self, phase, duration);
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
