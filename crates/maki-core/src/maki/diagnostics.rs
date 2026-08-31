use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::{
    analysis::AnalysisDiagnosticKind,
    parser::{self, BlockKind, Inline},
    source::SourceMap,
};

use super::links::{ExternalLinkCheck, check_external_link, note_link_target_for_href};
use super::{Maki, NoteLinkResolution, NoteRef, quote_mode_is_raw};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDiagnostic {
    source_path: PathBuf,
    line: Option<usize>,
    kind: ProjectDiagnosticKind,
}

impl Maki {
    pub fn diagnostics(&self) -> Vec<ProjectDiagnostic> {
        self.diagnostics_with_external_link_checker(&check_external_link)
    }

    pub fn diagnostics_without_external_links(&self) -> Vec<ProjectDiagnostic> {
        self.collect_note_diagnostics()
    }

    pub(super) fn diagnostics_with_external_link_checker(
        &self,
        check_external_link: &dyn Fn(&str) -> ExternalLinkCheck,
    ) -> Vec<ProjectDiagnostic> {
        let mut diagnostics = self.collect_note_diagnostics();
        self.push_external_link_diagnostics(&mut diagnostics, check_external_link);
        diagnostics
    }

    fn collect_note_diagnostics(&self) -> Vec<ProjectDiagnostic> {
        let mut diagnostics = vec![];

        for note in self.notes.values() {
            let source_path = note.source_path();
            let Some(source) = self.snapshot.source(source_path) else {
                diagnostics.push(ProjectDiagnostic::new(
                    source_path,
                    None,
                    ProjectDiagnosticKind::ReadFailed,
                ));
                continue;
            };
            let parsed = parser::parse(source);

            for diagnostic in &parsed.diagnostics {
                diagnostics.push(ProjectDiagnostic::new(
                    source_path,
                    Some(diagnostic.line),
                    ProjectDiagnosticKind::ParseWarning {
                        message: parser::format_parse_diagnostic_kind(&diagnostic.kind),
                    },
                ));
            }

            if let Some(document) = self.snapshot.analysis().document(source_path) {
                let source_map = SourceMap::new(source);
                for diagnostic in document
                    .diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.kind == AnalysisDiagnosticKind::DuplicateId)
                {
                    let id = document
                        .block_ids
                        .iter()
                        .find(|block_id| block_id.value_span == diagnostic.span)
                        .map(|block_id| block_id.id.clone())
                        .unwrap_or_else(|| {
                            diagnostic
                                .message
                                .strip_prefix("duplicate id: ")
                                .unwrap_or(&diagnostic.message)
                                .to_string()
                        });
                    let line = source_map
                        .position(diagnostic.span.start)
                        .map(|position| position.line + 1);
                    diagnostics.push(ProjectDiagnostic::new(
                        source_path,
                        line,
                        ProjectDiagnosticKind::DuplicateId { id },
                    ));
                }
            }

            let current = note.note_ref();
            collect_document_link_diagnostics(
                &mut diagnostics,
                self,
                &current,
                source_path,
                &parsed.document,
            );
        }

        diagnostics
    }

    fn push_external_link_diagnostics(
        &self,
        diagnostics: &mut Vec<ProjectDiagnostic>,
        check_external_link: &dyn Fn(&str) -> ExternalLinkCheck,
    ) {
        let mut checks = BTreeMap::new();

        for external_link in &self.external_links {
            let check = checks
                .entry(external_link.target.clone())
                .or_insert_with(|| check_external_link(&external_link.target))
                .clone();

            if let ExternalLinkCheck::Broken { reason } = check {
                diagnostics.push(ProjectDiagnostic::new(
                    external_link.source_path.clone(),
                    None,
                    ProjectDiagnosticKind::BrokenExternalLink {
                        target: external_link.target.clone(),
                        reason,
                    },
                ));
            }
        }
    }
}

impl ProjectDiagnostic {
    fn new(
        source_path: impl Into<PathBuf>,
        line: Option<usize>,
        kind: ProjectDiagnosticKind,
    ) -> Self {
        Self {
            source_path: source_path.into(),
            line,
            kind,
        }
    }

    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub fn line(&self) -> Option<usize> {
        self.line
    }

    pub fn kind(&self) -> &ProjectDiagnosticKind {
        &self.kind
    }

    pub fn message(&self) -> String {
        self.kind.message()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectDiagnosticKind {
    ParseWarning { message: String },
    DuplicateId { id: String },
    BrokenLink { target: String },
    AmbiguousLink { target: String },
    BrokenExternalLink { target: String, reason: String },
    ReadFailed,
}

impl ProjectDiagnosticKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::ParseWarning { .. } => "parser",
            Self::DuplicateId { .. } => "duplicate id",
            Self::BrokenLink { .. } => "broken link",
            Self::AmbiguousLink { .. } => "ambiguous link",
            Self::BrokenExternalLink { .. } => "external link",
            Self::ReadFailed => "read",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::ParseWarning { message } => message.clone(),
            Self::DuplicateId { id } => format!("duplicate id: {id}"),
            Self::BrokenLink { target } => format!("broken link: {target}"),
            Self::AmbiguousLink { target } => format!("ambiguous link: {target}"),
            Self::BrokenExternalLink { target, reason } => {
                format!("broken external link: {target} ({reason})")
            }
            Self::ReadFailed => "failed to read note".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProjectDiagnosticSummary {
    total: usize,
    parse_warnings: usize,
    duplicate_ids: usize,
    broken_links: usize,
    ambiguous_links: usize,
    broken_external_links: usize,
    read_failures: usize,
}

impl ProjectDiagnosticSummary {
    pub fn from_diagnostics(diagnostics: &[ProjectDiagnostic]) -> Self {
        let mut summary = Self {
            total: diagnostics.len(),
            ..Default::default()
        };

        for diagnostic in diagnostics {
            match diagnostic.kind() {
                ProjectDiagnosticKind::ParseWarning { .. } => summary.parse_warnings += 1,
                ProjectDiagnosticKind::DuplicateId { .. } => summary.duplicate_ids += 1,
                ProjectDiagnosticKind::BrokenLink { .. } => summary.broken_links += 1,
                ProjectDiagnosticKind::AmbiguousLink { .. } => summary.ambiguous_links += 1,
                ProjectDiagnosticKind::BrokenExternalLink { .. } => {
                    summary.broken_external_links += 1
                }
                ProjectDiagnosticKind::ReadFailed => summary.read_failures += 1,
            }
        }

        summary
    }

    pub fn total(&self) -> usize {
        self.total
    }

    pub fn parse_warnings(&self) -> usize {
        self.parse_warnings
    }

    pub fn duplicate_ids(&self) -> usize {
        self.duplicate_ids
    }

    pub fn broken_links(&self) -> usize {
        self.broken_links
    }

    pub fn ambiguous_links(&self) -> usize {
        self.ambiguous_links
    }

    pub fn broken_external_links(&self) -> usize {
        self.broken_external_links
    }

    pub fn read_failures(&self) -> usize {
        self.read_failures
    }
}
fn collect_inline_link_diagnostics(
    diagnostics: &mut Vec<ProjectDiagnostic>,
    maki: &Maki,
    current: &NoteRef,
    source_path: &Path,
    inlines: &[Inline<'_>],
) {
    for inline in inlines {
        match inline {
            Inline::NoteLink { target } => push_link_diagnostic(
                diagnostics,
                source_path,
                maki.resolve_note_link(current, target),
                target,
            ),
            _ => {
                if let Some(body) = inline.nested_inlines() {
                    collect_inline_link_diagnostics(diagnostics, maki, current, source_path, body);
                }
            }
        }
    }
}

fn collect_table_row_link_diagnostics(
    diagnostics: &mut Vec<ProjectDiagnostic>,
    maki: &Maki,
    current: &NoteRef,
    source_path: &Path,
    row: &parser::TableRow<'_>,
) {
    if row.is_separator() {
        return;
    }

    for cell in &row.cells {
        collect_inline_link_diagnostics(diagnostics, maki, current, source_path, &cell.body);
    }
}

fn collect_block_link_diagnostics(
    diagnostics: &mut Vec<ProjectDiagnostic>,
    maki: &Maki,
    current: &NoteRef,
    source_path: &Path,
    block: &parser::Block<'_>,
    references: &parser::ReferenceDefinitions<'_>,
) {
    match &block.kind {
        BlockKind::Paragraph { body } => {
            collect_inline_link_diagnostics(diagnostics, maki, current, source_path, body)
        }
        BlockKind::Heading { body, .. } => {
            collect_inline_link_diagnostics(diagnostics, maki, current, source_path, body);
        }
        BlockKind::List { items } => {
            for item in items {
                collect_inline_link_diagnostics(
                    diagnostics,
                    maki,
                    current,
                    source_path,
                    &item.body,
                );
                for child in &item.children {
                    collect_block_link_diagnostics(
                        diagnostics,
                        maki,
                        current,
                        source_path,
                        child,
                        references,
                    );
                }
            }
        }
        BlockKind::Quote { lines } if !quote_mode_is_raw(block.property("mode")) => {
            collect_maki_lines_link_diagnostics(
                diagnostics,
                maki,
                current,
                source_path,
                lines,
                references,
            )
        }
        BlockKind::Table { header, rows, .. } => {
            collect_table_row_link_diagnostics(diagnostics, maki, current, source_path, header);
            for row in rows {
                collect_table_row_link_diagnostics(diagnostics, maki, current, source_path, row);
            }
        }
        BlockKind::Container { kind, lines, .. }
            if *kind == "quote" && !quote_mode_is_raw(block.property("mode")) =>
        {
            collect_maki_lines_link_diagnostics(
                diagnostics,
                maki,
                current,
                source_path,
                lines,
                references,
            )
        }
        BlockKind::Quote { .. }
        | BlockKind::Code { .. }
        | BlockKind::Container { .. }
        | BlockKind::ReferenceDefinition { .. } => {}
    }
}

fn collect_maki_lines_link_diagnostics(
    diagnostics: &mut Vec<ProjectDiagnostic>,
    maki: &Maki,
    current: &NoteRef,
    source_path: &Path,
    lines: &[&str],
    references: &parser::ReferenceDefinitions<'_>,
) {
    let source = lines.join("\n");
    let parsed = parser::parse_with_references(&source, references);

    collect_document_link_diagnostics(diagnostics, maki, current, source_path, &parsed.document);
}

fn collect_document_link_diagnostics(
    diagnostics: &mut Vec<ProjectDiagnostic>,
    maki: &Maki,
    current: &NoteRef,
    source_path: &Path,
    document: &parser::Document<'_>,
) {
    for definition in document.reference_definitions().iter() {
        if parser::reference_value_is_link_shaped(definition.raw_value)
            && let Some(note_target) = note_link_target_for_href(definition.raw_value)
        {
            push_link_diagnostic(
                diagnostics,
                source_path,
                maki.resolve_note_link(current, &note_target),
                &note_target,
            );
        }
        if !parser::reference_value_is_link_shaped(definition.raw_value) {
            collect_inline_link_diagnostics(
                diagnostics,
                maki,
                current,
                source_path,
                &definition.value,
            );
        }
    }

    for block in &document.blocks {
        collect_block_link_diagnostics(
            diagnostics,
            maki,
            current,
            source_path,
            block,
            document.reference_definitions(),
        );
    }
}

fn push_link_diagnostic(
    diagnostics: &mut Vec<ProjectDiagnostic>,
    source_path: &Path,
    resolution: NoteLinkResolution,
    target: &str,
) {
    match resolution {
        NoteLinkResolution::Found(_)
        | NoteLinkResolution::FoundHeading { .. }
        | NoteLinkResolution::FoundId { .. } => {}
        NoteLinkResolution::Broken => diagnostics.push(ProjectDiagnostic::new(
            source_path,
            None,
            ProjectDiagnosticKind::BrokenLink {
                target: target.to_string(),
            },
        )),
        NoteLinkResolution::Ambiguous => diagnostics.push(ProjectDiagnostic::new(
            source_path,
            None,
            ProjectDiagnosticKind::AmbiguousLink {
                target: target.to_string(),
            },
        )),
    }
}
