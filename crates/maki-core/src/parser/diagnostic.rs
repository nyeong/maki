use crate::source::SourceSpan;

#[derive(Debug, PartialEq)]
pub struct ParseDiagnostic<'a> {
    pub line: usize,
    pub span: SourceSpan,
    pub kind: ParseDiagnosticKind<'a>,
}

#[derive(Debug, PartialEq)]
pub enum ParseDiagnosticKind<'a> {
    InvalidProperty { raw_line: &'a str },
    UnclosedContainer { raw_line: &'a str },
    PropertyOnProperty { raw_line: &'a str },
    DuplicateReferenceDefinition { raw_line: &'a str },
}

pub fn format_parse_diagnostic_kind(kind: &ParseDiagnosticKind<'_>) -> String {
    match kind {
        ParseDiagnosticKind::InvalidProperty { raw_line } => {
            format!("invalid property: {raw_line}")
        }
        ParseDiagnosticKind::UnclosedContainer { raw_line } => {
            format!("unclosed container block: {raw_line}")
        }
        ParseDiagnosticKind::PropertyOnProperty { raw_line } => {
            format!("property on property: {raw_line}")
        }
        ParseDiagnosticKind::DuplicateReferenceDefinition { raw_line } => {
            format!("duplicate reference definition: {raw_line}")
        }
    }
}
