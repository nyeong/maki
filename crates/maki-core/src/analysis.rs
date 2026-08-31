use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use crate::link_target::{DocumentSelector, InnerSelector, NoteLinkTarget};
use crate::parser::{self, Block, BlockKind, DateStamp, DateStampKind, Inline};
use crate::source::{SourceMap, SourceSpan};

#[derive(Debug, Clone, Copy)]
pub struct SourceSnapshot<'a> {
    pub path: &'a Path,
    pub source: &'a str,
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
    pub reference_links: Vec<ReferenceLinkOccurrence>,
    pub properties: Vec<PropertyOccurrence>,
    pub dates: Vec<DateOccurrence>,
    pub diagnostics: Vec<AnalysisDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectAnalysis {
    pub documents: BTreeMap<PathBuf, DocumentAnalysis>,
    pub diagnostics: Vec<AnalysisDiagnostic>,
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
    pub span: SourceSpan,
    pub target_span: SourceSpan,
    pub resolution: Option<LinkResolution>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceLinkOccurrence {
    pub title: String,
    pub target: String,
    pub span: SourceSpan,
    pub title_span: SourceSpan,
}

#[derive(Default)]
struct DocumentOccurrences {
    blocks: Vec<BlockOccurrence>,
    block_ids: Vec<BlockIdOccurrence>,
    headings: Vec<HeadingOccurrence>,
    note_links: Vec<NoteLinkOccurrence>,
    reference_links: Vec<ReferenceLinkOccurrence>,
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
    pub key: String,
    pub value: String,
    pub span: SourceSpan,
    pub key_span: SourceSpan,
    pub value_span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateOrigin {
    VisibleInline,
    PropertyValue { key: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateOccurrence {
    pub kind: DateStampKind,
    pub body: String,
    pub origin: DateOrigin,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisDiagnostic {
    pub path: PathBuf,
    pub span: SourceSpan,
    pub kind: AnalysisDiagnosticKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisDiagnosticKind {
    ParseWarning,
    DuplicateId,
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
    pub selection_span: SourceSpan,
    pub kind: DefinitionTargetKind,
    pub fragment: Option<String>,
}

pub fn analyze_document(path: &Path, source: &str) -> DocumentAnalysis {
    let parsed = parser::parse(source);
    let source_map = SourceMap::new(source);
    let properties = collect_properties(source, &source_map);
    let document_span = parsed
        .document
        .title()
        .and_then(|title| slice_span(source, title))
        .unwrap_or_default();
    let title = parsed
        .document
        .title()
        .map(str::to_owned)
        .unwrap_or_else(|| file_stem(path));
    let mut occurrences = DocumentOccurrences::default();

    for block in &parsed.document.blocks {
        collect_block(source, &source_map, block, &mut occurrences);
    }
    for definition in parsed.document.reference_definitions().iter() {
        if let parser::ReferenceDefinition::Footnote { body, .. } = definition {
            collect_inlines(source, body, DateOrigin::VisibleInline, &mut occurrences);
        }
    }
    for property in &properties {
        let value_source = &source[property.value_span.start..property.value_span.end];
        let parsed_value = parser::parse_inline(value_source);
        collect_inlines(
            source,
            &parsed_value,
            DateOrigin::PropertyValue {
                key: property.key.clone(),
            },
            &mut occurrences,
        );
    }
    occurrences.blocks.sort_by_key(|block| block.span);
    occurrences
        .block_ids
        .sort_by_key(|block_id| block_id.value_span);

    let mut diagnostics = parsed
        .diagnostics
        .iter()
        .map(|diagnostic| AnalysisDiagnostic {
            path: path.to_path_buf(),
            span: diagnostic.span,
            kind: AnalysisDiagnosticKind::ParseWarning,
            message: parser::format_parse_diagnostic_kind(&diagnostic.kind),
        })
        .collect::<Vec<_>>();
    diagnostics.extend(duplicate_id_diagnostics(
        path,
        &occurrences.block_ids,
        &occurrences.headings,
    ));
    diagnostics.sort_by_key(|diagnostic| diagnostic.span);

    DocumentAnalysis {
        path: path.to_path_buf(),
        canonical_path: canonical_path(path),
        title,
        document_span,
        blocks: occurrences.blocks,
        block_ids: occurrences.block_ids,
        headings: occurrences.headings,
        note_links: occurrences.note_links,
        reference_links: occurrences.reference_links,
        properties,
        dates: occurrences.dates,
        diagnostics,
    }
}

pub fn analyze_project(snapshots: &[SourceSnapshot<'_>]) -> ProjectAnalysis {
    let mut documents = snapshots
        .iter()
        .map(|snapshot| {
            let document = analyze_document(snapshot.path, snapshot.source);
            (document.path.clone(), document)
        })
        .collect::<BTreeMap<_, _>>();
    let lookup = ProjectLookup::new(&documents);
    let mut semantic_diagnostics = Vec::new();

    for document in documents.values_mut() {
        let current_path = document.path.clone();
        for occurrence in &mut document.note_links {
            let resolution = lookup.resolve(&current_path, &occurrence.target);
            if let Some((kind, message)) =
                diagnostic_for_resolution(&occurrence.target, &resolution)
            {
                semantic_diagnostics.push(AnalysisDiagnostic {
                    path: document.path.clone(),
                    span: occurrence.target_span,
                    kind,
                    message,
                });
            }
            occurrence.resolution = Some(resolution);
        }
    }

    let mut diagnostics = documents
        .values()
        .flat_map(|document| document.diagnostics.iter().cloned())
        .collect::<Vec<_>>();
    diagnostics.extend(semantic_diagnostics);
    diagnostics.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.span.cmp(&right.span))
    });

    ProjectAnalysis {
        documents,
        diagnostics,
    }
}

impl ProjectAnalysis {
    pub fn document(&self, path: &Path) -> Option<&DocumentAnalysis> {
        self.documents.get(path)
    }

    pub fn note_candidates(&self) -> impl Iterator<Item = &DocumentAnalysis> {
        self.documents.values()
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

fn collect_block(
    source: &str,
    source_map: &SourceMap<'_>,
    block: &Block<'_>,
    occurrences: &mut DocumentOccurrences,
) {
    let mut body_spans = Vec::new();
    let kind = match &block.kind {
        BlockKind::Paragraph { body } => {
            collect_inlines(source, body, DateOrigin::VisibleInline, occurrences);
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
            collect_inlines(source, body, DateOrigin::VisibleInline, occurrences);
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
                collect_inlines(source, &item.body, DateOrigin::VisibleInline, occurrences);
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
                    collect_inlines(source, &cell.body, DateOrigin::VisibleInline, occurrences);
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
                match definition {
                    parser::ReferenceDefinition::Link { title, target } => {
                        body_spans.extend(slice_span(source, title));
                        body_spans.extend(slice_span(source, target));
                    }
                    parser::ReferenceDefinition::Footnote {
                        label,
                        body,
                        raw_body,
                    } => {
                        body_spans.extend(slice_span(source, label));
                        body_spans.extend(slice_span(source, raw_body));
                        collect_inlines(source, body, DateOrigin::VisibleInline, occurrences);
                    }
                }
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

fn collect_inlines(
    source: &str,
    inlines: &[Inline<'_>],
    origin: DateOrigin,
    occurrences: &mut DocumentOccurrences,
) {
    for inline in inlines {
        match inline {
            Inline::NoteLink { target } => {
                if let Some(target_span) = slice_span(source, target) {
                    occurrences.note_links.push(NoteLinkOccurrence {
                        target: (*target).to_string(),
                        span: SourceSpan::new(
                            target_span.start.saturating_sub(2),
                            (target_span.end + 2).min(source.len()),
                        ),
                        target_span,
                        resolution: None,
                    });
                }
            }
            Inline::Link { title, target } => {
                if let Some(title_span) = slice_span(source, title) {
                    occurrences.reference_links.push(ReferenceLinkOccurrence {
                        title: (*title).to_string(),
                        target: target.trim().to_string(),
                        span: SourceSpan::new(
                            title_span.start.saturating_sub(1),
                            (title_span.end + 1).min(source.len()),
                        ),
                        title_span,
                    });
                }
            }
            Inline::DateStamp(stamp) => {
                collect_date(source, *stamp, origin.clone(), &mut occurrences.dates)
            }
            Inline::DateRange(range) => {
                for stamp in [range.start(), range.end()] {
                    collect_date(source, stamp, origin.clone(), &mut occurrences.dates);
                }
            }
            _ => {
                if let Some(children) = inline.nested_inlines() {
                    collect_inlines(source, children, origin.clone(), occurrences);
                }
            }
        }
    }
}

fn collect_date(
    source: &str,
    stamp: DateStamp<'_>,
    origin: DateOrigin,
    dates: &mut Vec<DateOccurrence>,
) {
    let Some(body_span) = slice_span(source, stamp.body()) else {
        return;
    };
    dates.push(DateOccurrence {
        kind: stamp.kind(),
        body: stamp.body().to_string(),
        origin,
        span: SourceSpan::new(
            body_span.start.saturating_sub(1),
            (body_span.end + 1).min(source.len()),
        ),
    });
}

fn collect_inline_source_spans(source: &str, inlines: &[Inline<'_>], spans: &mut Vec<SourceSpan>) {
    for inline in inlines {
        let slice = match inline {
            Inline::NoteLink { target }
            | Inline::HyperLink { target }
            | Inline::Text(target)
            | Inline::Code(target)
            | Inline::Superscript(target)
            | Inline::Subscript(target)
            | Inline::Insertion(target)
            | Inline::Deletion(target) => Some(*target),
            Inline::Link { title, .. } => Some(*title),
            Inline::Footnote { label } => Some(*label),
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

fn collect_properties(source: &str, source_map: &SourceMap<'_>) -> Vec<PropertyOccurrence> {
    (0..source_map.line_count())
        .filter_map(|line| {
            let span = source_map.line_span(line)?;
            let raw_line = &source[span.start..span.end];
            let indent = raw_line.len() - raw_line.trim_start_matches(' ').len();
            let content = &raw_line[indent..];
            let (direction, body) = if let Some(body) = content.strip_prefix("--^ ") {
                (PropertyDirection::Previous, body)
            } else if let Some(body) = content.strip_prefix("--v ") {
                (PropertyDirection::Next, body)
            } else {
                return None;
            };
            let (raw_key, raw_value) = body.split_once(':')?;
            let body_start = span.start + indent + "--^ ".len();
            let key_span = trimmed_subspan(raw_key, body_start);
            let value_start = body_start + raw_key.len() + ':'.len_utf8();
            let value_span = trimmed_subspan(raw_value, value_start);

            Some(PropertyOccurrence {
                direction,
                key: source[key_span.start..key_span.end].to_string(),
                value: source[value_span.start..value_span.end].to_string(),
                span,
                key_span,
                value_span,
            })
        })
        .collect()
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

fn trimmed_subspan(source: &str, absolute_start: usize) -> SourceSpan {
    let leading = source.len() - source.trim_start().len();
    let trimmed = source.trim();
    let start = absolute_start + leading;
    SourceSpan::new(start, start + trimmed.len())
}

fn slice_span(source: &str, slice: &str) -> Option<SourceSpan> {
    let source_start = source.as_ptr() as usize;
    let source_end = source_start + source.len();
    let slice_start = slice.as_ptr() as usize;
    let slice_end = slice_start + slice.len();

    (source_start <= slice_start && slice_end <= source_end)
        .then(|| SourceSpan::new(slice_start - source_start, slice_end - source_start))
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

struct ProjectLookup {
    by_exact: BTreeMap<String, PathBuf>,
    by_canonical: BTreeMap<String, Vec<PathBuf>>,
    by_stem: BTreeMap<String, Vec<PathBuf>>,
    headings: BTreeMap<PathBuf, Vec<HeadingOccurrence>>,
    block_ids: BTreeMap<PathBuf, Vec<BlockIdOccurrence>>,
    document_spans: BTreeMap<PathBuf, SourceSpan>,
}

impl ProjectLookup {
    fn new(documents: &BTreeMap<PathBuf, DocumentAnalysis>) -> Self {
        let mut by_exact = BTreeMap::new();
        let mut by_canonical: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        let mut by_stem: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        let mut headings = BTreeMap::new();
        let mut block_ids = BTreeMap::new();
        let mut document_spans = BTreeMap::new();

        for document in documents.values() {
            by_exact.insert(document.canonical_path.clone(), document.path.clone());
            by_canonical
                .entry(normalize_key(&document.canonical_path))
                .or_default()
                .push(document.path.clone());
            by_stem
                .entry(normalize_key(&file_stem(&document.path)))
                .or_default()
                .push(document.path.clone());
            headings.insert(document.path.clone(), document.headings.clone());
            block_ids.insert(document.path.clone(), document.block_ids.clone());
            document_spans.insert(document.path.clone(), document.document_span);
        }

        Self {
            by_exact,
            by_canonical,
            by_stem,
            headings,
            block_ids,
            document_spans,
        }
    }

    fn resolve(&self, current_path: &Path, target: &str) -> LinkResolution {
        let target = NoteLinkTarget::parse(target);
        let note = match self.resolve_document(current_path, target.document) {
            CandidateResolution::Found(path) => path,
            CandidateResolution::Broken => return LinkResolution::BrokenNote,
            CandidateResolution::Ambiguous => return LinkResolution::AmbiguousNote,
        };

        match target.inner {
            None => LinkResolution::Found(DefinitionTarget {
                selection_span: self.document_spans.get(&note).copied().unwrap_or_default(),
                path: note,
                kind: DefinitionTargetKind::Document,
                fragment: None,
            }),
            Some(InnerSelector::Heading(heading)) => self.resolve_heading(note, heading),
            Some(InnerSelector::Id(id)) => self.resolve_id(note, id),
        }
    }

    fn resolve_document(
        &self,
        current_path: &Path,
        selector: DocumentSelector<'_>,
    ) -> CandidateResolution {
        match selector {
            DocumentSelector::Current => CandidateResolution::Found(current_path.to_path_buf()),
            DocumentSelector::Legacy(target) => self.resolve_legacy_note(current_path, target),
            DocumentSelector::Root(target) => self.resolve_coordinate(target),
            DocumentSelector::Child(target) => self.resolve_child(current_path, target),
        }
    }

    fn resolve_heading(&self, note: PathBuf, heading_target: &str) -> LinkResolution {
        if heading_target.is_empty() {
            return LinkResolution::BrokenHeading;
        }

        let headings = self.headings.get(&note).map(Vec::as_slice).unwrap_or(&[]);
        let mut matches = headings
            .iter()
            .filter(|heading| heading.anchor == heading_target)
            .collect::<Vec<_>>();
        if matches.is_empty() {
            let normalized = normalize_key(heading_target);
            matches = headings
                .iter()
                .filter(|heading| normalize_key(&heading.anchor) == normalized)
                .collect();
        }

        match matches.as_slice() {
            [heading]
                if self.block_ids.get(&note).is_some_and(|block_ids| {
                    block_ids
                        .iter()
                        .any(|block_id| block_id_conflicts_with_heading(block_id, heading))
                }) =>
            {
                LinkResolution::AmbiguousHeading
            }
            [heading] => LinkResolution::Found(DefinitionTarget {
                path: note,
                selection_span: heading.title_span,
                kind: DefinitionTargetKind::Heading,
                fragment: Some(heading.anchor.clone()),
            }),
            [] => LinkResolution::BrokenHeading,
            _ => LinkResolution::AmbiguousHeading,
        }
    }

    fn resolve_id(&self, note: PathBuf, id_target: &str) -> LinkResolution {
        if id_target.is_empty() {
            return LinkResolution::BrokenId;
        }

        let ids = self.block_ids.get(&note).map(Vec::as_slice).unwrap_or(&[]);
        let matches = ids
            .iter()
            .filter(|block_id| block_id.id == id_target)
            .collect::<Vec<_>>();

        match matches.as_slice() {
            [block_id]
                if self.headings.get(&note).is_some_and(|headings| {
                    headings
                        .iter()
                        .any(|heading| block_id_conflicts_with_heading(block_id, heading))
                }) =>
            {
                LinkResolution::AmbiguousId
            }
            [block_id] => LinkResolution::Found(DefinitionTarget {
                path: note,
                selection_span: block_id.value_span,
                kind: DefinitionTargetKind::Id,
                fragment: Some(block_id.id.clone()),
            }),
            [] => LinkResolution::BrokenId,
            _ => LinkResolution::AmbiguousId,
        }
    }

    fn resolve_child(&self, current_path: &Path, target: &str) -> CandidateResolution {
        let normalized = normalize_document_target(target);
        if !is_normal_relative_target(normalized) {
            return CandidateResolution::Broken;
        }

        let child = Path::new(&canonical_path(current_path)).join(normalized);
        self.resolve_coordinate(&child.to_string_lossy())
    }

    fn resolve_coordinate(&self, target: &str) -> CandidateResolution {
        let normalized = normalize_document_target(target);
        if normalized.is_empty() {
            return CandidateResolution::Broken;
        }

        if let Some(path) = self.by_exact.get(normalized) {
            return CandidateResolution::Found(path.clone());
        }
        if let Some(paths) = self.by_canonical.get(&normalize_key(normalized)) {
            return resolve_paths(paths);
        }

        CandidateResolution::Broken
    }

    fn resolve_legacy_note(&self, current_path: &Path, target: &str) -> CandidateResolution {
        let normalized = normalize_document_target(target);

        if let Some(path) = self.by_exact.get(normalized) {
            return CandidateResolution::Found(path.clone());
        }
        if normalized.contains('/') {
            return self
                .by_canonical
                .get(&normalize_key(normalized))
                .map_or(CandidateResolution::Broken, |paths| resolve_paths(paths));
        }

        let sibling = current_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(format!("{normalized}.maki"));
        if let Some(paths) = self
            .by_canonical
            .get(&normalize_key(&canonical_path(&sibling)))
        {
            return resolve_paths(paths);
        }
        self.by_stem
            .get(&normalize_key(normalized))
            .map_or(CandidateResolution::Broken, |paths| resolve_paths(paths))
    }
}

fn normalize_document_target(target: &str) -> &str {
    target.strip_suffix(".maki").unwrap_or(target)
}

fn is_normal_relative_target(target: &str) -> bool {
    !target.is_empty()
        && target
            .split('/')
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
        && Path::new(target)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CandidateResolution {
    Found(PathBuf),
    Broken,
    Ambiguous,
}

fn resolve_paths(paths: &[PathBuf]) -> CandidateResolution {
    match paths {
        [path] => CandidateResolution::Found(path.clone()),
        [] => CandidateResolution::Broken,
        _ => CandidateResolution::Ambiguous,
    }
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
    fn slice_span_rejects_an_unrelated_empty_slice_without_panicking() {
        let source = String::from("source");

        assert_eq!(slice_span(&source, ""), None);
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
        let analysis = analyze_document(Path::new("docs/current.maki"), source);

        assert_eq!(analysis.title, "문서");
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
    fn document_analysis_handles_whitespace_only_property_fields() {
        let source = "--^ title: \n--v   : value\n";

        let analysis = analyze_document(Path::new("index.maki"), source);

        assert_eq!(analysis.properties.len(), 2);
        assert_eq!(analysis.properties[0].key, "title");
        assert_eq!(analysis.properties[0].value, "");
        assert_eq!(analysis.properties[0].value_span, SourceSpan::new(11, 11));
        assert_eq!(analysis.properties[1].key, "");
        assert_eq!(analysis.properties[1].key_span, SourceSpan::new(18, 18));
        assert_eq!(analysis.properties[1].value, "value");
    }

    #[test]
    fn document_analysis_locates_reference_link_markers() {
        let source = "- [김치]\n\n[김치]: https://hakkeido.com/\n";
        let analysis = analyze_document(Path::new("index.maki"), source);

        assert_eq!(analysis.reference_links.len(), 1);
        let reference = &analysis.reference_links[0];
        assert_eq!(reference.title, "김치");
        assert_eq!(reference.target, "https://hakkeido.com/");
        assert_eq!(&source[reference.span.start..reference.span.end], "[김치]");
        assert_eq!(
            &source[reference.title_span.start..reference.title_span.end],
            "김치"
        );
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
