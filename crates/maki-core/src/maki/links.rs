use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::parser;

use super::note::NoteRef;

#[derive(Default)]
pub(super) struct NoteIndex {
    exact_paths: BTreeMap<PathBuf, NoteRef>,
    children_by_parent: BTreeMap<PathBuf, Vec<NoteRef>>,
}

impl NoteIndex {
    pub(super) fn build<'a>(note_refs: impl Iterator<Item = &'a NoteRef>) -> Self {
        let mut index = Self::default();

        for note_ref in note_refs {
            index.insert(note_ref);
        }

        index
    }

    fn insert(&mut self, note_ref: &NoteRef) {
        self.exact_paths
            .insert(note_ref.canonical_path().to_path_buf(), note_ref.clone());

        let parent = note_ref
            .canonical_path()
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf();
        push_candidate(&mut self.children_by_parent, parent, note_ref);
    }

    pub(super) fn exact_path(&self, target: &Path) -> Option<NoteRef> {
        self.exact_paths.get(target).cloned()
    }

    pub(super) fn direct_parent(&self, note_ref: &NoteRef) -> Option<NoteRef> {
        self.exact_path(note_ref.canonical_path().parent()?)
    }

    pub(super) fn direct_children(&self, note_ref: &NoteRef) -> &[NoteRef] {
        self.children_by_parent
            .get(note_ref.canonical_path())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

fn push_candidate<K>(map: &mut BTreeMap<K, Vec<NoteRef>>, key: K, note_ref: &NoteRef)
where
    K: Ord,
{
    map.entry(key).or_default().push(note_ref.clone());
}

pub(super) fn normalize_key(key: &str) -> String {
    key.to_lowercase()
}

pub fn is_external_href(target: &str) -> bool {
    let target = target.trim();

    target.starts_with("//") || parser::uri_scheme(target).is_some()
}

pub fn is_safe_direct_href(target: &str) -> bool {
    parser::is_local_link_target(target)
}

fn is_checkable_external_href(target: &str) -> bool {
    let target = target.trim();
    let Some((scheme, body)) = target.split_once("://") else {
        return false;
    };
    !body.is_empty()
        && (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ExternalLinkCheck {
    Ok,
    Broken { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExternalLinkCheckError {
    Status(u16),
    Transport(String),
}

impl std::fmt::Display for ExternalLinkCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(status) => write!(f, "HTTP {status}"),
            Self::Transport(message) => write!(f, "{message}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalLinkCheckMethod {
    Head,
    Get,
}

pub(super) fn check_external_link(target: &str) -> ExternalLinkCheck {
    if !is_checkable_external_href(target) {
        return ExternalLinkCheck::Ok;
    }

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(3))
        .redirects(5)
        .build();

    let result = match request_external_link(&agent, ExternalLinkCheckMethod::Head, target) {
        Ok(()) => return ExternalLinkCheck::Ok,
        Err(ExternalLinkCheckError::Status(_)) => {
            request_external_link(&agent, ExternalLinkCheckMethod::Get, target)
        }
        Err(error) => Err(error),
    };

    match result {
        Ok(()) => ExternalLinkCheck::Ok,
        Err(error) => ExternalLinkCheck::Broken {
            reason: error.to_string(),
        },
    }
}

fn request_external_link(
    agent: &ureq::Agent,
    method: ExternalLinkCheckMethod,
    target: &str,
) -> Result<(), ExternalLinkCheckError> {
    let response = match method {
        ExternalLinkCheckMethod::Head => agent.head(target).call(),
        ExternalLinkCheckMethod::Get => agent.get(target).call(),
    };

    match response {
        Ok(response) if response.status() < 400 => Ok(()),
        Ok(response) => Err(ExternalLinkCheckError::Status(response.status())),
        Err(ureq::Error::Status(status, _response)) => Err(ExternalLinkCheckError::Status(status)),
        Err(ureq::Error::Transport(error)) => {
            Err(ExternalLinkCheckError::Transport(error.to_string()))
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum NoteLinkResolution {
    Found(NoteRef),
    FoundHeading { note: NoteRef, anchor: String },
    FoundId { note: NoteRef, id: String },
    Broken,
    Ambiguous,
}
