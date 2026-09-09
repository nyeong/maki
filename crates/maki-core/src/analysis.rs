use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use crate::link_target::{DocumentSelector, InnerSelector, NoteLinkTarget, http_url_display_title};
use crate::maki::{DateIndex, NoteRef, collect_parsed_document_dates};
use crate::nested::{MappedSource, NestedDocumentObserver, traverse_nested_documents};
use crate::parser::{
    self, Block, BlockKind, Date, DateMonth, DateRange, DateStamp, DateStampKind, DateStampTarget,
    Inline, IsoWeek,
};
use crate::source::{SourceMap, SourceSpan, slice_span};

#[derive(Debug, Clone, Copy)]
pub struct SourceSnapshot<'a> {
    pub path: &'a Path,
    pub source: &'a str,
}

static NEXT_SNAPSHOT_REVISION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnapshotRevision(u64);

impl SnapshotRevision {
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSnapshot {
    revision: SnapshotRevision,
    sources: BTreeMap<PathBuf, Arc<str>>,
    analysis: ProjectAnalysis,
    title_origins: BTreeMap<PathBuf, DocumentTitleOrigin>,
}

impl ProjectSnapshot {
    pub fn compile(sources: BTreeMap<PathBuf, String>) -> Self {
        Self::compile_shared(
            sources
                .into_iter()
                .map(|(path, source)| (path, Arc::<str>::from(source)))
                .collect(),
        )
    }

    fn compile_shared(sources: BTreeMap<PathBuf, Arc<str>>) -> Self {
        let snapshots = sources
            .iter()
            .map(|(path, source)| SourceSnapshot {
                path: path.as_path(),
                source: source.as_ref(),
            })
            .collect::<Vec<_>>();
        let (analysis, title_origins) = analyze_project_with_title_origins(&snapshots);

        Self {
            revision: SnapshotRevision(NEXT_SNAPSHOT_REVISION.fetch_add(1, Ordering::Relaxed)),
            sources,
            analysis,
            title_origins,
        }
    }

    pub fn revision(&self) -> SnapshotRevision {
        self.revision
    }

    pub fn source(&self, path: &Path) -> Option<&str> {
        self.sources.get(path).map(Arc::as_ref)
    }

    pub fn source_paths(&self) -> impl Iterator<Item = &Path> {
        self.sources.keys().map(PathBuf::as_path)
    }

    pub fn with_source(&self, path: PathBuf, source: Option<String>) -> Self {
        let mut sources = self.sources.clone();
        match source {
            Some(source) => {
                sources.insert(path, Arc::<str>::from(source));
            }
            None => {
                sources.remove(&path);
            }
        }
        Self::compile_shared(sources)
    }

    pub fn analysis(&self) -> &ProjectAnalysis {
        &self.analysis
    }

    pub(crate) fn title_origin(&self, path: &Path) -> DocumentTitleOrigin {
        self.title_origins
            .get(path)
            .copied()
            .unwrap_or(DocumentTitleOrigin::FileStem)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProjectExternalLink {
    pub path: PathBuf,
    pub target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentTitleOrigin {
    Authored,
    FileStem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentAnalysis {
    pub path: PathBuf,
    pub canonical_path: String,
    pub title: String,
    pub document_span: SourceSpan,
    pub blocks: Vec<BlockOccurrence>,
    pub block_ids: Vec<BlockIdOccurrence>,
    pub headings: Vec<HeadingOccurrence>,
    pub note_links: Vec<NoteLinkOccurrence>,
    pub reference_graph: DocumentReferenceGraph,
    pub reference_links: Vec<ReferenceLinkOccurrence>,
    pub url_links: Vec<UrlLinkOccurrence>,
    pub external_links: Vec<String>,
    pub properties: Vec<PropertyOccurrence>,
    pub date_markers: Vec<DateMarkerOccurrence>,
    pub dates: Vec<DateOccurrence>,
    pub diagnostics: Vec<AnalysisDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectAnalysis {
    documents: BTreeMap<PathBuf, DocumentAnalysis>,
    document_index: DocumentIndex,
    pub diagnostics: Vec<AnalysisDiagnostic>,
    date_marker_index: BTreeMap<DateTargetIdentity, Vec<DateMarkerLocation>>,
    date_index: DateIndex,
    external_links: Vec<ProjectExternalLink>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentSelection<'a> {
    Found(&'a DocumentAnalysis),
    Broken,
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentDescendant<'a> {
    pub document: &'a DocumentAnalysis,
    pub relative_coordinate: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisBlockKind {
    Paragraph,
    Code,
    Heading,
    List,
    Quote,
    Table,
    Container,
    ReferenceDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockOccurrence {
    pub kind: AnalysisBlockKind,
    pub span: SourceSpan,
    pub body_spans: Vec<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockIdOccurrence {
    pub id: String,
    pub owner_kind: AnalysisBlockKind,
    pub owner_span: SourceSpan,
    pub declaration_span: SourceSpan,
    pub value_span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadingOccurrence {
    pub level: usize,
    pub title: String,
    pub anchor: String,
    pub span: SourceSpan,
    pub marker_span: SourceSpan,
    pub title_span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteLinkOccurrence {
    pub target: String,
    pub title: Option<String>,
    pub span: SourceSpan,
    pub title_span: Option<SourceSpan>,
    pub target_span: SourceSpan,
    pub resolution: Option<LinkResolution>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlLinkOccurrence {
    pub target: String,
    pub title: Option<String>,
    pub span: SourceSpan,
    pub title_span: Option<SourceSpan>,
    pub target_span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceLinkOccurrence {
    pub title: String,
    pub target: String,
    pub span: SourceSpan,
    pub title_span: SourceSpan,
    pub target_span: SourceSpan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReferenceDefinitionId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferencePresentation {
    Link,
    Footnote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceDefinitionState {
    Active,
    Duplicate { winner: ReferenceDefinitionId },
}

impl ReferenceDefinitionState {
    pub fn winner(self, definition: ReferenceDefinitionId) -> ReferenceDefinitionId {
        match self {
            Self::Active => definition,
            Self::Duplicate { winner } => winner,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceDefinitionOccurrence {
    pub id: ReferenceDefinitionId,
    pub key: String,
    pub value: String,
    pub value_kind: parser::ReferenceValueKind,
    pub semantic_target: Option<String>,
    pub semantic_target_span: Option<SourceSpan>,
    pub definition_span: SourceSpan,
    pub key_span: SourceSpan,
    pub value_span: SourceSpan,
    pub state: ReferenceDefinitionState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceUseOccurrence {
    pub key: String,
    pub scope: ReferenceScope,
    pub title: Option<String>,
    pub presentation: ReferencePresentation,
    pub span: SourceSpan,
    pub marker_span: SourceSpan,
    pub title_span: Option<SourceSpan>,
    pub key_span: SourceSpan,
    pub definition_id: Option<ReferenceDefinitionId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceScope {
    Document,
    Nested,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocumentReferenceGraph {
    pub definitions: Vec<ReferenceDefinitionOccurrence>,
    pub uses: Vec<ReferenceUseOccurrence>,
    winners: BTreeMap<String, ReferenceDefinitionId>,
    uses_by_definition: BTreeMap<ReferenceDefinitionId, Vec<usize>>,
}

impl DocumentReferenceGraph {
    pub fn definition(&self, id: ReferenceDefinitionId) -> Option<&ReferenceDefinitionOccurrence> {
        self.definitions.get(id.0)
    }

    pub fn winner_id(&self, key: &str) -> Option<ReferenceDefinitionId> {
        self.winners.get(key).copied()
    }

    pub fn winner(&self, key: &str) -> Option<&ReferenceDefinitionOccurrence> {
        self.winner_id(key).and_then(|id| self.definition(id))
    }

    pub fn uses_for(
        &self,
        definition: ReferenceDefinitionId,
    ) -> impl Iterator<Item = &ReferenceUseOccurrence> {
        self.uses_by_definition
            .get(&definition)
            .into_iter()
            .flatten()
            .filter_map(|index| self.uses.get(*index))
    }
}

#[derive(Default)]
struct ReferenceGraphBuilder {
    definitions: Vec<ReferenceDefinitionOccurrence>,
    uses: Vec<ReferenceUseOccurrence>,
    winners: BTreeMap<String, ReferenceDefinitionId>,
}

struct PendingReferenceDefinition<'a> {
    key: &'a str,
    value: &'a str,
    value_kind: parser::ReferenceValueKind,
    semantic_target: Option<&'a str>,
    semantic_target_span: Option<SourceSpan>,
    definition_span: SourceSpan,
    key_span: SourceSpan,
    value_span: SourceSpan,
}

struct PendingReferenceUse<'a> {
    key: &'a str,
    title: Option<&'a str>,
    presentation: ReferencePresentation,
    span: SourceSpan,
    marker_span: SourceSpan,
    title_span: Option<SourceSpan>,
    key_span: SourceSpan,
}

impl ReferenceGraphBuilder {
    fn winner(&self, key: &str) -> Option<&ReferenceDefinitionOccurrence> {
        self.winners
            .get(key)
            .and_then(|id| self.definitions.get(id.0))
    }

    fn push_definition(&mut self, definition: PendingReferenceDefinition<'_>) {
        let PendingReferenceDefinition {
            key,
            value,
            value_kind,
            semantic_target,
            semantic_target_span,
            definition_span,
            key_span,
            value_span,
        } = definition;
        let id = ReferenceDefinitionId(self.definitions.len());
        let state = match self.winners.get(key).copied() {
            Some(winner) => ReferenceDefinitionState::Duplicate { winner },
            None => {
                self.winners.insert(key.to_string(), id);
                ReferenceDefinitionState::Active
            }
        };
        self.definitions.push(ReferenceDefinitionOccurrence {
            id,
            key: key.to_string(),
            value: value.to_string(),
            value_kind,
            semantic_target: semantic_target.map(str::to_owned),
            semantic_target_span,
            definition_span,
            key_span,
            value_span,
            state,
        });
    }

    fn push_use(&mut self, usage: PendingReferenceUse<'_>) {
        let PendingReferenceUse {
            key,
            title,
            presentation,
            span,
            marker_span,
            title_span,
            key_span,
        } = usage;
        let definition_id = self.winners.get(key).copied();
        self.uses.push(ReferenceUseOccurrence {
            key: key.to_string(),
            scope: ReferenceScope::Document,
            title: title.map(str::to_owned),
            presentation,
            span,
            marker_span,
            title_span,
            key_span,
            definition_id,
        });
    }

    fn finish(mut self) -> DocumentReferenceGraph {
        self.uses.sort_by_key(|usage| usage.span);
        let mut uses_by_definition: BTreeMap<ReferenceDefinitionId, Vec<usize>> = BTreeMap::new();
        for (index, usage) in self.uses.iter().enumerate() {
            if let Some(definition_id) = usage.definition_id {
                uses_by_definition
                    .entry(definition_id)
                    .or_default()
                    .push(index);
            }
        }
        DocumentReferenceGraph {
            definitions: self.definitions,
            uses: self.uses,
            winners: self.winners,
            uses_by_definition,
        }
    }
}

fn map_reference_graph(
    mut graph: DocumentReferenceGraph,
    coordinates: &MappedSource,
) -> Option<DocumentReferenceGraph> {
    for definition in &mut graph.definitions {
        definition.semantic_target_span = definition
            .semantic_target_span
            .and_then(|span| coordinates.map_span(span));
        definition.definition_span = coordinates.map_span(definition.definition_span)?;
        definition.key_span = coordinates.map_span(definition.key_span)?;
        definition.value_span = coordinates.map_span(definition.value_span)?;
    }
    for usage in &mut graph.uses {
        usage.scope = ReferenceScope::Nested;
        usage.span = coordinates.map_span(usage.span)?;
        usage.marker_span = coordinates.map_span(usage.marker_span)?;
        usage.title_span = usage.title_span.and_then(|span| coordinates.map_span(span));
        usage.key_span = coordinates.map_span(usage.key_span)?;
    }
    Some(graph)
}

#[derive(Default)]
struct DocumentOccurrences {
    blocks: Vec<BlockOccurrence>,
    block_ids: Vec<BlockIdOccurrence>,
    headings: Vec<HeadingOccurrence>,
    note_links: Vec<NoteLinkOccurrence>,
    references: ReferenceGraphBuilder,
    reference_links: Vec<ReferenceLinkOccurrence>,
    url_links: Vec<UrlLinkOccurrence>,
    external_links: BTreeSet<String>,
    properties: Vec<PropertyOccurrence>,
    date_markers: Vec<DateMarkerOccurrence>,
    dates: Vec<DateOccurrence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyDirection {
    Previous,
    Next,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyOccurrence {
    pub direction: PropertyDirection,
    pub owner: PropertyOwner,
    pub key: String,
    pub value: String,
    pub span: SourceSpan,
    pub key_span: SourceSpan,
    pub value_span: SourceSpan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyOwner {
    Document,
    Block {
        kind: AnalysisBlockKind,
        span: SourceSpan,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateOrigin {
    VisibleInline,
    PropertyValue { key: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateOccurrence {
    pub kind: DateStampKind,
    pub target: DateTargetIdentity,
    pub body: String,
    pub origin: DateOrigin,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DateTargetIdentity {
    Day(Date),
    Month(DateMonth),
    IsoWeek(IsoWeek),
    Range { start: Date, end: Date },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateMarkerOrigin {
    Inline,
    PropertyValue { key: String },
    ReferenceDefinitionValue { key: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateMarkerOccurrence {
    pub kind: DateStampKind,
    pub target: DateTargetIdentity,
    pub origin: DateMarkerOrigin,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateMarkerLocation {
    pub path: PathBuf,
    pub kind: DateStampKind,
    pub origin: DateMarkerOrigin,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisDiagnostic {
    pub path: PathBuf,
    pub span: SourceSpan,
    pub kind: AnalysisDiagnosticKind,
    pub subject: AnalysisDiagnosticSubject,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalysisDiagnosticSubject {
    None,
    Id(String),
    Reference(String),
    Link(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisDiagnosticKind {
    ParseWarning,
    DuplicateId,
    UnresolvedReference,
    BrokenNoteLink,
    AmbiguousNoteLink,
    BrokenHeadingLink,
    AmbiguousHeadingLink,
    BrokenIdLink,
    AmbiguousIdLink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkResolution {
    Found(DefinitionTarget),
    BrokenNote,
    AmbiguousNote,
    BrokenHeading,
    AmbiguousHeading,
    BrokenId,
    AmbiguousId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefinitionTargetKind {
    Document,
    Heading,
    Id,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionTarget {
    pub path: PathBuf,
    pub canonical_path: String,
    pub selection_span: SourceSpan,
    pub kind: DefinitionTargetKind,
    pub fragment: Option<String>,
}

pub fn analyze_document(path: &Path, source: &str) -> DocumentAnalysis {
    let parsed = parser::parse(source);
    let (mut document, _) = analyze_parsed_document_with_title_origin(path, source, &parsed);
    enrich_document_with_nested_analysis(&mut document, |_, observer| {
        traverse_nested_documents(source, &parsed, observer);
    });
    document
}

#[cfg(test)]
fn analyze_document_with_title_origin(
    path: &Path,
    source: &str,
) -> (DocumentAnalysis, DocumentTitleOrigin) {
    let parsed = parser::parse(source);
    let (mut document, title_origin) =
        analyze_parsed_document_with_title_origin(path, source, &parsed);
    enrich_document_with_nested_analysis(&mut document, |_, observer| {
        traverse_nested_documents(source, &parsed, observer);
    });
    (document, title_origin)
}

fn analyze_parsed_document_with_title_origin(
    path: &Path,
    source: &str,
    parsed: &parser::ParseResult<'_>,
) -> (DocumentAnalysis, DocumentTitleOrigin) {
    let (title, title_origin, document_span) = match parsed.document.title() {
        Some(title) => (
            title.to_owned(),
            DocumentTitleOrigin::Authored,
            slice_span(source, title).unwrap_or_default(),
        ),
        None => (
            file_stem(path),
            DocumentTitleOrigin::FileStem,
            SourceSpan::default(),
        ),
    };
    let mut occurrences = collect_document_occurrences(source, &parsed.document);
    let reference_graph = std::mem::take(&mut occurrences.references).finish();
    occurrences.blocks.sort_by_key(|block| block.span);
    occurrences
        .block_ids
        .sort_by_key(|block_id| block_id.value_span);
    occurrences
        .reference_links
        .sort_by_key(|reference| reference.span);
    occurrences.url_links.sort_by_key(|link| link.span);
    occurrences.note_links.sort_by_key(|link| link.span);
    occurrences.properties.sort_by_key(|property| property.span);
    occurrences.date_markers.sort_by_key(|marker| marker.span);
    let mut diagnostics = parsed
        .diagnostics
        .iter()
        .map(|diagnostic| AnalysisDiagnostic {
            path: path.to_path_buf(),
            span: diagnostic.span,
            kind: AnalysisDiagnosticKind::ParseWarning,
            subject: AnalysisDiagnosticSubject::None,
            message: parser::format_parse_diagnostic_kind(&diagnostic.kind),
        })
        .collect::<Vec<_>>();
    diagnostics.extend(duplicate_id_diagnostics(
        path,
        &occurrences.block_ids,
        &occurrences.headings,
    ));
    diagnostics.extend(
        reference_graph
            .uses
            .iter()
            .filter(|usage| {
                usage.scope == ReferenceScope::Document && usage.definition_id.is_none()
            })
            .map(|usage| AnalysisDiagnostic {
                path: path.to_path_buf(),
                span: usage.key_span,
                kind: AnalysisDiagnosticKind::UnresolvedReference,
                subject: AnalysisDiagnosticSubject::Reference(usage.key.clone()),
                message: format!("unresolved reference: {}", usage.key),
            }),
    );
    diagnostics.sort_by_key(|diagnostic| diagnostic.span);

    (
        DocumentAnalysis {
            path: path.to_path_buf(),
            canonical_path: canonical_path(path),
            title,
            document_span,
            blocks: occurrences.blocks,
            block_ids: occurrences.block_ids,
            headings: occurrences.headings,
            note_links: occurrences.note_links,
            reference_graph,
            reference_links: occurrences.reference_links,
            url_links: occurrences.url_links,
            external_links: occurrences.external_links.into_iter().collect(),
            properties: occurrences.properties,
            date_markers: occurrences.date_markers,
            dates: occurrences.dates,
            diagnostics,
        },
        title_origin,
    )
}

pub fn analyze_project(snapshots: &[SourceSnapshot<'_>]) -> ProjectAnalysis {
    analyze_project_with_title_origins(snapshots).0
}

pub(crate) fn analyze_project_with_title_origins(
    snapshots: &[SourceSnapshot<'_>],
) -> (ProjectAnalysis, BTreeMap<PathBuf, DocumentTitleOrigin>) {
    let mut documents = BTreeMap::new();
    let mut title_origins = BTreeMap::new();
    let mut date_index = DateIndex::default();
    let mut external_links = BTreeSet::new();
    for snapshot in snapshots {
        let parsed = parser::parse(snapshot.source);
        let (mut document, title_origin) =
            analyze_parsed_document_with_title_origin(snapshot.path, snapshot.source, &parsed);
        enrich_document_with_nested_analysis(&mut document, |document, observer| {
            collect_parsed_document_dates(
                &mut date_index,
                &document.path,
                NoteRef::new(&document.canonical_path),
                &document.title,
                snapshot.source,
                &parsed,
                observer,
            );
        });
        external_links.extend(
            document
                .external_links
                .iter()
                .map(|target| ProjectExternalLink {
                    path: document.path.clone(),
                    target: target.clone(),
                }),
        );
        title_origins.insert(document.path.clone(), title_origin);
        documents.insert(document.path.clone(), document);
    }
    date_index.sort_backlinks();
    let document_index = DocumentIndex::new(&documents);
    let mut diagnostics = documents
        .values()
        .flat_map(|document| document.diagnostics.iter().cloned())
        .collect::<Vec<_>>();
    let date_marker_index = build_date_marker_index(&documents);
    let mut analysis = ProjectAnalysis {
        documents,
        document_index,
        diagnostics: Vec::new(),
        date_marker_index,
        date_index,
        external_links: external_links.into_iter().collect(),
    };
    let resolutions = analysis
        .documents
        .values()
        .flat_map(|document| {
            document
                .note_links
                .iter()
                .enumerate()
                .map(|(index, occurrence)| {
                    (
                        document.path.clone(),
                        index,
                        occurrence.target_span,
                        occurrence.target.clone(),
                        analysis.resolve_note_link(&document.path, &occurrence.target),
                    )
                })
        })
        .collect::<Vec<_>>();

    for (path, index, target_span, target, resolution) in resolutions {
        if let Some((kind, message)) = diagnostic_for_resolution(&target, &resolution) {
            diagnostics.push(AnalysisDiagnostic {
                path: path.clone(),
                span: target_span,
                kind,
                subject: AnalysisDiagnosticSubject::Link(target.clone()),
                message,
            });
        }
        if let Some(occurrence) = analysis
            .documents
            .get_mut(&path)
            .and_then(|document| document.note_links.get_mut(index))
        {
            occurrence.resolution = Some(resolution);
        }
    }

    diagnostics.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.span.cmp(&right.span))
    });
    analysis.diagnostics = diagnostics;

    (analysis, title_origins)
}

fn build_date_marker_index(
    documents: &BTreeMap<PathBuf, DocumentAnalysis>,
) -> BTreeMap<DateTargetIdentity, Vec<DateMarkerLocation>> {
    let mut index: BTreeMap<DateTargetIdentity, Vec<DateMarkerLocation>> = BTreeMap::new();
    for document in documents.values() {
        for marker in &document.date_markers {
            index
                .entry(marker.target)
                .or_default()
                .push(DateMarkerLocation {
                    path: document.path.clone(),
                    kind: marker.kind,
                    origin: marker.origin.clone(),
                    span: marker.span,
                });
        }
    }
    for locations in index.values_mut() {
        locations.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.span.cmp(&right.span))
        });
        locations.dedup_by(|left, right| {
            left.path == right.path && left.span == right.span && left.kind == right.kind
        });
    }
    index
}

impl ProjectAnalysis {
    pub fn documents(&self) -> &BTreeMap<PathBuf, DocumentAnalysis> {
        &self.documents
    }

    pub fn document(&self, path: &Path) -> Option<&DocumentAnalysis> {
        self.documents.get(path)
    }

    pub fn note_candidates(&self) -> impl Iterator<Item = &DocumentAnalysis> {
        self.documents.values()
    }

    pub fn select_document(
        &self,
        current_path: &Path,
        selector: DocumentSelector<'_>,
    ) -> DocumentSelection<'_> {
        match self.document_index.select(current_path, selector) {
            PathSelection::Found(path) => self
                .documents
                .get(&path)
                .map_or(DocumentSelection::Broken, DocumentSelection::Found),
            PathSelection::Broken => DocumentSelection::Broken,
            PathSelection::Ambiguous => DocumentSelection::Ambiguous,
        }
    }

    pub fn resolve_note_link(&self, current_path: &Path, target: &str) -> LinkResolution {
        let target = NoteLinkTarget::parse(target);
        let document = match self.select_document(current_path, target.document) {
            DocumentSelection::Found(document) => document,
            DocumentSelection::Broken => return LinkResolution::BrokenNote,
            DocumentSelection::Ambiguous => return LinkResolution::AmbiguousNote,
        };

        match target.inner {
            None => LinkResolution::Found(DefinitionTarget {
                path: document.path.clone(),
                canonical_path: document.canonical_path.clone(),
                selection_span: document.document_span,
                kind: DefinitionTargetKind::Document,
                fragment: None,
            }),
            Some(InnerSelector::Heading(heading)) => resolve_heading(document, heading),
            Some(InnerSelector::Id(id)) => resolve_id(document, id),
        }
    }

    pub fn descendant_documents(
        &self,
        current_path: &Path,
    ) -> impl Iterator<Item = DocumentDescendant<'_>> {
        let prefix = self
            .document(current_path)
            .map(|document| format!("{}/", document.canonical_path));

        self.documents.values().filter_map(move |document| {
            let relative_coordinate = document.canonical_path.strip_prefix(prefix.as_deref()?)?;
            Some(DocumentDescendant {
                document,
                relative_coordinate,
            })
        })
    }

    pub fn date_marker_locations(&self, target: &DateTargetIdentity) -> &[DateMarkerLocation] {
        self.date_marker_index
            .get(target)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn date_index(&self) -> &DateIndex {
        &self.date_index
    }

    pub fn external_links(&self) -> &[ProjectExternalLink] {
        &self.external_links
    }

    pub fn property_keys(&self) -> Vec<String> {
        let mut keys = conventional_property_keys()
            .iter()
            .map(|key| (*key).to_string())
            .collect::<Vec<_>>();
        keys.extend(
            self.documents
                .values()
                .flat_map(|document| document.properties.iter())
                .map(|property| property.key.to_lowercase()),
        );
        keys.sort();
        keys.dedup();
        keys
    }
}

pub fn conventional_property_keys() -> &'static [&'static str] {
    &[
        "created", "date", "deadline", "id", "lang", "mode", "status", "title", "updated",
    ]
}

pub fn property_description(key: &str) -> Option<&'static str> {
    match key.to_ascii_lowercase().as_str() {
        "title" => Some("Document title."),
        "id" => Some("Case-sensitive, document-local explicit block identifier."),
        "lang" => Some("Code language."),
        "mode" => Some("Quote parsing mode: block, pre, or text."),
        "created" | "date" | "deadline" | "updated" => Some("Date metadata."),
        "status" => Some("Conventional workflow status."),
        _ => None,
    }
}

fn collect_document_occurrences(
    source: &str,
    document: &parser::Document<'_>,
) -> DocumentOccurrences {
    let source_map = SourceMap::new(source);
    let mut occurrences = DocumentOccurrences::default();
    collect_attached_properties(
        source,
        &source_map,
        document.property_declarations(),
        PropertyOwner::Document,
        &mut occurrences.properties,
    );
    collect_reference_definitions(
        source,
        &source_map,
        &document.blocks,
        &mut occurrences.references,
    );
    collect_reference_definition_date_markers(
        source,
        &document.blocks,
        &mut occurrences.date_markers,
    );
    for block in &document.blocks {
        collect_block(source, &source_map, block, &mut occurrences);
    }
    for definition in document.reference_definitions().iter() {
        if matches!(
            definition.value_kind(),
            parser::ReferenceValueKind::Prose
                | parser::ReferenceValueKind::NoteLink
                | parser::ReferenceValueKind::HyperLink
        ) {
            collect_inlines(
                source,
                &definition.value,
                DateOrigin::VisibleInline,
                InlineCollectionScope::ReferenceValue,
                &mut occurrences,
            );
        }
    }
    for property in &occurrences.properties {
        let value_source = &source[property.value_span.start..property.value_span.end];
        let parsed_value = parser::parse_inline(value_source);
        collect_property_dates(
            source,
            &parsed_value,
            DateOrigin::PropertyValue {
                key: property.key.clone(),
            },
            &mut occurrences.dates,
        );
        collect_date_markers(
            source,
            &parsed_value,
            DateMarkerOrigin::PropertyValue {
                key: property.key.clone(),
            },
            &mut occurrences.date_markers,
        );
    }
    occurrences
}

struct NestedCollection {
    occurrences: DocumentOccurrences,
    diagnostics: Vec<AnalysisDiagnostic>,
    reference_graph: DocumentReferenceGraph,
}

struct NestedOccurrenceCollector<'a> {
    path: &'a Path,
    occurrences: DocumentOccurrences,
    diagnostics: Vec<AnalysisDiagnostic>,
    reference_graph: DocumentReferenceGraph,
    reference_scopes: Vec<BTreeMap<String, ReferenceDefinitionId>>,
}

impl<'a> NestedOccurrenceCollector<'a> {
    fn new(path: &'a Path, reference_graph: DocumentReferenceGraph) -> Self {
        let root_scope = reference_graph.winners.clone();
        Self {
            path,
            occurrences: DocumentOccurrences::default(),
            diagnostics: Vec::new(),
            reference_graph,
            reference_scopes: vec![root_scope],
        }
    }

    fn finish(mut self) -> NestedCollection {
        debug_assert_eq!(self.reference_scopes.len(), 1);
        self.reference_graph.uses.sort_by_key(|usage| usage.span);
        self.reference_graph.uses_by_definition.clear();
        for (index, usage) in self.reference_graph.uses.iter().enumerate() {
            if let Some(definition_id) = usage.definition_id {
                self.reference_graph
                    .uses_by_definition
                    .entry(definition_id)
                    .or_default()
                    .push(index);
            }
        }

        NestedCollection {
            occurrences: self.occurrences,
            diagnostics: self.diagnostics,
            reference_graph: self.reference_graph,
        }
    }

    fn inherited_definition_id(&self, key: &str) -> Option<ReferenceDefinitionId> {
        self.reference_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(key).copied())
    }

    fn enter_reference_scope(&mut self, mut graph: DocumentReferenceGraph) {
        let offset = self.reference_graph.definitions.len();
        let local_scope = graph
            .winners
            .iter()
            .map(|(key, id)| (key.clone(), ReferenceDefinitionId(id.0 + offset)))
            .collect::<BTreeMap<_, _>>();

        for definition in &mut graph.definitions {
            definition.id.0 += offset;
            if let ReferenceDefinitionState::Duplicate { winner } = &mut definition.state {
                winner.0 += offset;
            }
        }

        let mut inherited_semantics = Vec::new();
        for usage in &mut graph.uses {
            if let Some(definition_id) = &mut usage.definition_id {
                definition_id.0 += offset;
                continue;
            }

            let Some(definition_id) = self.inherited_definition_id(&usage.key) else {
                continue;
            };
            usage.definition_id = Some(definition_id);
            if let Some(definition) = self.reference_graph.definition(definition_id) {
                inherited_semantics.push((usage.clone(), definition.clone()));
            }
        }

        self.reference_graph
            .definitions
            .append(&mut graph.definitions);
        self.reference_graph.uses.append(&mut graph.uses);
        for (usage, definition) in inherited_semantics {
            collect_resolved_reference_semantics(&usage, &definition, &mut self.occurrences);
        }
        self.reference_scopes.push(local_scope);
    }
}

impl NestedDocumentObserver for NestedOccurrenceCollector<'_> {
    fn enter(&mut self, mapped: &MappedSource, parsed: &parser::ParseResult<'_>) {
        let mut nested = collect_document_occurrences(&mapped.text, &parsed.document);
        self.diagnostics
            .extend(parsed.diagnostics.iter().filter_map(|diagnostic| {
                Some(AnalysisDiagnostic {
                    path: self.path.to_path_buf(),
                    span: mapped.map_span(diagnostic.span)?,
                    kind: AnalysisDiagnosticKind::ParseWarning,
                    subject: AnalysisDiagnosticSubject::None,
                    message: parser::format_parse_diagnostic_kind(&diagnostic.kind),
                })
            }));

        let nested_reference_graph = std::mem::take(&mut nested.references).finish();
        self.diagnostics.extend(
            nested_reference_graph
                .uses
                .iter()
                .filter(|usage| parsed.document.reference(&usage.key).is_none())
                .filter_map(|usage| {
                    Some(AnalysisDiagnostic {
                        path: self.path.to_path_buf(),
                        span: mapped.map_span(usage.key_span)?,
                        kind: AnalysisDiagnosticKind::UnresolvedReference,
                        subject: AnalysisDiagnosticSubject::Reference(usage.key.clone()),
                        message: format!("unresolved reference: {}", usage.key),
                    })
                }),
        );

        match map_reference_graph(nested_reference_graph, mapped) {
            Some(reference_graph) => self.enter_reference_scope(reference_graph),
            None => self.reference_scopes.push(BTreeMap::new()),
        }
        merge_mapped_occurrences(&mut self.occurrences, nested, mapped);
    }

    fn exit(&mut self) {
        debug_assert!(self.reference_scopes.len() > 1);
        self.reference_scopes.pop();
    }
}

fn collect_resolved_reference_semantics(
    usage: &ReferenceUseOccurrence,
    definition: &ReferenceDefinitionOccurrence,
    occurrences: &mut DocumentOccurrences,
) {
    if usage.presentation != ReferencePresentation::Link {
        return;
    }

    if matches!(
        definition.value_kind,
        parser::ReferenceValueKind::HyperLink | parser::ReferenceValueKind::NoteLink
    ) && let (Some(target), Some(target_span)) = (
        definition.semantic_target.as_ref(),
        definition.semantic_target_span,
    ) {
        occurrences.reference_links.push(ReferenceLinkOccurrence {
            title: usage.title.clone().unwrap_or_else(|| usage.key.clone()),
            target: target.clone(),
            span: usage.span,
            title_span: usage.title_span.unwrap_or(usage.key_span),
            target_span,
        });
    }

    let Some(target) = definition.semantic_target.as_deref() else {
        return;
    };
    let parsed_target = parser::parse_inline(target);
    match (definition.value_kind, parsed_target.as_slice()) {
        (parser::ReferenceValueKind::DateStamp, [Inline::DateStamp(stamp)]) => {
            push_date_occurrence(
                *stamp,
                DateOrigin::VisibleInline,
                usage.span,
                &mut occurrences.dates,
            );
        }
        (parser::ReferenceValueKind::DateRange, [Inline::DateRange(range)])
            if usage.title_span.is_none() =>
        {
            push_date_range_occurrence(
                *range,
                DateOrigin::VisibleInline,
                usage.span,
                &mut occurrences.dates,
            );
        }
        _ => {}
    }
}

fn merge_mapped_occurrences(
    target: &mut DocumentOccurrences,
    mut nested: DocumentOccurrences,
    coordinates: &MappedSource,
) {
    target
        .blocks
        .extend(nested.blocks.drain(..).filter_map(|mut block| {
            block.span = coordinates.map_span(block.span)?;
            block.body_spans = block
                .body_spans
                .into_iter()
                .filter_map(|span| coordinates.map_span(span))
                .collect();
            Some(block)
        }));
    target
        .block_ids
        .extend(nested.block_ids.drain(..).filter_map(|mut block_id| {
            block_id.owner_span = coordinates.map_span(block_id.owner_span)?;
            block_id.declaration_span = coordinates.map_span(block_id.declaration_span)?;
            block_id.value_span = coordinates.map_span(block_id.value_span)?;
            Some(block_id)
        }));
    target
        .headings
        .extend(nested.headings.drain(..).filter_map(|mut heading| {
            heading.span = coordinates.map_span(heading.span)?;
            heading.marker_span = coordinates.map_span(heading.marker_span)?;
            heading.title_span = coordinates.map_span(heading.title_span)?;
            Some(heading)
        }));
    target
        .note_links
        .extend(nested.note_links.drain(..).filter_map(|mut link| {
            link.span = coordinates.map_span(link.span)?;
            link.title_span = link.title_span.and_then(|span| coordinates.map_span(span));
            link.target_span = coordinates.map_span(link.target_span)?;
            Some(link)
        }));
    target.reference_links.extend(
        nested
            .reference_links
            .drain(..)
            .filter_map(|mut reference| {
                reference.span = coordinates.map_span(reference.span)?;
                reference.title_span = coordinates.map_span(reference.title_span)?;
                reference.target_span = coordinates.map_span(reference.target_span)?;
                Some(reference)
            }),
    );
    target
        .url_links
        .extend(nested.url_links.drain(..).filter_map(|mut link| {
            link.span = coordinates.map_span(link.span)?;
            link.title_span = link.title_span.and_then(|span| coordinates.map_span(span));
            link.target_span = coordinates.map_span(link.target_span)?;
            Some(link)
        }));
    target.external_links.append(&mut nested.external_links);
    target
        .properties
        .extend(nested.properties.drain(..).filter_map(|mut property| {
            property.span = coordinates.map_span(property.span)?;
            property.key_span = coordinates.map_span(property.key_span)?;
            property.value_span = coordinates.map_span(property.value_span)?;
            if let PropertyOwner::Block { kind, span } = property.owner {
                property.owner = PropertyOwner::Block {
                    kind,
                    span: coordinates.map_span(span)?,
                };
            }
            Some(property)
        }));
    target
        .date_markers
        .extend(nested.date_markers.drain(..).filter_map(|mut marker| {
            marker.span = coordinates.map_span(marker.span)?;
            Some(marker)
        }));
    target
        .dates
        .extend(nested.dates.drain(..).filter_map(|mut date| {
            date.span = coordinates.map_span(date.span)?;
            Some(date)
        }));
}

fn enrich_document_with_nested_analysis(
    document: &mut DocumentAnalysis,
    traverse_nested: impl FnOnce(&DocumentAnalysis, &mut dyn NestedDocumentObserver),
) {
    let root_reference_graph = std::mem::take(&mut document.reference_graph);
    let mut visitor = NestedOccurrenceCollector::new(&document.path, root_reference_graph);
    traverse_nested(document, &mut visitor);
    let collected = visitor.finish();
    document.reference_graph = collected.reference_graph;
    merge_nested_into_document(document, collected.occurrences, collected.diagnostics);
}

fn merge_nested_into_document(
    document: &mut DocumentAnalysis,
    mut nested: DocumentOccurrences,
    nested_diagnostics: Vec<AnalysisDiagnostic>,
) {
    document.blocks.append(&mut nested.blocks);
    document.block_ids.append(&mut nested.block_ids);
    document.headings.append(&mut nested.headings);
    document.note_links.append(&mut nested.note_links);
    document.reference_links.append(&mut nested.reference_links);
    document.url_links.append(&mut nested.url_links);
    document.properties.append(&mut nested.properties);
    document.date_markers.append(&mut nested.date_markers);
    document.dates.append(&mut nested.dates);

    let mut external_links = document.external_links.drain(..).collect::<BTreeSet<_>>();
    external_links.append(&mut nested.external_links);
    document.external_links = external_links.into_iter().collect();

    document.blocks.sort_by_key(|block| block.span);
    document
        .block_ids
        .sort_by_key(|block_id| block_id.value_span);
    document.headings.sort_by_key(|heading| heading.span);
    document.note_links.sort_by_key(|link| link.span);
    document
        .reference_links
        .sort_by_key(|reference| reference.span);
    document.url_links.sort_by_key(|link| link.span);
    document.properties.sort_by_key(|property| property.span);
    document.date_markers.sort_by_key(|marker| marker.span);
    document.dates.sort_by_key(|date| date.span);

    document
        .diagnostics
        .retain(|diagnostic| diagnostic.kind != AnalysisDiagnosticKind::DuplicateId);
    document.diagnostics.extend(nested_diagnostics);
    document.diagnostics.extend(duplicate_id_diagnostics(
        &document.path,
        &document.block_ids,
        &document.headings,
    ));
    document
        .diagnostics
        .sort_by_key(|diagnostic| diagnostic.span);
}

fn collect_reference_definitions(
    source: &str,
    source_map: &SourceMap<'_>,
    blocks: &[Block<'_>],
    graph: &mut ReferenceGraphBuilder,
) {
    for block in blocks {
        let BlockKind::ReferenceDefinition { definitions } = &block.kind else {
            continue;
        };
        for definition in definitions {
            let Some(key_span) = slice_span(source, definition.key) else {
                continue;
            };
            let definition_span = whole_line_span(source_map, key_span);
            let value_span = slice_span(source, definition.raw_value)
                .unwrap_or_else(|| SourceSpan::new(definition_span.end, definition_span.end));
            let value_kind = definition.value_kind();
            let semantic_target = match definition.value.as_slice() {
                [Inline::HyperLink { target, .. }] | [Inline::NoteLink { target, .. }] => {
                    Some(*target)
                }
                [Inline::DateStamp(_)] | [Inline::DateRange(_)] => Some(definition.raw_value),
                _ => None,
            };
            let semantic_target_span =
                semantic_target.and_then(|target| slice_span(source, target));
            graph.push_definition(PendingReferenceDefinition {
                key: definition.key,
                value: definition.raw_value,
                value_kind,
                semantic_target,
                semantic_target_span,
                definition_span,
                key_span,
                value_span,
            });
        }
    }
}

fn collect_reference_definition_date_markers(
    source: &str,
    blocks: &[Block<'_>],
    markers: &mut Vec<DateMarkerOccurrence>,
) {
    for block in blocks {
        let BlockKind::ReferenceDefinition { definitions } = &block.kind else {
            continue;
        };
        for definition in definitions {
            collect_date_markers(
                source,
                &definition.value,
                DateMarkerOrigin::ReferenceDefinitionValue {
                    key: definition.key.to_string(),
                },
                markers,
            );
        }
    }
}

fn collect_block(
    source: &str,
    source_map: &SourceMap<'_>,
    block: &Block<'_>,
    occurrences: &mut DocumentOccurrences,
) {
    let mut body_spans = Vec::new();
    let kind = match &block.kind {
        BlockKind::Paragraph { body } => {
            collect_visible_inlines(source, body, occurrences);
            collect_inline_source_spans(source, body, &mut body_spans);
            AnalysisBlockKind::Paragraph
        }
        BlockKind::Code { lines, .. } => {
            body_spans.extend(lines.iter().filter_map(|line| slice_span(source, line)));
            AnalysisBlockKind::Code
        }
        BlockKind::Heading {
            level,
            body,
            raw_body,
        } => {
            collect_visible_inlines(source, body, occurrences);
            if let Some(title_span) = slice_span(source, raw_body) {
                let span = whole_line_span(source_map, title_span);
                let marker_start = title_span.start.saturating_sub(level + 1);
                let marker_span = SourceSpan::new(marker_start, marker_start + level);
                let anchor = block
                    .property("id")
                    .filter(|id| !id.is_empty())
                    .unwrap_or(raw_body);
                occurrences.headings.push(HeadingOccurrence {
                    level: *level,
                    title: (*raw_body).to_string(),
                    anchor: anchor.to_string(),
                    span,
                    marker_span,
                    title_span,
                });
                body_spans.push(title_span);
            }
            AnalysisBlockKind::Heading
        }
        BlockKind::List { items } => {
            for item in items {
                collect_visible_inlines(source, &item.body, occurrences);
                collect_inline_source_spans(source, &item.body, &mut body_spans);
                for child in &item.children {
                    collect_block(source, source_map, child, occurrences);
                }
            }
            AnalysisBlockKind::List
        }
        BlockKind::Quote { lines } => {
            body_spans.extend(lines.iter().filter_map(|line| slice_span(source, line)));
            AnalysisBlockKind::Quote
        }
        BlockKind::Table { header, rows, .. } => {
            for row in std::iter::once(header).chain(rows) {
                for cell in &row.cells {
                    collect_visible_inlines(source, &cell.body, occurrences);
                    collect_inline_source_spans(source, &cell.body, &mut body_spans);
                }
            }
            AnalysisBlockKind::Table
        }
        BlockKind::Container { kind, lines, .. } => {
            body_spans.extend(slice_span(source, kind));
            body_spans.extend(lines.iter().filter_map(|line| slice_span(source, line)));
            AnalysisBlockKind::Container
        }
        BlockKind::ReferenceDefinition { definitions } => {
            for definition in definitions {
                body_spans.extend(slice_span(source, definition.key));
                body_spans.extend(slice_span(source, definition.raw_value));
            }
            AnalysisBlockKind::ReferenceDefinition
        }
    };

    let owner_span = covering_line_span(source_map, &body_spans);
    if let Some(span) = owner_span {
        occurrences.blocks.push(BlockOccurrence {
            kind,
            span,
            body_spans,
        });
    }

    let property_owner_span = owner_span.unwrap_or_default();
    collect_attached_properties(
        source,
        source_map,
        block.property_declarations(),
        PropertyOwner::Block {
            kind,
            span: property_owner_span,
        },
        &mut occurrences.properties,
    );

    if kind != AnalysisBlockKind::ReferenceDefinition
        && let Some(id) = block.property("id").filter(|id| !id.is_empty())
        && let Some(value_span) = slice_span(source, id)
    {
        let declaration_span = whole_line_span(source_map, value_span);
        occurrences.block_ids.push(BlockIdOccurrence {
            id: id.to_string(),
            owner_kind: kind,
            owner_span: owner_span.unwrap_or(declaration_span),
            declaration_span,
            value_span,
        });
    }
}

fn collect_visible_inlines(
    source: &str,
    inlines: &[Inline<'_>],
    occurrences: &mut DocumentOccurrences,
) {
    collect_inlines(
        source,
        inlines,
        DateOrigin::VisibleInline,
        InlineCollectionScope::Visible,
        occurrences,
    );
    collect_date_markers(
        source,
        inlines,
        DateMarkerOrigin::Inline,
        &mut occurrences.date_markers,
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InlineCollectionScope {
    Visible,
    ReferenceValue,
}

fn collect_inlines(
    source: &str,
    inlines: &[Inline<'_>],
    origin: DateOrigin,
    scope: InlineCollectionScope,
    occurrences: &mut DocumentOccurrences,
) {
    for inline in inlines {
        match inline {
            Inline::NoteLink { raw, title, target } => {
                if let (Some(span), Some(target_span)) =
                    (slice_span(source, raw), slice_span(source, target))
                {
                    let title_span = title.and_then(|title| slice_span(source, title));
                    occurrences.note_links.push(NoteLinkOccurrence {
                        target: (*target).to_string(),
                        title: title.map(ToString::to_string),
                        span,
                        title_span,
                        target_span,
                        resolution: None,
                    });
                }
            }
            Inline::Reference { raw, title, key } => {
                let (Some(span), Some(key_span)) =
                    (slice_span(source, raw), slice_span(source, key))
                else {
                    continue;
                };
                let default_title = reference_uses_default_title(raw);
                let title_span = if default_title {
                    None
                } else {
                    slice_span(source, title)
                };
                if !default_title && title_span.is_none() {
                    continue;
                }

                let semantic = occurrences.references.winner(key).map(|definition| {
                    (
                        definition.value_kind,
                        definition.semantic_target.clone(),
                        definition.semantic_target_span,
                    )
                });
                if let Some((value_kind, Some(target), Some(target_span))) = &semantic
                    && matches!(
                        value_kind,
                        parser::ReferenceValueKind::HyperLink
                            | parser::ReferenceValueKind::NoteLink
                    )
                {
                    occurrences.reference_links.push(ReferenceLinkOccurrence {
                        title: (*title).to_string(),
                        target: target.clone(),
                        span,
                        title_span: title_span.unwrap_or(key_span),
                        target_span: *target_span,
                    });
                }
                if let Some((value_kind, Some(target), _)) = semantic {
                    let parsed_target = parser::parse_inline(&target);
                    match (value_kind, parsed_target.as_slice()) {
                        (parser::ReferenceValueKind::DateStamp, [Inline::DateStamp(stamp)]) => {
                            push_date_occurrence(
                                *stamp,
                                origin.clone(),
                                span,
                                &mut occurrences.dates,
                            );
                        }
                        (parser::ReferenceValueKind::DateRange, [Inline::DateRange(range)])
                            if default_title =>
                        {
                            push_date_range_occurrence(
                                *range,
                                origin.clone(),
                                span,
                                &mut occurrences.dates,
                            );
                        }
                        _ => {}
                    }
                }
                occurrences.references.push_use(PendingReferenceUse {
                    key,
                    title: Some(*title),
                    presentation: ReferencePresentation::Link,
                    span,
                    marker_span: span,
                    title_span,
                    key_span,
                });
            }
            Inline::Footnote { raw, title, key } => {
                let (Some(span), Some(key_span)) =
                    (slice_span(source, raw), slice_span(source, key))
                else {
                    continue;
                };
                let title_span = if reference_uses_default_title(raw) {
                    None
                } else {
                    title.and_then(|title| slice_span(source, title))
                };
                occurrences.references.push_use(PendingReferenceUse {
                    key,
                    title: *title,
                    presentation: ReferencePresentation::Footnote,
                    span,
                    marker_span: span,
                    title_span,
                    key_span,
                });
            }
            Inline::HyperLink { raw, title, target } => {
                occurrences.external_links.insert(target.trim().to_string());
                if scope != InlineCollectionScope::Visible {
                    continue;
                }
                if let (Some(span), Some(target_span)) =
                    (slice_span(source, raw), slice_span(source, target))
                {
                    let title_span = title.and_then(|title| slice_span(source, title));
                    occurrences.url_links.push(UrlLinkOccurrence {
                        target: (*target).to_string(),
                        title: title.map(ToString::to_string),
                        span,
                        title_span,
                        target_span,
                    });
                    let reference_title =
                        (*title).unwrap_or_else(|| http_url_display_title(target));
                    let Some(reference_title_span) = slice_span(source, reference_title) else {
                        continue;
                    };
                    occurrences.reference_links.push(ReferenceLinkOccurrence {
                        title: reference_title.to_string(),
                        target: (*target).to_string(),
                        span,
                        title_span: reference_title_span,
                        target_span,
                    });
                }
            }
            Inline::DirectLink { raw, title, target } => {
                if let (Some(span), Some(title_span), Some(target_span)) = (
                    slice_span(source, raw),
                    slice_span(source, title),
                    slice_span(source, target),
                ) {
                    occurrences.reference_links.push(ReferenceLinkOccurrence {
                        title: (*title).to_string(),
                        target: (*target).to_string(),
                        span,
                        title_span,
                        target_span,
                    });
                }
            }
            Inline::DateStamp(stamp) => {
                collect_date(source, *stamp, origin.clone(), &mut occurrences.dates)
            }
            Inline::DateRange(range) => {
                collect_date_range(source, *range, origin.clone(), &mut occurrences.dates);
            }
            _ => {
                if let Some(children) = inline.nested_inlines() {
                    collect_inlines(source, children, origin.clone(), scope, occurrences);
                }
            }
        }
    }
}

fn reference_uses_default_title(raw: &str) -> bool {
    raw.ends_with("][]")
}

fn collect_date(
    source: &str,
    stamp: DateStamp<'_>,
    origin: DateOrigin,
    dates: &mut Vec<DateOccurrence>,
) {
    let Some(span) = date_stamp_span(source, stamp) else {
        return;
    };
    push_date_occurrence(stamp, origin, span, dates);
}

fn push_date_occurrence(
    stamp: DateStamp<'_>,
    origin: DateOrigin,
    span: SourceSpan,
    dates: &mut Vec<DateOccurrence>,
) {
    dates.push(DateOccurrence {
        kind: stamp.kind(),
        target: date_target_identity(stamp.target()),
        body: stamp.body().to_string(),
        origin,
        span,
    });
}

fn collect_date_range(
    source: &str,
    range: DateRange<'_>,
    origin: DateOrigin,
    dates: &mut Vec<DateOccurrence>,
) {
    let start = date_stamp_span(source, range.start());
    let end = date_stamp_span(source, range.end());
    if let (Some(start), Some(end)) = (start, end) {
        push_date_range_occurrence(range, origin, SourceSpan::new(start.start, end.end), dates);
    }
}

fn push_date_range_occurrence(
    range: DateRange<'_>,
    origin: DateOrigin,
    span: SourceSpan,
    dates: &mut Vec<DateOccurrence>,
) {
    let (Some(start), Some(end)) = (range.start().date(), range.end().date()) else {
        return;
    };
    dates.push(DateOccurrence {
        kind: range.kind(),
        target: DateTargetIdentity::Range { start, end },
        body: format!("{}--{}", range.start().body(), range.end().body()),
        origin,
        span,
    });
}

fn collect_property_dates(
    source: &str,
    inlines: &[Inline<'_>],
    origin: DateOrigin,
    dates: &mut Vec<DateOccurrence>,
) {
    for inline in inlines {
        match inline {
            Inline::DateStamp(stamp) => collect_date(source, *stamp, origin.clone(), dates),
            Inline::DateRange(range) => {
                collect_date_range(source, *range, origin.clone(), dates);
            }
            _ => {
                if let Some(children) = inline.nested_inlines() {
                    collect_property_dates(source, children, origin.clone(), dates);
                }
            }
        }
    }
}

fn collect_date_markers(
    source: &str,
    inlines: &[Inline<'_>],
    origin: DateMarkerOrigin,
    markers: &mut Vec<DateMarkerOccurrence>,
) {
    for inline in inlines {
        match inline {
            Inline::DateStamp(stamp) => {
                let Some(span) = date_stamp_span(source, *stamp) else {
                    continue;
                };
                markers.push(DateMarkerOccurrence {
                    kind: stamp.kind(),
                    target: date_target_identity(stamp.target()),
                    origin: origin.clone(),
                    span,
                });
            }
            Inline::DateRange(range) => {
                collect_date_range_marker(source, *range, origin.clone(), markers);
            }
            _ => {
                if let Some(children) = inline.nested_inlines() {
                    collect_date_markers(source, children, origin.clone(), markers);
                }
            }
        }
    }
}

fn collect_date_range_marker(
    source: &str,
    range: DateRange<'_>,
    origin: DateMarkerOrigin,
    markers: &mut Vec<DateMarkerOccurrence>,
) {
    let start_stamp = range.start();
    let end_stamp = range.end();
    let (Some(start_span), Some(end_span), Some(start), Some(end)) = (
        date_stamp_span(source, start_stamp),
        date_stamp_span(source, end_stamp),
        start_stamp.date(),
        end_stamp.date(),
    ) else {
        return;
    };
    markers.push(DateMarkerOccurrence {
        kind: range.kind(),
        target: DateTargetIdentity::Range { start, end },
        origin,
        span: SourceSpan::new(start_span.start, end_span.end),
    });
}

fn date_stamp_span(source: &str, stamp: DateStamp<'_>) -> Option<SourceSpan> {
    let body_span = slice_span(source, stamp.body())?;
    Some(SourceSpan::new(
        body_span.start.saturating_sub(1),
        (body_span.end + 1).min(source.len()),
    ))
}

fn date_target_identity(target: DateStampTarget) -> DateTargetIdentity {
    match target {
        DateStampTarget::Date(date) => DateTargetIdentity::Day(date),
        DateStampTarget::Month(month) => DateTargetIdentity::Month(month),
        DateStampTarget::IsoWeek(week) => DateTargetIdentity::IsoWeek(week),
    }
}

fn collect_inline_source_spans(source: &str, inlines: &[Inline<'_>], spans: &mut Vec<SourceSpan>) {
    for inline in inlines {
        let slice = match inline {
            Inline::Text(target)
            | Inline::Code(target)
            | Inline::Superscript(target)
            | Inline::Subscript(target)
            | Inline::Insertion(target)
            | Inline::Deletion(target) => Some(*target),
            Inline::NoteLink { raw, .. }
            | Inline::HyperLink { raw, .. }
            | Inline::Reference { raw, .. }
            | Inline::Footnote { raw, .. }
            | Inline::DirectLink { raw, .. } => Some(*raw),
            Inline::DateStamp(stamp) => Some(stamp.body()),
            Inline::DateRange(range) => Some(range.start().body()),
            Inline::SoftBreak | Inline::Italic(_) | Inline::Strong(_) | Inline::Highlight(_) => {
                None
            }
        };
        spans.extend(slice.and_then(|slice| slice_span(source, slice)));
        if let Some(children) = inline.nested_inlines() {
            collect_inline_source_spans(source, children, spans);
        }
    }
}

fn collect_attached_properties(
    source: &str,
    source_map: &SourceMap<'_>,
    declarations: &[parser::PropertyDeclaration<'_>],
    owner: PropertyOwner,
    properties: &mut Vec<PropertyOccurrence>,
) {
    properties.extend(declarations.iter().filter_map(|declaration| {
        let raw_span = slice_span(source, declaration.raw_line())?;
        let key_span = slice_span(source, declaration.key())?;
        let value_span = slice_span(source, declaration.value())?;
        let direction = match declaration.direction() {
            parser::PropertyDirection::Previous => PropertyDirection::Previous,
            parser::PropertyDirection::Next => PropertyDirection::Next,
        };

        Some(PropertyOccurrence {
            direction,
            owner,
            key: declaration.key().to_string(),
            value: declaration.value().to_string(),
            span: whole_line_span(source_map, raw_span),
            key_span,
            value_span,
        })
    }));
}

fn duplicate_id_diagnostics(
    path: &Path,
    block_ids: &[BlockIdOccurrence],
    headings: &[HeadingOccurrence],
) -> Vec<AnalysisDiagnostic> {
    let mut by_id: BTreeMap<&str, Vec<&BlockIdOccurrence>> = BTreeMap::new();
    for block_id in block_ids {
        by_id.entry(&block_id.id).or_default().push(block_id);
    }

    let mut diagnostics = Vec::new();
    for (id, occurrences) in &by_id {
        if occurrences.len() > 1 {
            diagnostics.extend(occurrences.iter().map(|occurrence| AnalysisDiagnostic {
                path: path.to_path_buf(),
                span: occurrence.value_span,
                kind: AnalysisDiagnosticKind::DuplicateId,
                subject: AnalysisDiagnosticSubject::Id((*id).to_string()),
                message: format!("duplicate id: {id}"),
            }));
        }
    }
    for block_id in block_ids {
        if by_id.get(block_id.id.as_str()).map(Vec::len) == Some(1)
            && headings
                .iter()
                .any(|heading| block_id_conflicts_with_heading(block_id, heading))
        {
            diagnostics.push(AnalysisDiagnostic {
                path: path.to_path_buf(),
                span: block_id.value_span,
                kind: AnalysisDiagnosticKind::DuplicateId,
                subject: AnalysisDiagnosticSubject::Id(block_id.id.clone()),
                message: format!("id conflicts with heading anchor: {}", block_id.id),
            });
        }
    }
    diagnostics
}

fn block_id_conflicts_with_heading(
    block_id: &BlockIdOccurrence,
    heading: &HeadingOccurrence,
) -> bool {
    block_id.id == heading.anchor
        && !(block_id.owner_kind == AnalysisBlockKind::Heading
            && block_id.owner_span == heading.span)
}

fn whole_line_span(source_map: &SourceMap<'_>, span: SourceSpan) -> SourceSpan {
    let line = source_map
        .position(span.start)
        .map_or(0, |position| position.line);
    source_map.line_span(line).unwrap_or(span)
}

fn covering_line_span(source_map: &SourceMap<'_>, spans: &[SourceSpan]) -> Option<SourceSpan> {
    let first = spans.iter().map(|span| span.start).min()?;
    let last = spans.iter().map(|span| span.end).max()?;
    let start_line = source_map.position(first)?.line;
    let end_line = source_map.position(last)?.line;
    Some(SourceSpan::new(
        source_map.line_span(start_line)?.start,
        source_map.line_span(end_line)?.end,
    ))
}

fn canonical_path(path: &Path) -> String {
    path.with_extension("")
        .to_string_lossy()
        .trim_start_matches("./")
        .replace('\\', "/")
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DocumentIndex {
    canonical_by_source: BTreeMap<PathBuf, String>,
    by_exact: BTreeMap<String, Vec<PathBuf>>,
    by_canonical: BTreeMap<String, Vec<PathBuf>>,
    by_stem: BTreeMap<String, Vec<PathBuf>>,
}

impl DocumentIndex {
    fn new(documents: &BTreeMap<PathBuf, DocumentAnalysis>) -> Self {
        let mut canonical_by_source = BTreeMap::new();
        let mut by_exact: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        let mut by_canonical: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        let mut by_stem: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();

        for document in documents.values() {
            canonical_by_source.insert(document.path.clone(), document.canonical_path.clone());
            by_exact
                .entry(document.canonical_path.clone())
                .or_default()
                .push(document.path.clone());
            by_canonical
                .entry(normalize_key(&document.canonical_path))
                .or_default()
                .push(document.path.clone());
            let stem = document
                .canonical_path
                .rsplit_once('/')
                .map_or(document.canonical_path.as_str(), |(_, stem)| stem);
            by_stem
                .entry(normalize_key(stem))
                .or_default()
                .push(document.path.clone());
        }

        Self {
            canonical_by_source,
            by_exact,
            by_canonical,
            by_stem,
        }
    }

    fn select(&self, current_path: &Path, selector: DocumentSelector<'_>) -> PathSelection {
        match selector {
            DocumentSelector::Current => {
                if self.canonical_by_source.contains_key(current_path) {
                    PathSelection::Found(current_path.to_path_buf())
                } else {
                    PathSelection::Broken
                }
            }
            DocumentSelector::Legacy(target) => self.resolve_legacy(current_path, target),
            DocumentSelector::Root(target) => self.resolve_coordinate(target),
            DocumentSelector::Child(target) => self.resolve_child(current_path, target),
        }
    }

    fn resolve_child(&self, current_path: &Path, target: &str) -> PathSelection {
        let normalized = normalize_document_target(target);
        if !is_normal_document_coordinate(&normalized) {
            return PathSelection::Broken;
        }
        let Some(current) = self.canonical_by_source.get(current_path) else {
            return PathSelection::Broken;
        };

        self.resolve_coordinate(&format!("{current}/{normalized}"))
    }

    fn resolve_coordinate(&self, target: &str) -> PathSelection {
        let normalized = normalize_document_target(target);
        if !is_normal_document_coordinate(&normalized) {
            return PathSelection::Broken;
        }

        if let Some(paths) = self.by_exact.get(normalized.as_ref()) {
            return select_paths(paths);
        }
        if let Some(paths) = self.by_canonical.get(&normalize_key(&normalized)) {
            return select_paths(paths);
        }

        PathSelection::Broken
    }

    fn resolve_legacy(&self, current_path: &Path, target: &str) -> PathSelection {
        let normalized = normalize_document_target(target);
        if !is_normal_document_coordinate(&normalized) {
            return PathSelection::Broken;
        }

        if let Some(paths) = self.by_exact.get(normalized.as_ref()) {
            return select_paths(paths);
        }
        if normalized.contains('/') {
            return self
                .by_canonical
                .get(&normalize_key(&normalized))
                .map_or(PathSelection::Broken, |paths| select_paths(paths));
        }

        let Some(current) = self.canonical_by_source.get(current_path) else {
            return PathSelection::Broken;
        };
        let sibling = current.rsplit_once('/').map_or_else(
            || normalized.to_string(),
            |(parent, _)| format!("{parent}/{normalized}"),
        );
        match self.resolve_coordinate(&sibling) {
            PathSelection::Broken => {}
            resolution => return resolution,
        }
        self.by_stem
            .get(&normalize_key(&normalized))
            .map_or(PathSelection::Broken, |paths| select_paths(paths))
    }
}

fn resolve_heading(document: &DocumentAnalysis, heading_target: &str) -> LinkResolution {
    if heading_target.is_empty() {
        return LinkResolution::BrokenHeading;
    }

    let mut matches = document
        .headings
        .iter()
        .filter(|heading| heading.anchor == heading_target)
        .collect::<Vec<_>>();
    if matches.is_empty() {
        let normalized = normalize_key(heading_target);
        matches = document
            .headings
            .iter()
            .filter(|heading| normalize_key(&heading.anchor) == normalized)
            .collect();
    }

    match matches.as_slice() {
        [heading]
            if document
                .block_ids
                .iter()
                .any(|block_id| block_id_conflicts_with_heading(block_id, heading)) =>
        {
            LinkResolution::AmbiguousHeading
        }
        [heading] => LinkResolution::Found(DefinitionTarget {
            path: document.path.clone(),
            canonical_path: document.canonical_path.clone(),
            selection_span: heading.title_span,
            kind: DefinitionTargetKind::Heading,
            fragment: Some(heading.anchor.clone()),
        }),
        [] => LinkResolution::BrokenHeading,
        _ => LinkResolution::AmbiguousHeading,
    }
}

fn resolve_id(document: &DocumentAnalysis, id_target: &str) -> LinkResolution {
    if id_target.is_empty() {
        return LinkResolution::BrokenId;
    }

    let matches = document
        .block_ids
        .iter()
        .filter(|block_id| block_id.id == id_target)
        .collect::<Vec<_>>();

    match matches.as_slice() {
        [block_id]
            if document
                .headings
                .iter()
                .any(|heading| block_id_conflicts_with_heading(block_id, heading)) =>
        {
            LinkResolution::AmbiguousId
        }
        [block_id] => LinkResolution::Found(DefinitionTarget {
            path: document.path.clone(),
            canonical_path: document.canonical_path.clone(),
            selection_span: block_id.value_span,
            kind: DefinitionTargetKind::Id,
            fragment: Some(block_id.id.clone()),
        }),
        [] => LinkResolution::BrokenId,
        _ => LinkResolution::AmbiguousId,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathSelection {
    Found(PathBuf),
    Broken,
    Ambiguous,
}

fn select_paths(paths: &[PathBuf]) -> PathSelection {
    match paths {
        [path] => PathSelection::Found(path.clone()),
        [] => PathSelection::Broken,
        _ => PathSelection::Ambiguous,
    }
}

fn normalize_document_target(target: &str) -> Cow<'_, str> {
    let target = target.strip_suffix(".maki").unwrap_or(target);
    if target.contains('\\') {
        Cow::Owned(target.replace('\\', "/"))
    } else {
        Cow::Borrowed(target)
    }
}

fn is_normal_document_coordinate(target: &str) -> bool {
    if target.is_empty() || target.starts_with('/') {
        return false;
    }

    target.split('/').all(is_normal_document_component)
}

fn is_normal_document_component(component: &str) -> bool {
    if component.is_empty() || matches!(component, "." | "..") {
        return false;
    }

    let bytes = component.as_bytes();
    !(bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

fn normalize_key(value: &str) -> String {
    value.to_lowercase()
}

fn diagnostic_for_resolution(
    target: &str,
    resolution: &LinkResolution,
) -> Option<(AnalysisDiagnosticKind, String)> {
    let (kind, label) = match resolution {
        LinkResolution::Found(_) => return None,
        LinkResolution::BrokenNote => (AnalysisDiagnosticKind::BrokenNoteLink, "broken note link"),
        LinkResolution::AmbiguousNote => (
            AnalysisDiagnosticKind::AmbiguousNoteLink,
            "ambiguous note link",
        ),
        LinkResolution::BrokenHeading => (
            AnalysisDiagnosticKind::BrokenHeadingLink,
            "broken heading link",
        ),
        LinkResolution::AmbiguousHeading => (
            AnalysisDiagnosticKind::AmbiguousHeadingLink,
            "ambiguous heading link",
        ),
        LinkResolution::BrokenId => (AnalysisDiagnosticKind::BrokenIdLink, "broken id link"),
        LinkResolution::AmbiguousId => {
            (AnalysisDiagnosticKind::AmbiguousIdLink, "ambiguous id link")
        }
    };

    Some((kind, format!("{label}: {target}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_snapshot_keeps_sources_analysis_and_revision_in_one_value() {
        let path = PathBuf::from("index.maki");
        let retained_path = PathBuf::from("retained.maki");
        let first = ProjectSnapshot::compile(BTreeMap::from([
            (path.clone(), "--^ title: First\n".to_string()),
            (retained_path.clone(), "= Retained\n".to_string()),
        ]));
        let cloned = first.clone();
        let second = ProjectSnapshot::compile(BTreeMap::from([(
            path.clone(),
            "--^ title: Second\n".to_string(),
        )]));
        let updated = first.with_source(path.clone(), Some("--^ title: Updated\n".to_string()));

        assert_eq!(first.source(&path), Some("--^ title: First\n"));
        assert_eq!(first.analysis().document(&path).unwrap().title, "First");
        assert_eq!(cloned.revision(), first.revision());
        assert_ne!(second.revision(), first.revision());
        assert_eq!(second.analysis().document(&path).unwrap().title, "Second");
        assert_eq!(updated.source(&path), Some("--^ title: Updated\n"));
        assert_eq!(updated.analysis().document(&path).unwrap().title, "Updated");
        assert_ne!(updated.revision(), first.revision());
        assert!(Arc::ptr_eq(
            first.sources.get(&retained_path).unwrap(),
            updated.sources.get(&retained_path).unwrap(),
        ));

        let removed = updated.with_source(retained_path.clone(), None);
        assert_eq!(removed.source(&retained_path), None);
        assert_eq!(
            removed
                .source_paths()
                .map(Path::to_path_buf)
                .collect::<Vec<_>>(),
            vec![path]
        );
    }

    #[test]
    fn standalone_and_project_analysis_share_nested_document_traversal() {
        let path = Path::new("index.maki");
        let source = r#"> = Nested
> --^ id: nested
> [2026-09-09] <https://example.com>

--- quote
== Deeper
--^ status: active
---"#;

        let standalone = analyze_document(path, source);
        let project = analyze_project(&[SourceSnapshot { path, source }]);

        assert_eq!(project.document(path), Some(&standalone));
    }

    #[test]
    fn document_analysis_handles_a_container_without_a_kind() {
        let source = "---\nplain\n---";

        let analysis = analyze_document(Path::new("index.maki"), source);

        assert_eq!(analysis.blocks.len(), 1);
        assert_eq!(analysis.blocks[0].kind, AnalysisBlockKind::Container);
    }

    #[test]
    fn document_analysis_locates_headings_properties_links_and_dates() {
        let source = "--^ title: 문서\n\n= 소개\n--^ id: intro\n[[다른문서#詳細]] [2026-08-25]\n";
        let (analysis, title_origin) =
            analyze_document_with_title_origin(Path::new("docs/current.maki"), source);

        assert_eq!(analysis.title, "문서");
        assert_eq!(title_origin, DocumentTitleOrigin::Authored);
        assert_eq!(analysis.headings[0].anchor, "intro");
        assert_eq!(
            &source[analysis.headings[0].title_span.start..analysis.headings[0].title_span.end],
            "소개"
        );
        assert_eq!(analysis.properties.len(), 2);
        assert_eq!(analysis.note_links[0].target, "다른문서#詳細");
        assert_eq!(analysis.dates[0].origin, DateOrigin::VisibleInline);
    }

    #[test]
    fn document_analysis_preserves_titled_note_and_url_link_spans() {
        let source = "😀[표시][[/다른#제목]] <http://bare.example/path> [사이트]<HTTPS://named.example/path> [로컬](/path)";
        let analysis = analyze_document(Path::new("index.maki"), source);

        assert_eq!(analysis.note_links.len(), 1);
        let note = &analysis.note_links[0];
        assert_eq!(note.title.as_deref(), Some("표시"));
        assert_eq!(
            &source[note.span.start..note.span.end],
            "[표시][[/다른#제목]]"
        );
        assert_eq!(
            note.title_span.map(|span| &source[span.start..span.end]),
            Some("표시")
        );
        assert_eq!(
            &source[note.target_span.start..note.target_span.end],
            "/다른#제목"
        );

        assert_eq!(analysis.url_links.len(), 2);
        let bare = &analysis.url_links[0];
        assert_eq!(bare.title, None);
        assert_eq!(
            &source[bare.span.start..bare.span.end],
            "<http://bare.example/path>"
        );
        assert_eq!(bare.title_span, None);
        assert_eq!(
            &source[bare.target_span.start..bare.target_span.end],
            "http://bare.example/path"
        );
        let bare_reference = &analysis.reference_links[0];
        assert_eq!(bare_reference.title, "bare.example/path");
        assert_eq!(
            &source[bare_reference.title_span.start..bare_reference.title_span.end],
            bare_reference.title
        );
        let named = &analysis.url_links[1];
        assert_eq!(named.title.as_deref(), Some("사이트"));
        assert_eq!(
            &source[named.span.start..named.span.end],
            "[사이트]<HTTPS://named.example/path>"
        );
        assert_eq!(
            named.title_span.map(|span| &source[span.start..span.end]),
            Some("사이트")
        );
        assert_eq!(
            &source[named.target_span.start..named.target_span.end],
            "HTTPS://named.example/path"
        );

        assert_eq!(analysis.reference_links.len(), 3);
        assert_eq!(
            analysis.reference_links[0].target,
            "http://bare.example/path"
        );
        assert_eq!(
            analysis.reference_links[1].target,
            "HTTPS://named.example/path"
        );
        let local = &analysis.reference_links[2];
        assert_eq!(local.title, "로컬");
        assert_eq!(local.target, "/path");
        assert_eq!(&source[local.span.start..local.span.end], "[로컬](/path)");
        assert_eq!(
            &source[local.title_span.start..local.title_span.end],
            "로컬"
        );
        assert_eq!(
            &source[local.target_span.start..local.target_span.end],
            "/path"
        );
    }

    #[test]
    fn titled_note_links_keep_project_resolution_by_target() {
        let project = analyze_project(&[
            SourceSnapshot {
                path: Path::new("index.maki"),
                source: "[다른 이름][[/target]]",
            },
            SourceSnapshot {
                path: Path::new("target.maki"),
                source: "--^ title: Target\n",
            },
        ]);
        let link = &project
            .document(Path::new("index.maki"))
            .unwrap()
            .note_links[0];

        assert_eq!(link.title.as_deref(), Some("다른 이름"));
        assert!(matches!(
            link.resolution,
            Some(LinkResolution::Found(DefinitionTarget {
                ref path,
                kind: DefinitionTargetKind::Document,
                ..
            })) if path == Path::new("target.maki")
        ));
    }

    #[test]
    fn url_link_occurrences_exclude_reference_definition_values() {
        let source = r#"Visible <https://visible.example/>.

[exact]: <https://exact.example/>
[prose]: Before <https://prose.example/> after
[note]: [[target]]"#;
        let analysis = analyze_document(Path::new("index.maki"), source);

        assert_eq!(analysis.url_links.len(), 1);
        assert_eq!(analysis.url_links[0].target, "https://visible.example/");
        assert_eq!(analysis.reference_links.len(), 1);
        assert_eq!(
            analysis.reference_links[0].target,
            "https://visible.example/"
        );
        assert_eq!(analysis.note_links.len(), 1);
        assert_eq!(analysis.note_links[0].target, "target");
        assert_eq!(
            analysis.external_links,
            vec![
                "https://exact.example/",
                "https://prose.example/",
                "https://visible.example/",
            ]
        );
    }

    #[test]
    fn document_analysis_marks_file_stem_fallback_titles() {
        let (analysis, title_origin) =
            analyze_document_with_title_origin(Path::new("docs/current.maki"), "body");

        assert_eq!(analysis.title, "current");
        assert_eq!(title_origin, DocumentTitleOrigin::FileStem);
    }

    #[test]
    fn document_analysis_handles_whitespace_only_property_fields() {
        let source = "--^ title: \n--v   : value\n";

        let analysis = analyze_document(Path::new("index.maki"), source);

        assert_eq!(analysis.properties.len(), 1);
        assert_eq!(analysis.properties[0].key, "title");
        assert_eq!(analysis.properties[0].value, "");
        assert_eq!(analysis.properties[0].value_span, SourceSpan::new(10, 10));
        assert_eq!(analysis.properties[0].owner, PropertyOwner::Document);
    }

    #[test]
    fn document_analysis_records_parser_authoritative_property_owners() {
        let source = "--^ title: Project\n= First\n--v status: todo\n= Second\n--^ status: done\n";

        let analysis = analyze_document(Path::new("index.maki"), source);

        assert_eq!(analysis.properties.len(), 3);
        assert_eq!(analysis.properties[0].owner, PropertyOwner::Document);
        let block = &analysis.blocks[1];
        for property in &analysis.properties[1..] {
            assert_eq!(
                property.owner,
                PropertyOwner::Block {
                    kind: AnalysisBlockKind::Heading,
                    span: block.span,
                }
            );
        }
        assert_eq!(analysis.properties[1].direction, PropertyDirection::Next);
        assert_eq!(
            analysis.properties[2].direction,
            PropertyDirection::Previous
        );
    }

    #[test]
    fn property_values_only_contribute_date_occurrences_to_inline_analysis() {
        let source = "--^ title: [missing][] [[note]] [direct](page) *[2026-08-31]*\n";

        let analysis = analyze_document(Path::new("index.maki"), source);

        assert!(analysis.reference_graph.uses.is_empty());
        assert!(analysis.reference_links.is_empty());
        assert!(analysis.note_links.is_empty());
        assert!(analysis.diagnostics.is_empty());
        assert_eq!(analysis.dates.len(), 1);
        assert_eq!(analysis.dates[0].body, "2026-08-31");
        assert_eq!(
            analysis.dates[0].origin,
            DateOrigin::PropertyValue {
                key: "title".to_string()
            }
        );
    }

    #[test]
    fn nested_date_markers_use_parser_attached_properties_with_root_spans() {
        let source = "---quote\n--^ deadline: [2026-09-07]\nbody <2026-09-08>\n---\n\noutside\n--^ deadline: [2026-09-09]\n";
        let analysis = analyze_document(Path::new("index.maki"), source);

        assert_eq!(analysis.properties.len(), 2);
        assert_eq!(analysis.dates.len(), 3);
        assert_eq!(
            analysis
                .date_markers
                .iter()
                .map(|marker| &source[marker.span.start..marker.span.end])
                .collect::<Vec<_>>(),
            vec!["[2026-09-07]", "<2026-09-08>", "[2026-09-09]"]
        );
    }

    #[test]
    fn authored_date_markers_map_reparsed_quote_bodies_to_root_spans() {
        let source = "> [2026-09-07]\n\n---quote\n<2026-09-08>\n---\n\noutside [2026-09-09]\n";
        let analysis = analyze_document(Path::new("index.maki"), source);

        assert_eq!(
            analysis
                .date_markers
                .iter()
                .map(|marker| &source[marker.span.start..marker.span.end])
                .collect::<Vec<_>>(),
            vec!["[2026-09-07]", "<2026-09-08>", "[2026-09-09]"]
        );
    }

    #[test]
    fn nested_quote_navigation_and_diagnostics_use_root_source_spans() {
        let source = "intro\r\n> = 인용 😀\r\n> See [[missing]] and [unknown][]\r\n> [local][]\r\n> [local]: [[local-target]]\r\n\r\n---quote\r\n= Container\r\n--^ id: nested-id\r\n[[#nested-id]] [[@nested-id]]\r\n---\r\n";
        let project = analyze_project(&[SourceSnapshot {
            path: Path::new("index.maki"),
            source,
        }]);
        let document = project.document(Path::new("index.maki")).unwrap();

        assert_eq!(
            document
                .headings
                .iter()
                .map(|heading| &source[heading.title_span.start..heading.title_span.end])
                .collect::<Vec<_>>(),
            vec!["인용 😀", "Container"]
        );
        let nested_id = document
            .block_ids
            .iter()
            .find(|block_id| block_id.id == "nested-id")
            .unwrap();
        assert_eq!(
            &source[nested_id.value_span.start..nested_id.value_span.end],
            "nested-id"
        );
        assert_eq!(
            &source[nested_id.owner_span.start..nested_id.owner_span.end],
            "= Container"
        );
        assert!(document.note_links.iter().any(|link| {
            link.target == "#nested-id"
                && matches!(
                    link.resolution,
                    Some(LinkResolution::Found(DefinitionTarget {
                        kind: DefinitionTargetKind::Heading,
                        ..
                    }))
                )
        }));
        assert!(project.diagnostics.iter().any(|diagnostic| {
            diagnostic.subject == AnalysisDiagnosticSubject::Link("missing".to_string())
                && &source[diagnostic.span.start..diagnostic.span.end] == "missing"
        }));
        assert!(project.diagnostics.iter().any(|diagnostic| {
            diagnostic.subject == AnalysisDiagnosticSubject::Reference("unknown".to_string())
                && &source[diagnostic.span.start..diagnostic.span.end] == "unknown"
        }));
        let local_use = document
            .reference_graph
            .uses
            .iter()
            .find(|usage| usage.key == "local")
            .unwrap();
        assert_eq!(local_use.scope, ReferenceScope::Nested);
        assert!(local_use.definition_id.is_some());
        assert_eq!(
            &source[local_use.key_span.start..local_use.key_span.end],
            "local"
        );
        assert!(document.reference_graph.winner("local").is_none());
    }

    #[test]
    fn nested_reference_scopes_resolve_the_nearest_definition_without_leaking_to_siblings() {
        let source = concat!(
            "> [shared][]\r\n",
            "> > [shared][] [day][] [site][]\r\n",
            "> [shared]: [[outer-target]]\r\n",
            "\r\n",
            "[shared]: [[root-target]]\r\n",
            "[day]: [2026-09-07]\r\n",
            "[site]: <https://root.example/>\r\n",
            "\r\n",
            "> [shared][]\r\n",
        );
        let analysis = analyze_document(Path::new("index.maki"), source);
        let graph = &analysis.reference_graph;

        let root_shared = graph.winner_id("shared").unwrap();
        let outer_shared = graph
            .definitions
            .iter()
            .find(|definition| definition.key == "shared" && definition.id != root_shared)
            .unwrap()
            .id;
        let shared_uses = graph
            .uses
            .iter()
            .filter(|usage| usage.key == "shared")
            .collect::<Vec<_>>();

        assert_eq!(shared_uses.len(), 3);
        assert_eq!(shared_uses[0].definition_id, Some(outer_shared));
        assert_eq!(shared_uses[1].definition_id, Some(outer_shared));
        assert_eq!(shared_uses[2].definition_id, Some(root_shared));
        assert_eq!(graph.uses_for(outer_shared).count(), 2);
        assert!(shared_uses.iter().all(|usage| {
            &source[usage.key_span.start..usage.key_span.end] == "shared"
                && usage.scope == ReferenceScope::Nested
        }));

        let day_use = graph.uses.iter().find(|usage| usage.key == "day").unwrap();
        assert_eq!(day_use.definition_id, graph.winner_id("day"));
        assert_eq!(analysis.dates.len(), 1);
        assert_eq!(analysis.dates[0].body, "2026-09-07");
        assert_eq!(
            &source[analysis.dates[0].span.start..analysis.dates[0].span.end],
            "[day][]"
        );

        let site_link = analysis
            .reference_links
            .iter()
            .find(|link| link.target == "https://root.example/")
            .unwrap();
        assert_eq!(
            &source[site_link.span.start..site_link.span.end],
            "[site][]"
        );
        assert_eq!(
            &source[site_link.target_span.start..site_link.target_span.end],
            "https://root.example/"
        );
        assert!(
            analysis.diagnostics.iter().all(|diagnostic| {
                diagnostic.kind != AnalysisDiagnosticKind::UnresolvedReference
            })
        );
    }

    #[test]
    fn document_analysis_locates_reference_link_markers() {
        let source = "- [김치][]\n\n[김치]: <https://hakkeido.com/>\n";
        let analysis = analyze_document(Path::new("index.maki"), source);

        assert_eq!(analysis.reference_links.len(), 1);
        let reference = &analysis.reference_links[0];
        assert_eq!(reference.title, "김치");
        assert_eq!(reference.target, "https://hakkeido.com/");
        assert_eq!(
            &source[reference.span.start..reference.span.end],
            "[김치][]"
        );
        assert_eq!(
            &source[reference.title_span.start..reference.title_span.end],
            "김치"
        );
        assert_eq!(
            &source[reference.target_span.start..reference.target_span.end],
            "https://hakkeido.com/"
        );
    }

    #[test]
    fn reference_use_spans_distinguish_default_titles_and_url_targets() {
        let source = "[web][] [shown][web] [^web][] [^shown][web] [direct]<https://direct.example>\n\n[web]: <https://example.com>";
        let analysis = analyze_document(Path::new("index.maki"), source);
        let uses = &analysis.reference_graph.uses;

        assert_eq!(uses.len(), 4);
        assert_eq!(uses[0].title_span, None);
        assert_eq!(uses[1].title.as_deref(), Some("shown"));
        assert_eq!(
            uses[1].title_span.map(|span| &source[span.start..span.end]),
            Some("shown")
        );
        assert_eq!(uses[2].title_span, None);
        assert_eq!(
            uses[3].title_span.map(|span| &source[span.start..span.end]),
            Some("shown")
        );

        let direct = &analysis.url_links[0];
        assert_eq!(direct.title.as_deref(), Some("direct"));
        assert_eq!(
            &source[direct.target_span.start..direct.target_span.end],
            "https://direct.example"
        );
    }

    #[test]
    fn date_reference_uses_contribute_dates_at_their_rendered_markers() {
        let source = "[day][] [range][] [label][range]\n\n[day]: [2026-08-31]\n[range]: [2026-09-01]--[2026-09-02]";
        let analysis = analyze_document(Path::new("index.maki"), source);

        assert_eq!(
            analysis
                .dates
                .iter()
                .map(|date| (date.body.as_str(), &source[date.span.start..date.span.end]))
                .collect::<Vec<_>>(),
            vec![
                ("2026-08-31", "[day][]"),
                ("2026-09-01--2026-09-02", "[range][]"),
            ]
        );
    }

    #[test]
    fn authored_date_markers_normalize_equivalent_day_spellings_and_keep_period_identity() {
        let source = "*[2026-09-07 Monday]* [2026-W37-1] [2026-09] [2026-W37]";
        let analysis = analyze_document(Path::new("index.maki"), source);

        assert_eq!(analysis.date_markers.len(), 4);
        assert_eq!(
            analysis.date_markers[0].target,
            DateTargetIdentity::Day(Date::parse("2026-09-07").unwrap())
        );
        assert_eq!(
            analysis.date_markers[1].target,
            analysis.date_markers[0].target
        );
        assert_eq!(
            analysis.date_markers[2].target,
            DateTargetIdentity::Month(DateMonth::new(2026, 9).unwrap())
        );
        assert_eq!(
            analysis.date_markers[3].target,
            DateTargetIdentity::IsoWeek(IsoWeek::new(2026, 37).unwrap())
        );
        assert!(analysis.date_markers.iter().all(|marker| {
            marker.kind == DateStampKind::Date && marker.origin == DateMarkerOrigin::Inline
        }));
        assert_eq!(
            analysis
                .date_markers
                .iter()
                .map(|marker| &source[marker.span.start..marker.span.end])
                .collect::<Vec<_>>(),
            vec![
                "[2026-09-07 Monday]",
                "[2026-W37-1]",
                "[2026-09]",
                "[2026-W37]",
            ]
        );
    }

    #[test]
    fn authored_date_markers_record_property_and_root_reference_definition_origins() {
        let source = "--^ scheduled: *<2026-09-07 10:00>*\n\n[launch]: See **[2026-W37-1 Monday]**\n[launch]: <2026-09-08 18:00>";
        let analysis = analyze_document(Path::new("index.maki"), source);

        assert_eq!(analysis.date_markers.len(), 3);
        assert_eq!(
            analysis.date_markers[0].origin,
            DateMarkerOrigin::PropertyValue {
                key: "scheduled".to_string()
            }
        );
        assert_eq!(analysis.date_markers[0].kind, DateStampKind::Event);
        assert_eq!(
            analysis.date_markers[1].origin,
            DateMarkerOrigin::ReferenceDefinitionValue {
                key: "launch".to_string()
            }
        );
        assert_eq!(
            analysis.date_markers[2].origin,
            DateMarkerOrigin::ReferenceDefinitionValue {
                key: "launch".to_string()
            }
        );
        assert_eq!(analysis.date_markers[2].kind, DateStampKind::Event);
        assert_eq!(
            analysis
                .date_markers
                .iter()
                .map(|marker| &source[marker.span.start..marker.span.end])
                .collect::<Vec<_>>(),
            vec![
                "<2026-09-07 10:00>",
                "[2026-W37-1 Monday]",
                "<2026-09-08 18:00>",
            ]
        );
    }

    #[test]
    fn authored_date_range_is_one_marker_covering_both_endpoints_and_separator() {
        let source = "before <2026-W37-1 Mon>--<2026-09-11 Fri> after";
        let analysis = analyze_document(Path::new("index.maki"), source);

        assert_eq!(analysis.date_markers.len(), 1);
        assert_eq!(analysis.date_markers[0].kind, DateStampKind::Event);
        assert_eq!(analysis.date_markers[0].origin, DateMarkerOrigin::Inline);
        assert_eq!(
            analysis.date_markers[0].target,
            DateTargetIdentity::Range {
                start: Date::parse("2026-09-07").unwrap(),
                end: Date::parse("2026-09-11").unwrap(),
            }
        );
        assert_eq!(
            &source[analysis.date_markers[0].span.start..analysis.date_markers[0].span.end],
            "<2026-W37-1 Mon>--<2026-09-11 Fri>"
        );
        assert_eq!(analysis.dates.len(), 1);
        assert_eq!(analysis.dates[0].target, analysis.date_markers[0].target);
    }

    #[test]
    fn authored_date_markers_exclude_semantically_expanded_reference_uses() {
        let source = "[day][] [range][] [shown][day]\n\n[day]: [2026-09-07]\n[range]: [2026-09-08]--[2026-09-09]";
        let analysis = analyze_document(Path::new("index.maki"), source);

        assert_eq!(
            analysis
                .date_markers
                .iter()
                .map(|marker| &source[marker.span.start..marker.span.end])
                .collect::<Vec<_>>(),
            vec!["[2026-09-07]", "[2026-09-08]--[2026-09-09]"]
        );
        assert!(analysis.date_markers.iter().all(|marker| matches!(
            marker.origin,
            DateMarkerOrigin::ReferenceDefinitionValue { .. }
        )));
        assert_eq!(analysis.dates.len(), 3);
        assert_eq!(
            &source[analysis.dates[0].span.start..analysis.dates[0].span.end],
            "[day][]"
        );
    }

    #[test]
    fn document_reference_graph_unifies_definitions_and_collects_each_use_once() {
        let source = r#"[shared][] *[^shared][]*
- [alias][]
| usage |
|---|
| [^shared][] |

[shared]: See [alias][] [^alias][].
[shared]: ignored [alias][]
[alias]: Alias body."#;
        let analysis = analyze_document(Path::new("index.maki"), source);
        let graph = &analysis.reference_graph;

        assert!(
            analysis
                .blocks
                .iter()
                .any(|block| block.kind == AnalysisBlockKind::List)
        );
        assert!(
            analysis
                .blocks
                .iter()
                .any(|block| block.kind == AnalysisBlockKind::Table)
        );

        assert_eq!(graph.definitions.len(), 3);
        assert_eq!(graph.definitions[0].id, ReferenceDefinitionId(0));
        assert_eq!(graph.definitions[0].key, "shared");
        assert_eq!(graph.definitions[0].value, "See [alias][] [^alias][].");
        assert_eq!(
            graph.definitions[0].value_kind,
            parser::ReferenceValueKind::Prose
        );
        assert_eq!(graph.definitions[0].state, ReferenceDefinitionState::Active);
        assert_eq!(
            graph.definitions[1].state,
            ReferenceDefinitionState::Duplicate {
                winner: ReferenceDefinitionId(0)
            }
        );
        assert_eq!(graph.definitions[2].key, "alias");
        assert_eq!(
            graph.definitions[2].value_kind,
            parser::ReferenceValueKind::Prose
        );
        assert_eq!(
            &source[graph.definitions[1].key_span.start..graph.definitions[1].key_span.end],
            "shared"
        );
        assert_eq!(
            &source[graph.definitions[1].value_span.start..graph.definitions[1].value_span.end],
            "ignored [alias][]"
        );

        assert_eq!(graph.uses.len(), 6);
        assert_eq!(
            graph
                .uses
                .iter()
                .map(|usage| (usage.key.as_str(), usage.presentation, usage.definition_id))
                .collect::<Vec<_>>(),
            vec![
                (
                    "shared",
                    ReferencePresentation::Link,
                    Some(ReferenceDefinitionId(0))
                ),
                (
                    "shared",
                    ReferencePresentation::Footnote,
                    Some(ReferenceDefinitionId(0))
                ),
                (
                    "alias",
                    ReferencePresentation::Link,
                    Some(ReferenceDefinitionId(2))
                ),
                (
                    "shared",
                    ReferencePresentation::Footnote,
                    Some(ReferenceDefinitionId(0))
                ),
                (
                    "alias",
                    ReferencePresentation::Link,
                    Some(ReferenceDefinitionId(2))
                ),
                (
                    "alias",
                    ReferencePresentation::Footnote,
                    Some(ReferenceDefinitionId(2))
                ),
            ]
        );
        for usage in &graph.uses {
            let marker = &source[usage.marker_span.start..usage.marker_span.end];
            assert_eq!(marker, &source[usage.span.start..usage.span.end]);
            assert!(marker.starts_with('[') && marker.ends_with(']'));
            assert_eq!(&source[usage.key_span.start..usage.key_span.end], usage.key);
        }
        assert_eq!(graph.uses_for(ReferenceDefinitionId(0)).count(), 3);
        assert_eq!(graph.uses_for(ReferenceDefinitionId(2)).count(), 3);
        assert!(analysis.reference_links.is_empty());
    }

    #[test]
    fn document_reference_graph_keeps_keys_case_sensitive_and_document_local() {
        let first = analyze_document(
            Path::new("first.maki"),
            "[Key][] [key][] [^Key][]\n\n[Key]: upper\n[key]: lower",
        );
        let second = analyze_document(Path::new("second.maki"), "[Key][]\n\n[Key]: other");

        assert_eq!(
            first
                .reference_graph
                .winner("Key")
                .map(|item| item.value.as_str()),
            Some("upper")
        );
        assert_eq!(
            first
                .reference_graph
                .winner("key")
                .map(|item| item.value.as_str()),
            Some("lower")
        );
        assert_eq!(first.reference_graph.uses.len(), 3);
        assert_eq!(
            second.reference_graph.definitions[0].id,
            ReferenceDefinitionId(0)
        );
        assert_eq!(second.reference_graph.definitions[0].value, "other");
        assert_eq!(second.reference_graph.uses.len(), 1);
    }

    #[test]
    fn document_reference_graph_ignores_non_root_definition_blocks() {
        let source = r#"- parent
  [nested]: <https://nested.example/>
  [nested][]

[root][]
[root]: <https://root.example/>"#;
        let analysis = analyze_document(Path::new("index.maki"), source);

        assert_eq!(analysis.reference_graph.definitions.len(), 1);
        assert_eq!(analysis.reference_graph.definitions[0].key, "root");
        assert_eq!(analysis.reference_graph.uses.len(), 2);
        assert_eq!(analysis.reference_graph.uses[0].key, "nested");
        assert_eq!(analysis.reference_graph.uses[0].definition_id, None);
        assert_eq!(analysis.reference_graph.uses[1].key, "root");
        assert!(analysis.reference_graph.winner("nested").is_none());
        assert_eq!(analysis.reference_links.len(), 1);
        assert!(analysis.diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == AnalysisDiagnosticKind::UnresolvedReference
                && diagnostic.message == "unresolved reference: nested"
        }));
    }

    #[test]
    fn exact_note_link_reference_values_are_resolved_without_becoming_reference_uses() {
        let analysis = analyze_document(Path::new("index.maki"), "[raw][]\n\n[raw]: [[missing]]");

        assert_eq!(analysis.reference_graph.uses.len(), 1);
        assert_eq!(analysis.reference_links.len(), 1);
        assert_eq!(analysis.reference_links[0].target, "missing");
        assert_eq!(analysis.note_links.len(), 1);
        assert_eq!(analysis.note_links[0].target, "missing");
        assert!(analysis.dates.is_empty());
    }

    #[test]
    fn document_analysis_collects_addressable_block_ids_with_owner_spans() {
        let source = "--^ id: document-id\n\n= Heading\n--^ id: heading-id\n\n- parent\n  nested paragraph\n  --^ id: nested-id\n--^ id: list-id\n\n[target]: https://example.com\n--^ id: hidden-id\n\nempty\n--^ id:\n";
        let analysis = analyze_document(Path::new("index.maki"), source);

        assert_eq!(
            analysis
                .block_ids
                .iter()
                .map(|block_id| (block_id.id.as_str(), block_id.owner_kind))
                .collect::<Vec<_>>(),
            vec![
                ("heading-id", AnalysisBlockKind::Heading),
                ("nested-id", AnalysisBlockKind::Paragraph),
                ("list-id", AnalysisBlockKind::List),
            ]
        );

        let nested = &analysis.block_ids[1];
        assert_eq!(
            &source[nested.owner_span.start..nested.owner_span.end],
            "  nested paragraph"
        );
        assert_eq!(
            &source[nested.declaration_span.start..nested.declaration_span.end],
            "  --^ id: nested-id"
        );
        assert_eq!(
            &source[nested.value_span.start..nested.value_span.end],
            "nested-id"
        );
    }

    #[test]
    fn document_analysis_diagnoses_every_exact_duplicate_id_declaration() {
        let source = "first\n--^ id: same\nsecond\n--^ id: same\nthird\n--^ id: Same\n";
        let analysis = analyze_document(Path::new("index.maki"), source);
        let duplicates = analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.kind == AnalysisDiagnosticKind::DuplicateId)
            .collect::<Vec<_>>();

        assert_eq!(duplicates.len(), 2);
        assert!(duplicates.iter().all(|diagnostic| {
            &source[diagnostic.span.start..diagnostic.span.end] == "same"
                && diagnostic.message == "duplicate id: same"
        }));
    }

    #[test]
    fn project_date_marker_index_normalizes_targets_and_sorts_locations_by_path_and_span() {
        let later_source = "[2026-09-07] then <2026-09-07>";
        let earlier_source = "<2026-W37-1 Monday>";
        let project = analyze_project(&[
            SourceSnapshot {
                path: Path::new("z.maki"),
                source: later_source,
            },
            SourceSnapshot {
                path: Path::new("a.maki"),
                source: earlier_source,
            },
        ]);
        let target = DateTargetIdentity::Day(Date::parse("2026-09-07").unwrap());

        let locations = project.date_marker_locations(&target);
        assert_eq!(locations.len(), 3);
        assert_eq!(
            locations
                .iter()
                .map(|location| location.path.as_path())
                .collect::<Vec<_>>(),
            vec![
                Path::new("a.maki"),
                Path::new("z.maki"),
                Path::new("z.maki")
            ]
        );
        assert_eq!(
            locations
                .iter()
                .map(|location| location.kind)
                .collect::<Vec<_>>(),
            vec![
                DateStampKind::Event,
                DateStampKind::Date,
                DateStampKind::Event
            ]
        );
        assert!(locations[1].span < locations[2].span);
        assert!(
            project
                .date_marker_locations(&DateTargetIdentity::Month(DateMonth::new(2026, 9).unwrap()))
                .is_empty()
        );
    }

    #[test]
    fn project_analysis_resolves_exact_note_link_reference_targets() {
        let source = "[source][]\n\n[source]: [[missing]]";
        let project = analyze_project(&[SourceSnapshot {
            path: Path::new("index.maki"),
            source,
        }]);
        let document = project.document(Path::new("index.maki")).unwrap();

        assert_eq!(document.note_links.len(), 1);
        assert_eq!(
            document.note_links[0].resolution,
            Some(LinkResolution::BrokenNote)
        );
        assert!(project.diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == AnalysisDiagnosticKind::BrokenNoteLink
                && &source[diagnostic.span.start..diagnostic.span.end] == "missing"
        }));
    }

    #[test]
    fn project_analysis_rejects_explicit_ids_that_collide_with_other_heading_fragments() {
        let source = "= shared\n\nbody\n--^ id: shared\n\n[[#shared]] [[@shared]]\n";
        let analysis = analyze_project(&[SourceSnapshot {
            path: Path::new("index.maki"),
            source,
        }]);
        let document = analysis.document(Path::new("index.maki")).unwrap();

        assert!(document.diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == AnalysisDiagnosticKind::DuplicateId
                && diagnostic.message == "id conflicts with heading anchor: shared"
        }));
        assert_eq!(
            document.note_links[0].resolution,
            Some(LinkResolution::AmbiguousHeading)
        );
        assert_eq!(
            document.note_links[1].resolution,
            Some(LinkResolution::AmbiguousId)
        );
    }

    #[test]
    fn project_analysis_resolves_notes_before_heading_lookup() {
        let current = "= Current\n[[other#詳細]]\n[[missing#Heading]]\n";
        let other = "= 詳細\n";
        let analysis = analyze_project(&[
            SourceSnapshot {
                path: Path::new("docs/current.maki"),
                source: current,
            },
            SourceSnapshot {
                path: Path::new("docs/other.maki"),
                source: other,
            },
        ]);
        let current = analysis
            .document(Path::new("docs/current.maki"))
            .expect("current document should exist");

        assert!(matches!(
            current.note_links[0].resolution,
            Some(LinkResolution::Found(DefinitionTarget {
                fragment: Some(_),
                ..
            }))
        ));
        assert_eq!(
            current.note_links[1].resolution,
            Some(LinkResolution::BrokenNote)
        );
    }

    #[test]
    fn project_analysis_preserves_exact_canonical_path_priority() {
        let analysis = analyze_project(&[
            SourceSnapshot {
                path: Path::new("index.maki"),
                source: "[[nix]]",
            },
            SourceSnapshot {
                path: Path::new("nix.maki"),
                source: "lower",
            },
            SourceSnapshot {
                path: Path::new("NIX.maki"),
                source: "upper",
            },
        ]);
        let index = analysis.document(Path::new("index.maki")).unwrap();

        assert!(matches!(
            index.note_links[0].resolution,
            Some(LinkResolution::Found(DefinitionTarget { ref path, .. }))
                if path == Path::new("nix.maki")
        ));
    }

    #[test]
    fn document_selection_contract_is_host_independent_and_exact_first() {
        #[derive(Debug, Clone, Copy)]
        enum Expected<'a> {
            Found(&'a str),
            Broken,
            Ambiguous,
        }

        let sources = [
            ("docs/current.maki", "current"),
            ("docs/current/child.maki", "child"),
            ("docs/current/deep/page.maki", "deep"),
            ("docs/local.maki", "local sibling"),
            ("other/local.maki", "local elsewhere"),
            ("docs/page.maki", "page"),
            ("root.maki", "root"),
            ("unique.maki", "unique"),
            ("case/path.maki", "lower"),
            ("CASE/path.maki", "upper"),
        ];
        let snapshots = sources
            .iter()
            .map(|(path, source)| SourceSnapshot {
                path: Path::new(path),
                source,
            })
            .collect::<Vec<_>>();
        let project = analyze_project(&snapshots);
        let current = Path::new("docs/current.maki");

        let cases = [
            (
                DocumentSelector::Current,
                Expected::Found("docs/current.maki"),
            ),
            (DocumentSelector::Root("root"), Expected::Found("root.maki")),
            (
                DocumentSelector::Root("docs\\page.maki"),
                Expected::Found("docs/page.maki"),
            ),
            (
                DocumentSelector::Child("CHILD"),
                Expected::Found("docs/current/child.maki"),
            ),
            (
                DocumentSelector::Child("deep\\page.maki"),
                Expected::Found("docs/current/deep/page.maki"),
            ),
            (
                DocumentSelector::Legacy("LOCAL"),
                Expected::Found("docs/local.maki"),
            ),
            (
                DocumentSelector::Legacy("UNIQUE"),
                Expected::Found("unique.maki"),
            ),
            (
                DocumentSelector::Root("case/path"),
                Expected::Found("case/path.maki"),
            ),
            (DocumentSelector::Root("CaSe/PaTh"), Expected::Ambiguous),
            (DocumentSelector::Legacy("path"), Expected::Ambiguous),
            (DocumentSelector::Root(""), Expected::Broken),
            (DocumentSelector::Root("/root"), Expected::Broken),
            (DocumentSelector::Root("docs//page"), Expected::Broken),
            (DocumentSelector::Child("../page"), Expected::Broken),
            (DocumentSelector::Child("./page"), Expected::Broken),
            (DocumentSelector::Child("C:\\page"), Expected::Broken),
            (DocumentSelector::Root("docs/C:/page"), Expected::Broken),
            (DocumentSelector::Legacy("missing"), Expected::Broken),
        ];

        for (selector, expected) in cases {
            let actual = project.select_document(current, selector);
            match (actual, expected) {
                (DocumentSelection::Found(document), Expected::Found(path)) => {
                    assert_eq!(document.path, Path::new(path), "selector: {selector:?}");
                }
                (DocumentSelection::Broken, Expected::Broken)
                | (DocumentSelection::Ambiguous, Expected::Ambiguous) => {}
                (actual, expected) => {
                    panic!("unexpected selection for {selector:?}: {actual:?}, wanted {expected:?}")
                }
            }
        }
    }

    #[test]
    fn project_analysis_resolves_document_and_inner_selector_matrix() {
        let current = "= Coding\n--^ id: coding-id\n[[#coding-id]]\n[[@coding-id]]\n[[+child#Problems]]\n[[+child@week-1]]\n[[/root#Root]]\n[[/root@root-id]]\n[[@only-other]]\n";
        let child = "= Problems\ndetails\n--^ id: week-1\n";
        let root = "= Root\nroot details\n--^ id: root-id\n";
        let other = "other details\n--^ id: only-other\n";
        let analysis = analyze_project(&[
            SourceSnapshot {
                path: Path::new("plans/future.maki"),
                source: current,
            },
            SourceSnapshot {
                path: Path::new("plans/future/child.maki"),
                source: child,
            },
            SourceSnapshot {
                path: Path::new("root.maki"),
                source: root,
            },
            SourceSnapshot {
                path: Path::new("other.maki"),
                source: other,
            },
        ]);
        let current = analysis.document(Path::new("plans/future.maki")).unwrap();

        for (index, kind, path, fragment) in [
            (
                0,
                DefinitionTargetKind::Heading,
                "plans/future.maki",
                "coding-id",
            ),
            (
                1,
                DefinitionTargetKind::Id,
                "plans/future.maki",
                "coding-id",
            ),
            (
                2,
                DefinitionTargetKind::Heading,
                "plans/future/child.maki",
                "Problems",
            ),
            (
                3,
                DefinitionTargetKind::Id,
                "plans/future/child.maki",
                "week-1",
            ),
            (4, DefinitionTargetKind::Heading, "root.maki", "Root"),
            (5, DefinitionTargetKind::Id, "root.maki", "root-id"),
        ] {
            assert!(matches!(
                &current.note_links[index].resolution,
                Some(LinkResolution::Found(DefinitionTarget {
                    path: target_path,
                    kind: target_kind,
                    fragment: Some(target_fragment),
                    ..
                })) if target_path == Path::new(path)
                    && *target_kind == kind
                    && target_fragment == fragment
            ));
        }
        assert_eq!(
            current.note_links[6].resolution,
            Some(LinkResolution::BrokenId)
        );
    }

    #[test]
    fn project_analysis_keeps_root_and_child_coordinates_deterministic() {
        let current = "[[/only]] [[+only]] [[+../root]] [[only]]";
        let analysis = analyze_project(&[
            SourceSnapshot {
                path: Path::new("plans/future.maki"),
                source: current,
            },
            SourceSnapshot {
                path: Path::new("elsewhere/only.maki"),
                source: "only",
            },
        ]);
        let current = analysis.document(Path::new("plans/future.maki")).unwrap();

        assert_eq!(
            current.note_links[0].resolution,
            Some(LinkResolution::BrokenNote)
        );
        assert_eq!(
            current.note_links[1].resolution,
            Some(LinkResolution::BrokenNote)
        );
        assert_eq!(
            current.note_links[2].resolution,
            Some(LinkResolution::BrokenNote)
        );
        assert!(matches!(
            &current.note_links[3].resolution,
            Some(LinkResolution::Found(DefinitionTarget { path, .. }))
                if path == Path::new("elsewhere/only.maki")
        ));
    }

    #[test]
    fn project_analysis_resolves_ids_exactly_and_only_inside_the_selected_document() {
        let current = "local\n--^ id: Shared\n[[@Shared]] [[@shared]] [[/other@Shared]]";
        let other = "other\n--^ id: Shared\n";
        let analysis = analyze_project(&[
            SourceSnapshot {
                path: Path::new("current.maki"),
                source: current,
            },
            SourceSnapshot {
                path: Path::new("other.maki"),
                source: other,
            },
        ]);
        let current = analysis.document(Path::new("current.maki")).unwrap();

        assert!(matches!(
            &current.note_links[0].resolution,
            Some(LinkResolution::Found(DefinitionTarget { path, .. }))
                if path == Path::new("current.maki")
        ));
        assert_eq!(
            current.note_links[1].resolution,
            Some(LinkResolution::BrokenId)
        );
        assert!(matches!(
            &current.note_links[2].resolution,
            Some(LinkResolution::Found(DefinitionTarget { path, .. }))
                if path == Path::new("other.maki")
        ));
    }

    #[test]
    fn project_analysis_reports_ambiguous_and_broken_id_links_with_target_spans() {
        let source = "first\n--^ id: same\nsecond\n--^ id: same\n[[@same]] [[@missing]]";
        let analysis = analyze_project(&[SourceSnapshot {
            path: Path::new("index.maki"),
            source,
        }]);
        let index = analysis.document(Path::new("index.maki")).unwrap();

        assert_eq!(
            index.note_links[0].resolution,
            Some(LinkResolution::AmbiguousId)
        );
        assert_eq!(
            index.note_links[1].resolution,
            Some(LinkResolution::BrokenId)
        );
        let semantic = analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                matches!(
                    diagnostic.kind,
                    AnalysisDiagnosticKind::AmbiguousIdLink | AnalysisDiagnosticKind::BrokenIdLink
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(semantic.len(), 2);
        assert_eq!(
            &source[semantic[0].span.start..semantic[0].span.end],
            "@same"
        );
        assert_eq!(
            &source[semantic[1].span.start..semantic[1].span.end],
            "@missing"
        );
    }
}
