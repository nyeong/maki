//! Parser for the Maki markup language.

#[cfg(test)]
use std::cell::Cell;

mod build;
mod diagnostic;
mod draft;
mod inline;
mod line;
mod types;

#[cfg(test)]
mod tests;

pub use diagnostic::{ParseDiagnostic, ParseDiagnosticKind, format_parse_diagnostic_kind};
pub(crate) use draft::PropertyDirection;
pub use inline::parse_inline;
pub(crate) use inline::{is_local_link_target, uri_scheme};
pub(crate) use types::PropertyDeclaration;
pub use types::{
    Block, BlockKind, Date, DateMonth, DateRange, DateStamp, DateStampKind, DateStampTarget,
    Document, Inline, IsoWeek, ListItem, ListKind, ReferenceDefinition, ReferenceDefinitions,
    ReferenceValueKind, TableCell, TableColumnAlignment, TableRow, TableRowKind, TodoState,
};

pub struct ParseResult<'a> {
    pub document: Document<'a>,
    pub diagnostics: Vec<ParseDiagnostic<'a>>,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParseCounters {
    pub(crate) root: usize,
    pub(crate) nested: usize,
}

#[cfg(test)]
thread_local! {
    static ROOT_PARSE_COUNT: Cell<usize> = const { Cell::new(0) };
    static NESTED_PARSE_COUNT: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_parse_counters() {
    ROOT_PARSE_COUNT.set(0);
    NESTED_PARSE_COUNT.set(0);
}

#[cfg(test)]
pub(crate) fn read_parse_counters() -> ParseCounters {
    ParseCounters {
        root: ROOT_PARSE_COUNT.get(),
        nested: NESTED_PARSE_COUNT.get(),
    }
}

pub fn parse(source: &str) -> ParseResult<'_> {
    #[cfg(test)]
    ROOT_PARSE_COUNT.set(ROOT_PARSE_COUNT.get() + 1);

    let lines = line::scan_lines(source);
    let (drafts, diagnostics) = draft::parse_drafts(&lines);
    let document = build::build_documents(&drafts);

    ParseResult {
        document,
        diagnostics,
    }
}

pub(crate) fn parse_with_references<'source, 'parent>(
    source: &'source str,
    inherited: &ReferenceDefinitions<'parent>,
) -> ParseResult<'source>
where
    'parent: 'source,
{
    #[cfg(test)]
    NESTED_PARSE_COUNT.set(NESTED_PARSE_COUNT.get() + 1);

    let lines = line::scan_lines(source);
    let (drafts, diagnostics) = draft::parse_drafts(&lines);
    let document = build::build_documents_with_references(&drafts, Some(inherited));

    ParseResult {
        document,
        diagnostics,
    }
}
