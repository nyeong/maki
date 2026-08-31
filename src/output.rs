use std::path::Path;

use maki_core::{ProjectDiagnostic, ProjectDiagnosticSummary, parser};

pub(crate) fn emit_parse_warnings(file: &Path, diagnostics: &[parser::ParseDiagnostic<'_>]) {
    for diagnostic in diagnostics {
        eprintln!("{}", format_parse_warning(file, diagnostic));
    }
}

pub(crate) fn format_parse_warning(
    file: &Path,
    diagnostic: &parser::ParseDiagnostic<'_>,
) -> String {
    format!(
        "warning: {}:{}: {}",
        file.display(),
        diagnostic.line,
        format_parse_warning_kind(&diagnostic.kind)
    )
}

fn format_parse_warning_kind(kind: &parser::ParseDiagnosticKind<'_>) -> String {
    parser::format_parse_diagnostic_kind(kind)
}

pub(crate) fn emit_project_diagnostic_summary(diagnostics: &[ProjectDiagnostic]) {
    if diagnostics.is_empty() {
        return;
    }

    eprintln!("{}", format_project_diagnostic_summary(diagnostics));
    for diagnostic in diagnostics {
        eprintln!("{}", format_project_diagnostic(diagnostic));
    }
}

fn format_project_diagnostic_summary(diagnostics: &[ProjectDiagnostic]) -> String {
    let summary = ProjectDiagnosticSummary::from_diagnostics(diagnostics);

    format!(
        "diagnostics: {} issue(s): {} duplicate id(s), {} unresolved reference(s), {} broken link(s), {} ambiguous link(s), {} broken external link(s), {} parser warning(s), {} read failure(s)",
        summary.total(),
        summary.duplicate_ids(),
        summary.unresolved_references(),
        summary.broken_links(),
        summary.ambiguous_links(),
        summary.broken_external_links(),
        summary.parse_warnings(),
        summary.read_failures()
    )
}

fn format_project_diagnostic(diagnostic: &ProjectDiagnostic) -> String {
    let mut location = diagnostic.source_path().display().to_string();
    if let Some(line) = diagnostic.line() {
        location.push(':');
        location.push_str(&line.to_string());
    }

    format!("warning: {}: {}", location, diagnostic.message())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_parse_warning() {
        let diagnostic = parser::ParseDiagnostic {
            line: 3,
            span: maki_core::source::SourceSpan::new(0, 20),
            kind: parser::ParseDiagnosticKind::InvalidProperty {
                raw_line: "--^ invalid-property",
            },
        };

        assert_eq!(
            format_parse_warning(Path::new("docs/example.maki"), &diagnostic),
            "warning: docs/example.maki:3: invalid property: --^ invalid-property"
        );
    }
}
