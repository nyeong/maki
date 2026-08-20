//! Parser for the Maki markup language.

mod build;
mod diagnostic;
mod draft;
mod inline;
mod line;
mod types;

#[cfg(test)]
mod tests;

pub use diagnostic::{ParseDiagnostic, ParseDiagnosticKind, format_parse_diagnostic_kind};
pub use inline::parse_inline;
pub use types::{
    Block, BlockKind, Date, DateRange, DateStamp, DateStampKind, Document, Inline, ListItem,
    ListKind, TableCell, TableColumnAlignment, TableRow, TableRowKind,
};

pub struct ParseResult<'a> {
    pub document: Document<'a>,
    pub diagnostics: Vec<ParseDiagnostic<'a>>,
}

pub fn parse(source: &str) -> ParseResult<'_> {
    let lines = line::scan_lines(source);
    let (drafts, diagnostics) = draft::parse_drafts(&lines);
    let document = build::build_documents(&drafts);

    ParseResult {
        document,
        diagnostics,
    }
}
