use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{MAIN_SEPARATOR, Path, PathBuf};

use maki_core::analysis::{
    AnalysisDiagnostic, AnalysisDiagnosticSeverity, AnalysisDiagnosticSubject, DocumentAnalysis,
    ValidationReport, ValidationSummary,
};
use maki_core::source::{SourceMap, SourceSpan};
use maki_core::{Error as MakiError, Maki, MakiConfig};
use maki_fs::{
    find_project_root, is_discoverable_maki_path, load_project_config, load_project_with_config,
};

use crate::cli::CheckFormat;

const JSON_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckOutcome {
    Clean,
    Findings,
    Incomplete,
}

#[derive(Debug)]
pub(crate) enum Error {
    Project(MakiError),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    InvalidTarget {
        path: PathBuf,
        message: &'static str,
    },
    InvalidDiagnosticRange {
        path: PathBuf,
        span: SourceSpan,
    },
    NonUtf8Path {
        path: PathBuf,
    },
    Json(serde_json::Error),
}

impl From<MakiError> for Error {
    fn from(source: MakiError) -> Self {
        Self::Project(source)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Project(error) => error.fmt(formatter),
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} {}: {source}",
                path.display()
            ),
            Self::InvalidTarget { path, message } => {
                write!(
                    formatter,
                    "invalid check target {}: {message}",
                    path.display()
                )
            }
            Self::InvalidDiagnosticRange { path, span } => write!(
                formatter,
                "invalid diagnostic range for {}: {}..{}",
                path.display(),
                span.start,
                span.end
            ),
            Self::NonUtf8Path { path } => {
                write!(
                    formatter,
                    "cannot report non-UTF-8 path: {}",
                    path.display()
                )
            }
            Self::Json(error) => write!(formatter, "failed to encode check report: {error}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReportPosition {
    byte: usize,
    line: usize,
    column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReportRange {
    start: ReportPosition,
    end: ReportPosition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReportSubject {
    kind: &'static str,
    value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReportRelatedLocation {
    path: String,
    message: String,
    range: ReportRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReportDiagnostic {
    path: String,
    severity: AnalysisDiagnosticSeverity,
    code: &'static str,
    message: String,
    subject: Option<ReportSubject>,
    range: ReportRange,
    related: Vec<ReportRelatedLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CheckReport {
    complete: bool,
    diagnostics: Vec<ReportDiagnostic>,
    summary: ValidationSummary,
    unavailable_sources: Vec<PathBuf>,
}

impl CheckReport {
    fn outcome(&self) -> CheckOutcome {
        if !self.complete {
            CheckOutcome::Incomplete
        } else if self.diagnostics.is_empty() {
            CheckOutcome::Clean
        } else {
            CheckOutcome::Findings
        }
    }
}

pub(crate) fn run(target: &Path, format: CheckFormat) -> Result<CheckOutcome, Error> {
    let metadata = fs::metadata(target).map_err(|source| io_error("inspect", target, source))?;
    let report = if metadata.is_file() {
        check_file(target)?
    } else if metadata.is_dir() {
        check_directory(target)?
    } else {
        return Err(Error::InvalidTarget {
            path: target.to_path_buf(),
            message: "expected a regular .maki file or directory",
        });
    };

    write_report(&report, format)?;
    for path in &report.unavailable_sources {
        eprintln!("failed to read Maki source: {}", best_effort_path(path));
    }
    Ok(report.outcome())
}

fn check_file(target: &Path) -> Result<CheckReport, Error> {
    if target.extension() != Some(OsStr::new("maki")) {
        return Err(Error::InvalidTarget {
            path: target.to_path_buf(),
            message: "expected a .maki file",
        });
    }

    let (canonical_parent, entry_path) = canonical_entry_path(target)?;
    if let Some(project_root) = find_project_root(&canonical_parent)? {
        let config = load_project_config(&project_root)?;
        let source_root = config.project_source_root(&project_root);
        if let Ok(canonical_source_root) = fs::canonicalize(&source_root)
            && let Ok(selected) = entry_path.strip_prefix(&canonical_source_root)
            && is_discoverable_maki_path(selected)
        {
            let maki = load_project_with_config(&source_root, config)?;
            return report_project(&maki, Some(selected));
        }
    }

    let source =
        fs::read_to_string(&entry_path).map_err(|source| io_error("read", target, source))?;
    let analysis = maki_core::analysis::analyze_document(target, &source);
    report_document(&analysis, &source)
}

fn check_directory(target: &Path) -> Result<CheckReport, Error> {
    let canonical_target = canonicalize(target)?;
    if let Some(project_root) = find_project_root(&canonical_target)? {
        let config = load_project_config(&project_root)?;
        let source_root = config.project_source_root(&project_root);
        let in_project_source = fs::canonicalize(&source_root)
            .is_ok_and(|canonical_source_root| canonical_target.starts_with(canonical_source_root));
        if canonical_target == project_root || in_project_source {
            let maki = load_project_with_config(&source_root, config)?;
            return report_project(&maki, None);
        }
    }

    let maki = load_project_with_config(&canonical_target, MakiConfig::default())?;
    report_project(&maki, None)
}

fn report_project(maki: &Maki, selected: Option<&Path>) -> Result<CheckReport, Error> {
    collect_report(maki.validation_report(), selected, |path| maki.source(path))
}

fn report_document(analysis: &DocumentAnalysis, source: &str) -> Result<CheckReport, Error> {
    collect_report(analysis.validation_report(), None, |_path| Some(source))
}

fn collect_report<'a>(
    validation: ValidationReport<'a>,
    selected: Option<&Path>,
    source_for_path: impl Fn(&Path) -> Option<&'a str>,
) -> Result<CheckReport, Error> {
    let mut output = validation
        .diagnostics()
        .iter()
        .filter(|diagnostic| selected.is_none_or(|selected| diagnostic.source_path() == selected))
        .map(|diagnostic| report_diagnostic(diagnostic, &source_for_path))
        .collect::<Result<Vec<_>, _>>()?;

    output.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.range.start.byte.cmp(&right.range.start.byte))
            .then_with(|| left.range.end.byte.cmp(&right.range.end.byte))
            .then_with(|| left.code.cmp(right.code))
            .then_with(|| left.message.cmp(&right.message))
    });
    let summary =
        ValidationSummary::from_severities(output.iter().map(|diagnostic| diagnostic.severity));

    Ok(CheckReport {
        complete: validation.is_complete(),
        diagnostics: output,
        summary,
        unavailable_sources: validation.unavailable_sources().to_vec(),
    })
}

fn report_diagnostic<'a>(
    diagnostic: &AnalysisDiagnostic,
    source_for_path: &impl Fn(&Path) -> Option<&'a str>,
) -> Result<ReportDiagnostic, Error> {
    let path = diagnostic.source_path();
    let source = source_for_path(path).ok_or_else(|| Error::InvalidDiagnosticRange {
        path: path.to_path_buf(),
        span: diagnostic.span(),
    })?;
    let related = diagnostic
        .related()
        .iter()
        .map(|location| {
            let related_source =
                source_for_path(&location.path).ok_or_else(|| Error::InvalidDiagnosticRange {
                    path: location.path.clone(),
                    span: location.span,
                })?;
            Ok(ReportRelatedLocation {
                path: normalize_path(&location.path)?,
                message: location.message.clone(),
                range: report_range(related_source, &location.path, location.span)?,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;

    Ok(ReportDiagnostic {
        path: normalize_path(path)?,
        severity: diagnostic.severity(),
        code: diagnostic.code(),
        message: diagnostic.message().to_string(),
        subject: report_subject(diagnostic.subject()),
        range: report_range(source, path, diagnostic.span())?,
        related,
    })
}

fn report_subject(subject: &AnalysisDiagnosticSubject) -> Option<ReportSubject> {
    let (kind, value) = match subject {
        AnalysisDiagnosticSubject::None => return None,
        AnalysisDiagnosticSubject::Id(value) => ("id", value),
        AnalysisDiagnosticSubject::Reference(value) => ("reference", value),
        AnalysisDiagnosticSubject::Link(value) => ("link", value),
    };
    Some(ReportSubject {
        kind,
        value: value.clone(),
    })
}

fn report_range(source: &str, path: &Path, span: SourceSpan) -> Result<ReportRange, Error> {
    let source_map = SourceMap::new(source);
    let position = |offset| {
        source_map
            .scalar_position(offset)
            .ok_or_else(|| Error::InvalidDiagnosticRange {
                path: path.to_path_buf(),
                span,
            })
    };
    let start = position(span.start)?;
    let end = position(span.end)?;

    Ok(ReportRange {
        start: ReportPosition {
            byte: span.start,
            line: start.line + 1,
            column: start.column + 1,
        },
        end: ReportPosition {
            byte: span.end,
            line: end.line + 1,
            column: end.column + 1,
        },
    })
}

fn write_report(report: &CheckReport, format: CheckFormat) -> Result<(), Error> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    match format {
        CheckFormat::Text => {
            for diagnostic in &report.diagnostics {
                writeln!(
                    stdout,
                    "{}:{}:{}: {}[{}]: {}",
                    diagnostic.path,
                    diagnostic.range.start.line,
                    diagnostic.range.start.column,
                    severity_label(diagnostic.severity),
                    diagnostic.code,
                    single_line(&diagnostic.message),
                )
                .map_err(|source| io_error("write", Path::new("<stdout>"), source))?;
            }
        }
        CheckFormat::Json => {
            serde_json::to_writer(&mut stdout, &report_json(report)).map_err(Error::Json)?;
            writeln!(stdout).map_err(|source| io_error("write", Path::new("<stdout>"), source))?;
        }
    }
    stdout
        .flush()
        .map_err(|source| io_error("flush", Path::new("<stdout>"), source))
}

fn report_json(report: &CheckReport) -> serde_json::Value {
    let diagnostics = report
        .diagnostics
        .iter()
        .map(|diagnostic| {
            serde_json::json!({
                "path": diagnostic.path,
                "severity": severity_label(diagnostic.severity),
                "code": diagnostic.code,
                "message": diagnostic.message,
                "subject": diagnostic.subject.as_ref().map(|subject| serde_json::json!({
                    "kind": subject.kind,
                    "value": subject.value,
                })),
                "range": range_json(&diagnostic.range),
                "related": diagnostic.related.iter().map(|location| serde_json::json!({
                    "path": location.path,
                    "message": location.message,
                    "range": range_json(&location.range),
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "schema_version": JSON_SCHEMA_VERSION,
        "complete": report.complete,
        "diagnostics": diagnostics,
        "summary": {
            "total": report.summary.total(),
            "errors": report.summary.errors(),
            "warnings": report.summary.warnings(),
            "information": report.summary.information(),
            "hints": report.summary.hints(),
        },
    })
}

fn range_json(range: &ReportRange) -> serde_json::Value {
    serde_json::json!({
        "start": {
            "byte": range.start.byte,
            "line": range.start.line,
            "column": range.start.column,
        },
        "end": {
            "byte": range.end.byte,
            "line": range.end.line,
            "column": range.end.column,
        },
    })
}

fn severity_label(severity: AnalysisDiagnosticSeverity) -> &'static str {
    match severity {
        AnalysisDiagnosticSeverity::Error => "error",
        AnalysisDiagnosticSeverity::Warning => "warning",
        AnalysisDiagnosticSeverity::Information => "information",
        AnalysisDiagnosticSeverity::Hint => "hint",
    }
}

fn normalize_path(path: &Path) -> Result<String, Error> {
    let path = path.to_str().ok_or_else(|| Error::NonUtf8Path {
        path: path.to_path_buf(),
    })?;
    Ok(path.replace(MAIN_SEPARATOR, "/"))
}

fn best_effort_path(path: &Path) -> String {
    path.to_string_lossy().replace(MAIN_SEPARATOR, "/")
}

fn single_line(message: &str) -> String {
    message
        .chars()
        .map(|character| match character {
            '\r' | '\n' => ' ',
            character => character,
        })
        .collect()
}

fn canonicalize(path: &Path) -> Result<PathBuf, Error> {
    fs::canonicalize(path).map_err(|source| io_error("resolve", path, source))
}

fn canonical_entry_path(path: &Path) -> Result<(PathBuf, PathBuf), Error> {
    let file_name = path.file_name().ok_or_else(|| Error::InvalidTarget {
        path: path.to_path_buf(),
        message: "expected a file name",
    })?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let canonical_parent = canonicalize(parent)?;
    let entry_path = canonical_parent.join(file_name);
    Ok((canonical_parent, entry_path))
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> Error {
    Error::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}
