use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::{
    analysis::{
        AnalysisDiagnostic, AnalysisDiagnosticKind, AnalysisDiagnosticSubject, ProjectExternalLink,
    },
    source::SourceMap,
};

use super::Maki;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalLinkCheck {
    Ok,
    Broken { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDiagnostic {
    source_path: PathBuf,
    line: Option<usize>,
    kind: ProjectDiagnosticKind,
}

impl Maki {
    pub fn diagnostics(&self) -> Vec<ProjectDiagnostic> {
        self.collect_note_diagnostics()
    }

    pub fn diagnostics_with_external_link_checks(
        &self,
        checks: &BTreeMap<String, ExternalLinkCheck>,
    ) -> Vec<ProjectDiagnostic> {
        let mut diagnostics = self.collect_note_diagnostics();
        diagnostics.extend(external_link_diagnostics(
            self.snapshot.analysis().external_links(),
            checks,
        ));
        diagnostics
    }

    fn collect_note_diagnostics(&self) -> Vec<ProjectDiagnostic> {
        let mut diagnostics = vec![];

        let mut diagnostics_by_path = BTreeMap::<&Path, Vec<&AnalysisDiagnostic>>::new();
        for diagnostic in &self.snapshot.analysis().diagnostics {
            diagnostics_by_path
                .entry(&diagnostic.path)
                .or_default()
                .push(diagnostic);
        }

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
            let Some(source_diagnostics) = diagnostics_by_path.get(&source_path) else {
                continue;
            };
            let source_map = SourceMap::new(source);
            for &diagnostic in source_diagnostics {
                let Some(kind) = project_diagnostic_kind(diagnostic) else {
                    continue;
                };
                let line = source_map
                    .position(diagnostic.span.start)
                    .map(|position| position.line + 1);
                diagnostics.push(ProjectDiagnostic::new(source_path, line, kind));
            }
        }

        diagnostics
    }
}

pub fn external_link_diagnostics(
    external_links: &[ProjectExternalLink],
    checks: &BTreeMap<String, ExternalLinkCheck>,
) -> Vec<ProjectDiagnostic> {
    external_links
        .iter()
        .filter_map(|external_link| {
            let ExternalLinkCheck::Broken { reason } = checks.get(&external_link.target)? else {
                return None;
            };
            Some(ProjectDiagnostic::new(
                external_link.path.clone(),
                None,
                ProjectDiagnosticKind::BrokenExternalLink {
                    target: external_link.target.clone(),
                    reason: reason.clone(),
                },
            ))
        })
        .collect()
}

fn project_diagnostic_kind(diagnostic: &AnalysisDiagnostic) -> Option<ProjectDiagnosticKind> {
    match diagnostic.kind {
        AnalysisDiagnosticKind::ParseWarning => Some(ProjectDiagnosticKind::ParseWarning {
            message: diagnostic.message.clone(),
        }),
        AnalysisDiagnosticKind::DuplicateId => {
            let AnalysisDiagnosticSubject::Id(id) = &diagnostic.subject else {
                return None;
            };
            Some(ProjectDiagnosticKind::DuplicateId { id: id.clone() })
        }
        AnalysisDiagnosticKind::UnresolvedReference => {
            let AnalysisDiagnosticSubject::Reference(key) = &diagnostic.subject else {
                return None;
            };
            Some(ProjectDiagnosticKind::UnresolvedReference { key: key.clone() })
        }
        AnalysisDiagnosticKind::BrokenNoteLink
        | AnalysisDiagnosticKind::BrokenHeadingLink
        | AnalysisDiagnosticKind::BrokenIdLink => {
            let AnalysisDiagnosticSubject::Link(target) = &diagnostic.subject else {
                return None;
            };
            Some(ProjectDiagnosticKind::BrokenLink {
                target: target.clone(),
            })
        }
        AnalysisDiagnosticKind::AmbiguousNoteLink
        | AnalysisDiagnosticKind::AmbiguousHeadingLink
        | AnalysisDiagnosticKind::AmbiguousIdLink => {
            let AnalysisDiagnosticSubject::Link(target) = &diagnostic.subject else {
                return None;
            };
            Some(ProjectDiagnosticKind::AmbiguousLink {
                target: target.clone(),
            })
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
    UnresolvedReference { key: String },
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
            Self::UnresolvedReference { .. } => "unresolved reference",
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
            Self::UnresolvedReference { key } => format!("unresolved reference: {key}"),
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
    unresolved_references: usize,
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
                ProjectDiagnosticKind::UnresolvedReference { .. } => {
                    summary.unresolved_references += 1
                }
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

    pub fn unresolved_references(&self) -> usize {
        self.unresolved_references
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
