#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct HttpRequestLabels {
    pub(super) method: &'static str,
    pub(super) route: &'static str,
    pub(super) status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct HttpResponseBytesLabels {
    pub(super) route: &'static str,
    pub(super) status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct MetricsRequestLabels {
    pub(super) method: &'static str,
    pub(super) status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ResponseCacheLabels {
    pub(super) kind: &'static str,
    pub(super) cache: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ResponseCacheWarmupLabels {
    pub(super) kind: &'static str,
    pub(super) result: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct KindLabels {
    pub(super) kind: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct PhaseLabels {
    pub(super) phase: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ProjectReloadLabels {
    pub(super) source: &'static str,
    pub(super) result: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct SourceLabels {
    pub(super) source: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ResultLabels {
    pub(super) result: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct LiveReloadDisconnectLabels {
    pub(super) reason: &'static str,
}
