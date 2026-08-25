use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use crate::http;
use crate::metrics::Metrics;
use maki_core::{DatePeriod, Error as MakiError, Maki, MakiConfig, MakiConfigOverrides};

use super::MAX_SSE_CLIENTS;
use super::live_reload::{LiveReload, inject_live_reload_script};
use super::watch::{FileSnapshot, collect_watched_project_snapshot};

pub(super) struct AppState {
    project_root: PathBuf,
    config_overrides: MakiConfigOverrides,
    pub(super) project: RwLock<ProjectState>,
    live_reload: Option<LiveReload>,
    metrics: Metrics,
}

pub(super) struct ProjectState {
    pub(super) maki: Maki,
    response_cache: Mutex<BTreeMap<ResponseCacheKey, http::Response>>,
}

impl ProjectState {
    pub(super) fn new(maki: Maki) -> Self {
        Self {
            maki,
            response_cache: Mutex::new(BTreeMap::new()),
        }
    }

    pub(super) fn cached_response(&self, key: &ResponseCacheKey) -> Option<http::Response> {
        self.response_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(key).cloned())
    }

    pub(super) fn insert_response(
        &self,
        key: ResponseCacheKey,
        response: http::Response,
    ) -> Option<usize> {
        self.response_cache.lock().ok().map(|mut cache| {
            cache.insert(key, response);
            cache.len()
        })
    }

    #[cfg(test)]
    pub(super) fn cached_response_count(&self) -> usize {
        self.response_cache
            .lock()
            .map(|cache| cache.len())
            .unwrap_or_default()
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ResponseCacheKey {
    MetaIndex,
    Recents,
    Sitemap,
    SitemapXml,
    Diagnostics,
    DatesIndex,
    DatePeriodPage(DatePeriod),
    ProjectIndex,
    SearchIndex,
    NotePage(PathBuf),
}

impl ResponseCacheKey {
    pub(super) fn kind(&self) -> &'static str {
        match self {
            Self::MetaIndex => "meta",
            Self::Recents => "recents",
            Self::Sitemap => "sitemap",
            Self::SitemapXml => "sitemap_xml",
            Self::Diagnostics => "diagnostics",
            Self::DatesIndex => "dates",
            Self::DatePeriodPage(_) => "date",
            Self::ProjectIndex => "project_index",
            Self::SearchIndex => "search_index",
            Self::NotePage(_) => "note",
        }
    }
}

impl AppState {
    #[cfg(test)]
    pub(super) fn new(maki: Maki) -> Self {
        Self::new_with_overrides(
            maki.root().to_path_buf(),
            maki,
            MakiConfigOverrides::default(),
            true,
            Metrics::disabled(),
        )
    }

    #[cfg(test)]
    pub(super) fn new_with_metrics(maki: Maki, metrics: Metrics) -> Self {
        Self::new_with_overrides(
            maki.root().to_path_buf(),
            maki,
            MakiConfigOverrides::default(),
            true,
            metrics,
        )
    }

    pub(super) fn new_with_overrides(
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

    pub(super) fn reload(&self) -> Result<(), MakiError> {
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

    pub(super) fn replace_maki(&self, next: Maki) -> Result<(), MakiError> {
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

    pub(super) fn with_live_reload(&self, html: String) -> String {
        match &self.live_reload {
            Some(live_reload) => inject_live_reload_script(html, &live_reload.token()),
            None => html,
        }
    }

    pub(super) fn live_reload(&self) -> Option<&LiveReload> {
        self.live_reload.as_ref()
    }

    pub(super) fn watched_snapshot(&self) -> Result<FileSnapshot, std::io::Error> {
        let source_root = self
            .current_root()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        collect_watched_project_snapshot(&self.project_root, &source_root)
    }

    #[cfg(test)]
    pub(super) fn cached_response_count(&self) -> usize {
        self.project
            .read()
            .map(|project| project.cached_response_count())
            .unwrap_or_default()
    }

    pub(super) fn metrics(&self) -> &Metrics {
        &self.metrics
    }
}

#[derive(Clone)]
pub struct ProjectReloader {
    state: Arc<AppState>,
}

impl ProjectReloader {
    pub(super) fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    pub fn replace_maki(&self, next: Maki) -> Result<(), MakiError> {
        self.state.replace_maki(next)
    }

    pub fn metrics(&self) -> &Metrics {
        self.state.metrics()
    }
}
