use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{
    link_target::DocumentSelector,
    parser::{self, BlockKind, Inline},
};

use super::{MAKI_SOURCE_EXTENSION, note::NoteRef, quote_mode_is_raw};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ExternalLinkRef {
    pub(super) source_path: PathBuf,
    pub(super) target: String,
}

#[derive(Default)]
pub(super) struct NoteIndex {
    exact_paths: BTreeMap<PathBuf, NoteRef>,
    normalized_paths: BTreeMap<String, Vec<NoteRef>>,
    normalized_stems: BTreeMap<String, Vec<NoteRef>>,
    sibling_normalized_stems: BTreeMap<(PathBuf, String), Vec<NoteRef>>,
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
        push_candidate(
            &mut self.normalized_paths,
            normalize_path(note_ref.canonical_path()),
            note_ref,
        );

        let Some(stem) = note_ref
            .canonical_path()
            .file_name()
            .and_then(|name| name.to_str())
            .map(normalize_key)
        else {
            return;
        };

        push_candidate(&mut self.normalized_stems, stem.clone(), note_ref);

        let parent = note_ref
            .canonical_path()
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf();
        push_candidate(
            &mut self.sibling_normalized_stems,
            (parent.clone(), stem),
            note_ref,
        );
        push_candidate(&mut self.children_by_parent, parent, note_ref);
    }

    pub(super) fn exact_path(&self, target: &Path) -> Option<NoteRef> {
        self.exact_paths.get(target).cloned()
    }

    pub(super) fn resolve_normalized_path(&self, target: &Path) -> Option<NoteLinkResolution> {
        resolve_candidates(self.normalized_paths.get(&normalize_path(target)))
    }

    pub(super) fn resolve_sibling_stem(
        &self,
        current: &NoteRef,
        target: &str,
    ) -> Option<NoteLinkResolution> {
        let parent = current.canonical_path().parent()?.to_path_buf();
        let key = (parent, normalize_key(target));

        resolve_candidates(self.sibling_normalized_stems.get(&key))
    }

    pub(super) fn resolve_project_stem(&self, target: &str) -> Option<NoteLinkResolution> {
        resolve_candidates(self.normalized_stems.get(&normalize_key(target)))
    }

    pub(super) fn resolve_document(
        &self,
        current: &NoteRef,
        selector: DocumentSelector<'_>,
    ) -> NoteLinkResolution {
        match selector {
            DocumentSelector::Current => NoteLinkResolution::Found(current.clone()),
            DocumentSelector::Root(target) => self.resolve_coordinate(target),
            DocumentSelector::Child(target) => {
                let target = normalize_document_coordinate(target);
                if !is_normal_relative_target(target) {
                    return NoteLinkResolution::Broken;
                }
                self.resolve_path(&current.canonical_path().join(Path::new(target)))
            }
            DocumentSelector::Legacy(target) => self.resolve_legacy(current, target),
        }
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

    fn resolve_coordinate(&self, target: &str) -> NoteLinkResolution {
        let target = normalize_document_coordinate(target);
        if target.is_empty() {
            return NoteLinkResolution::Broken;
        }
        self.resolve_path(Path::new(target))
    }

    fn resolve_path(&self, target: &Path) -> NoteLinkResolution {
        self.exact_path(target)
            .map(NoteLinkResolution::Found)
            .or_else(|| self.resolve_normalized_path(target))
            .unwrap_or(NoteLinkResolution::Broken)
    }

    fn resolve_legacy(&self, current: &NoteRef, target: &str) -> NoteLinkResolution {
        let target = normalize_document_coordinate(target);

        if let Some(note_ref) = self.exact_path(Path::new(target)) {
            return NoteLinkResolution::Found(note_ref);
        }
        if target.contains('/')
            && let Some(resolution) = self.resolve_normalized_path(Path::new(target))
        {
            return resolution;
        }
        if !target.contains('/') {
            if let Some(resolution) = self.resolve_sibling_stem(current, target) {
                return resolution;
            }
            if let Some(resolution) = self.resolve_project_stem(target) {
                return resolution;
            }
        }

        NoteLinkResolution::Broken
    }
}

fn normalize_document_coordinate(target: &str) -> &str {
    target.strip_suffix(MAKI_SOURCE_EXTENSION).unwrap_or(target)
}

fn is_normal_relative_target(target: &str) -> bool {
    !target.is_empty()
        && target
            .split('/')
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
        && Path::new(target)
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn push_candidate<K>(map: &mut BTreeMap<K, Vec<NoteRef>>, key: K, note_ref: &NoteRef)
where
    K: Ord,
{
    map.entry(key).or_default().push(note_ref.clone());
}

fn normalize_path(path: &Path) -> String {
    normalize_key(&path.to_string_lossy())
}

pub(super) fn normalize_key(key: &str) -> String {
    key.to_lowercase()
}

fn has_uri_scheme(target: &str) -> bool {
    let Some((scheme, _rest)) = target.split_once(':') else {
        return false;
    };

    !scheme.is_empty()
        && scheme
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
}

pub fn is_external_href(target: &str) -> bool {
    let target = target.trim();

    target.starts_with("//") || has_uri_scheme(target)
}

fn is_checkable_external_href(target: &str) -> bool {
    let target = target.trim();

    target.starts_with("https://") || target.starts_with("http://")
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

pub fn note_link_target_for_href(target: &str) -> Option<String> {
    let target = target.trim();

    if target.is_empty()
        || target.contains('#')
        || target.contains('@')
        || target.contains('?')
        || target.starts_with('+')
        || target.starts_with("//")
        || has_uri_scheme(target)
    {
        return None;
    }

    let path_part = target.strip_prefix('/').unwrap_or(target);
    if path_part == "@" || path_part.starts_with("@/") || path_part.starts_with(".maki/") {
        return None;
    }

    let extension = Path::new(path_part)
        .extension()
        .and_then(|ext| ext.to_str());

    if !matches!(extension, Some("maki") | None) {
        return None;
    }

    let normalized = path_part
        .strip_suffix(MAKI_SOURCE_EXTENSION)
        .unwrap_or(path_part);
    Some(if target.starts_with('/') {
        format!("/{normalized}")
    } else {
        normalized.to_string()
    })
}

fn resolve_candidates(candidates: Option<&Vec<NoteRef>>) -> Option<NoteLinkResolution> {
    let candidates = candidates?;

    match candidates.as_slice() {
        [] => None,
        [note_ref] => Some(NoteLinkResolution::Found(note_ref.clone())),
        _ => Some(NoteLinkResolution::Ambiguous),
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
fn collect_inline_external_links(
    external_links: &mut BTreeSet<ExternalLinkRef>,
    source_path: &Path,
    inlines: &[Inline<'_>],
) {
    for inline in inlines {
        match inline {
            Inline::HyperLink { target } => {
                external_links.insert(ExternalLinkRef {
                    source_path: source_path.to_path_buf(),
                    target: target.trim().to_string(),
                });
            }
            _ => {
                if let Some(body) = inline.nested_inlines() {
                    collect_inline_external_links(external_links, source_path, body);
                }
            }
        }
    }
}

fn collect_table_row_external_links(
    external_links: &mut BTreeSet<ExternalLinkRef>,
    source_path: &Path,
    row: &parser::TableRow<'_>,
) {
    if row.is_separator() {
        return;
    }

    for cell in &row.cells {
        collect_inline_external_links(external_links, source_path, &cell.body);
    }
}

fn collect_block_external_links(
    external_links: &mut BTreeSet<ExternalLinkRef>,
    source_path: &Path,
    block: &parser::Block<'_>,
    references: &parser::ReferenceDefinitions<'_>,
) {
    match &block.kind {
        BlockKind::Paragraph { body } => {
            collect_inline_external_links(external_links, source_path, body)
        }
        BlockKind::Heading { body, .. } => {
            collect_inline_external_links(external_links, source_path, body);
        }
        BlockKind::List { items } => {
            for item in items {
                collect_inline_external_links(external_links, source_path, &item.body);
                for child in &item.children {
                    collect_block_external_links(external_links, source_path, child, references);
                }
            }
        }
        BlockKind::Quote { lines } if !quote_mode_is_raw(block.property("mode")) => {
            collect_maki_lines_external_links(external_links, source_path, lines, references)
        }
        BlockKind::Table { header, rows, .. } => {
            collect_table_row_external_links(external_links, source_path, header);
            for row in rows {
                collect_table_row_external_links(external_links, source_path, row);
            }
        }
        BlockKind::Container { kind, lines, .. }
            if *kind == "quote" && !quote_mode_is_raw(block.property("mode")) =>
        {
            collect_maki_lines_external_links(external_links, source_path, lines, references)
        }
        BlockKind::Quote { .. }
        | BlockKind::Code { .. }
        | BlockKind::Container { .. }
        | BlockKind::ReferenceDefinition { .. } => {}
    }
}

fn collect_maki_lines_external_links(
    external_links: &mut BTreeSet<ExternalLinkRef>,
    source_path: &Path,
    lines: &[&str],
    references: &parser::ReferenceDefinitions<'_>,
) {
    let source = lines.join("\n");
    let parsed = parser::parse_with_references(&source, references);

    collect_document_external_links(external_links, source_path, &parsed.document);
}

fn collect_document_external_links(
    external_links: &mut BTreeSet<ExternalLinkRef>,
    source_path: &Path,
    document: &parser::Document<'_>,
) {
    for definition in document.reference_definitions().iter() {
        match definition {
            parser::ReferenceDefinition::Link { target, .. }
                if is_checkable_external_href(target) =>
            {
                external_links.insert(ExternalLinkRef {
                    source_path: source_path.to_path_buf(),
                    target: target.trim().to_string(),
                });
            }
            parser::ReferenceDefinition::Footnote { body, .. } => {
                collect_inline_external_links(external_links, source_path, body)
            }
            parser::ReferenceDefinition::Link { .. } => {}
        }
    }

    for block in &document.blocks {
        collect_block_external_links(
            external_links,
            source_path,
            block,
            document.reference_definitions(),
        );
    }
}

pub(super) fn collect_external_links(sources: &BTreeMap<PathBuf, String>) -> Vec<ExternalLinkRef> {
    let mut external_links = BTreeSet::new();

    for (source_path, source) in sources {
        let parsed = parser::parse(source);

        collect_document_external_links(&mut external_links, source_path, &parsed.document);
    }

    external_links.into_iter().collect()
}
