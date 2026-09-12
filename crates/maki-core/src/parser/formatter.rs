use std::{borrow::Cow, fmt};

use crate::{
    parser::{self, Block, BlockKind, Document, ListItem, PropertyDirection, ReferenceDefinitions},
    source::{SourceSpan, slice_span},
};

use crate::parser::draft::{BlockDraft, PropertyItemDraft, TableRowDraft};

const MAX_FORMAT_NESTING_DEPTH: usize = 128;

/// A formatter refusal.
///
/// Maki formatting is deliberately conservative: malformed input and any
/// transformation that cannot be proven idempotent and semantics-preserving
/// are left untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    Parse { line: usize, message: String },
    ChangedSemantics,
    NotIdempotent,
    NestingTooDeep { limit: usize },
    InvalidEdit,
}

impl fmt::Display for FormatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { line, message } => {
                write!(formatter, "cannot format line {line}: {message}")
            }
            Self::ChangedSemantics => {
                write!(formatter, "formatter would change document semantics")
            }
            Self::NotIdempotent => write!(formatter, "formatter output is not idempotent"),
            Self::NestingTooDeep { limit } => {
                write!(
                    formatter,
                    "cannot format more than {limit} nested quote levels"
                )
            }
            Self::InvalidEdit => write!(formatter, "formatter produced an invalid source edit"),
        }
    }
}

impl std::error::Error for FormatError {}

#[derive(Debug, PartialEq, Eq)]
struct Edit {
    span: SourceSpan,
    replacement: String,
}

/// Formats one Maki source document without touching paragraph wrapping,
/// line endings, or raw block bodies.
///
/// The returned value borrows `source` when it is already canonical. Before
/// returning changed text, this function reparses it, compares the semantic
/// document model, and verifies that a second formatting pass is a no-op.
pub fn format_source(source: &str) -> Result<Cow<'_, str>, FormatError> {
    format_source_at_depth(source, 0)
}

fn format_source_at_depth(source: &str, depth: usize) -> Result<Cow<'_, str>, FormatError> {
    if depth > MAX_FORMAT_NESTING_DEPTH {
        return Err(FormatError::NestingTooDeep {
            limit: MAX_FORMAT_NESTING_DEPTH,
        });
    }

    let plan = plan_source(source, depth)?;
    if plan.edits.is_empty() {
        return Ok(Cow::Borrowed(source));
    }

    let formatted = apply_edits(source, &plan.edits)?;
    let formatted_plan = plan_source(&formatted, depth)?;
    if !documents_semantically_equal(&plan.document, &formatted_plan.document) {
        return Err(FormatError::ChangedSemantics);
    }
    if !formatted_plan.edits.is_empty() {
        return Err(FormatError::NotIdempotent);
    }

    Ok(Cow::Owned(formatted))
}

struct FormatPlan<'a> {
    document: Document<'a>,
    edits: Vec<Edit>,
}

fn plan_source(source: &str, depth: usize) -> Result<FormatPlan<'_>, FormatError> {
    let lines = parser::line::scan_lines(source);
    let (drafts, diagnostics) = parser::draft::parse_drafts(&lines);
    if let Some(diagnostic) = diagnostics.first() {
        return Err(FormatError::Parse {
            line: diagnostic.line,
            message: parser::format_parse_diagnostic_kind(&diagnostic.kind),
        });
    }

    let document = parser::build::build_documents(&drafts);
    let mut edits = Vec::new();
    collect_edits(source, &drafts, &mut edits)?;
    collect_nested_quote_edits(source, &document.blocks, depth, &mut edits)?;
    edits.sort_by_key(|edit| edit.span);
    validate_edits(source, &edits)?;

    Ok(FormatPlan { document, edits })
}

fn collect_edits(
    source: &str,
    drafts: &[BlockDraft<'_>],
    edits: &mut Vec<Edit>,
) -> Result<(), FormatError> {
    for draft in drafts {
        match draft {
            BlockDraft::Property { kind, items, .. } => {
                for item in items {
                    let replacement = format_property(*kind, item);
                    push_edit(source, item.raw_line, replacement, edits)?;
                }
            }
            BlockDraft::Heading {
                raw_line,
                level,
                body,
            } => {
                push_edit(
                    source,
                    raw_line,
                    format!("{} {body}", "=".repeat(*level)),
                    edits,
                )?;
            }
            BlockDraft::Container {
                opener_raw_line,
                fence_len,
                kind,
                args,
                ..
            } => {
                push_edit(
                    source,
                    opener_raw_line,
                    format_container_header(*fence_len, kind, args),
                    edits,
                )?;
            }
            BlockDraft::List { items } => {
                for item in items {
                    if item.todo.is_some() && item.body.is_empty() {
                        push_edit(
                            source,
                            item.raw_line,
                            item.raw_line.trim_end().to_owned(),
                            edits,
                        )?;
                    }
                    collect_edits(source, &item.children, edits)?;
                }
            }
            BlockDraft::Table {
                header_raw_line,
                separator_raw_line,
                header,
                rows,
            } => {
                push_edit(
                    source,
                    header_raw_line,
                    format_table_data_row(header),
                    edits,
                )?;
                push_edit(
                    source,
                    separator_raw_line,
                    format_table_separator(header.len()),
                    edits,
                )?;
                for row in rows {
                    let replacement = format_table_row(row, header.len());
                    push_edit(source, row.raw_line, replacement, edits)?;
                }
            }
            BlockDraft::Paragraph { .. }
            | BlockDraft::Code { .. }
            | BlockDraft::Quote { .. }
            | BlockDraft::ReferenceDefinition { .. } => {}
        }
    }

    Ok(())
}

fn collect_nested_quote_edits(
    source: &str,
    blocks: &[Block<'_>],
    depth: usize,
    edits: &mut Vec<Edit>,
) -> Result<(), FormatError> {
    for block in blocks {
        match &block.kind {
            BlockKind::List { items } => {
                for item in items {
                    collect_nested_quote_edits(source, &item.children, depth, edits)?;
                }
            }
            BlockKind::Quote { lines } if !parser::quote_mode_is_raw(block.property("mode")) => {
                collect_nested_line_edits(source, lines, depth, edits)?;
            }
            BlockKind::Container { kind, lines, .. }
                if *kind == "quote" && !parser::quote_mode_is_raw(block.property("mode")) =>
            {
                collect_nested_line_edits(source, lines, depth, edits)?;
            }
            BlockKind::Paragraph { .. }
            | BlockKind::Code { .. }
            | BlockKind::Heading { .. }
            | BlockKind::Quote { .. }
            | BlockKind::Table { .. }
            | BlockKind::Container { .. }
            | BlockKind::ReferenceDefinition { .. } => {}
        }
    }

    Ok(())
}

fn collect_nested_line_edits(
    source: &str,
    lines: &[&str],
    depth: usize,
    edits: &mut Vec<Edit>,
) -> Result<(), FormatError> {
    if lines.is_empty() {
        return Ok(());
    }

    let nested_source = lines.join("\n");
    let formatted = format_source_at_depth(&nested_source, depth + 1)
        .map_err(|error| map_nested_format_error(source, lines, error))?;
    if matches!(formatted, Cow::Borrowed(_)) {
        return Ok(());
    }

    let mut formatted_lines = formatted.split('\n');
    for original in lines {
        let replacement = formatted_lines.next().ok_or(FormatError::InvalidEdit)?;
        push_edit(source, original, replacement.to_owned(), edits)?;
    }
    if formatted_lines.next().is_some() {
        return Err(FormatError::InvalidEdit);
    }

    Ok(())
}

fn map_nested_format_error(source: &str, lines: &[&str], error: FormatError) -> FormatError {
    let FormatError::Parse { line, message } = error else {
        return error;
    };
    let Some(line_source) = line.checked_sub(1).and_then(|index| lines.get(index)) else {
        return FormatError::InvalidEdit;
    };
    let Some(span) = slice_span(source, line_source) else {
        return FormatError::InvalidEdit;
    };

    FormatError::Parse {
        line: source[..span.start]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1,
        message,
    }
}

fn format_property(kind: PropertyDirection, item: &PropertyItemDraft<'_>) -> String {
    let marker = match kind {
        PropertyDirection::Previous => "--^",
        PropertyDirection::Next => "--v",
    };
    if item.value.is_empty() {
        format!("{marker} {}:", item.key)
    } else {
        format!("{marker} {}: {}", item.key, item.value)
    }
}

fn format_container_header(fence_len: usize, kind: &str, args: &[&str]) -> String {
    let mut formatted = "-".repeat(fence_len);
    if !kind.is_empty() {
        if kind.starts_with('-') {
            formatted.push(' ');
        }
        formatted.push_str(kind);
        for argument in args {
            formatted.push(' ');
            formatted.push_str(argument);
        }
    }
    formatted
}

fn format_table_row(row: &TableRowDraft<'_>, column_count: usize) -> String {
    match row.kind {
        parser::TableRowKind::Data => format_table_data_row(&row.cells),
        parser::TableRowKind::Separator => format_table_separator(column_count),
    }
}

fn format_table_data_row(cells: &[&str]) -> String {
    format!("| {} |", cells.join(" | "))
}

fn format_table_separator(column_count: usize) -> String {
    format!(
        "|{}|",
        std::iter::repeat_n("---", column_count)
            .collect::<Vec<_>>()
            .join("+")
    )
}

fn push_edit(
    source: &str,
    original: &str,
    replacement: String,
    edits: &mut Vec<Edit>,
) -> Result<(), FormatError> {
    if original == replacement {
        return Ok(());
    }
    let span = slice_span(source, original).ok_or(FormatError::InvalidEdit)?;
    edits.push(Edit { span, replacement });
    Ok(())
}

fn validate_edits(source: &str, edits: &[Edit]) -> Result<(), FormatError> {
    let mut previous_end = 0;
    for edit in edits {
        if edit.span.start < previous_end
            || edit.span.end > source.len()
            || !source.is_char_boundary(edit.span.start)
            || !source.is_char_boundary(edit.span.end)
        {
            return Err(FormatError::InvalidEdit);
        }
        previous_end = edit.span.end;
    }
    Ok(())
}

fn apply_edits(source: &str, edits: &[Edit]) -> Result<String, FormatError> {
    validate_edits(source, edits)?;
    let capacity = edits.iter().fold(source.len(), |capacity, edit| {
        capacity
            .saturating_sub(edit.span.len())
            .saturating_add(edit.replacement.len())
    });
    let mut formatted = String::with_capacity(capacity);
    let mut cursor = 0;
    for edit in edits {
        formatted.push_str(&source[cursor..edit.span.start]);
        formatted.push_str(&edit.replacement);
        cursor = edit.span.end;
    }
    formatted.push_str(&source[cursor..]);
    Ok(formatted)
}

fn documents_semantically_equal(left: &Document<'_>, right: &Document<'_>) -> bool {
    properties_semantically_equal(left.property_declarations(), right.property_declarations())
        && references_semantically_equal(
            left.reference_definitions(),
            right.reference_definitions(),
        )
        && blocks_semantically_equal(&left.blocks, &right.blocks)
}

fn properties_semantically_equal(
    left: &[parser::PropertyDeclaration<'_>],
    right: &[parser::PropertyDeclaration<'_>],
) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.direction() == right.direction()
                && left.key() == right.key()
                && left.value() == right.value()
        })
}

fn references_semantically_equal(
    left: &ReferenceDefinitions<'_>,
    right: &ReferenceDefinitions<'_>,
) -> bool {
    left.iter().eq(right.iter())
}

fn blocks_semantically_equal(left: &[Block<'_>], right: &[Block<'_>]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| block_semantically_equal(left, right))
}

fn block_semantically_equal(left: &Block<'_>, right: &Block<'_>) -> bool {
    if !properties_semantically_equal(left.property_declarations(), right.property_declarations()) {
        return false;
    }

    match (&left.kind, &right.kind) {
        (BlockKind::Paragraph { body: left }, BlockKind::Paragraph { body: right }) => {
            left == right
        }
        (
            BlockKind::Code {
                lines: left_lines,
                lang: left_lang,
            },
            BlockKind::Code {
                lines: right_lines,
                lang: right_lang,
            },
        ) => left_lines == right_lines && left_lang == right_lang,
        (
            BlockKind::Heading {
                level: left_level,
                body: left_body,
                raw_body: left_raw_body,
            },
            BlockKind::Heading {
                level: right_level,
                body: right_body,
                raw_body: right_raw_body,
            },
        ) => {
            left_level == right_level && left_body == right_body && left_raw_body == right_raw_body
        }
        (BlockKind::List { items: left }, BlockKind::List { items: right }) => {
            list_items_semantically_equal(left, right)
        }
        (BlockKind::Quote { lines: left_lines }, BlockKind::Quote { lines: right_lines }) => {
            if parser::quote_mode_is_raw(left.property("mode")) {
                left_lines == right_lines
            } else {
                nested_lines_semantically_equal(left_lines, right_lines)
            }
        }
        (
            BlockKind::Table {
                header: left_header,
                alignments: left_alignments,
                rows: left_rows,
            },
            BlockKind::Table {
                header: right_header,
                alignments: right_alignments,
                rows: right_rows,
            },
        ) => {
            left_header == right_header
                && left_alignments == right_alignments
                && left_rows == right_rows
        }
        (
            BlockKind::Container {
                kind: left_kind,
                args: left_args,
                lines: left_lines,
            },
            BlockKind::Container {
                kind: right_kind,
                args: right_args,
                lines: right_lines,
            },
        ) => {
            left_kind == right_kind
                && left_args == right_args
                && if *left_kind == "quote" && !parser::quote_mode_is_raw(left.property("mode")) {
                    nested_lines_semantically_equal(left_lines, right_lines)
                } else {
                    left_lines == right_lines
                }
        }
        (
            BlockKind::ReferenceDefinition { definitions: left },
            BlockKind::ReferenceDefinition { definitions: right },
        ) => left == right,
        _ => false,
    }
}

fn nested_lines_semantically_equal(left: &[&str], right: &[&str]) -> bool {
    let left_source = left.join("\n");
    let right_source = right.join("\n");
    let left = parser::parse(&left_source);
    let right = parser::parse(&right_source);

    left.diagnostics.is_empty()
        && right.diagnostics.is_empty()
        && documents_semantically_equal(&left.document, &right.document)
}

fn list_items_semantically_equal(left: &[ListItem<'_>], right: &[ListItem<'_>]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.kind == right.kind
                && left.todo == right.todo
                && left.body == right.body
                && blocks_semantically_equal(&left.children, &right.children)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{self, AnalysisBlockKind, PropertyOwner};
    use std::path::Path;

    #[test]
    fn formats_structural_whitespace_and_preserves_line_endings() {
        let source = concat!(
            "  --^   title  :  Draft  \r\n",
            "  == Heading\r\n",
            "|a| b  |\r\n",
            "| --- | --- |\r\n",
            "|1| 2 |\r\n",
            "----   code\t rust  \r\n",
            "raw  \r\n",
            "----\r\n",
        );
        let expected = concat!(
            "--^ title: Draft\r\n",
            "== Heading\r\n",
            "| a | b |\r\n",
            "|---+---|\r\n",
            "| 1 | 2 |\r\n",
            "----code rust\r\n",
            "raw  \r\n",
            "----\r\n",
        );

        let formatted = format_source(source).unwrap();

        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), expected);
    }

    #[test]
    fn preserves_paragraph_wrapping_escapes_and_final_newline_policy() {
        let source = concat!(
            "first line  \r\n",
            "second line\r\n",
            "\r\n",
            "\\= paragraph marker\r\n",
            ": raw code  \r\n",
            ":   indentation is content",
        );

        assert!(matches!(format_source(source), Ok(Cow::Borrowed(_))));
        assert_eq!(format_source(source).unwrap(), source);
    }

    #[test]
    fn preserves_raw_container_bodies_for_every_raw_kind() {
        let source = concat!(
            "---   code  rust\n= not a heading  \n---\n",
            "--- text\n  leading and trailing  \n---\n",
            "---pre\n|not|a|table|\n---\n",
            "--- unknown\n--v not: a property\n---\n",
        );
        let expected = concat!(
            "---code rust\n= not a heading  \n---\n",
            "---text\n  leading and trailing  \n---\n",
            "---pre\n|not|a|table|\n---\n",
            "---unknown\n--v not: a property\n---\n",
        );

        assert_eq!(format_source(source).unwrap(), expected);
    }

    #[test]
    fn formats_nested_structures_without_changing_list_bodies() {
        let source = concat!(
            "- [ ]  keep the authored leading space\n",
            "    --v  id : nested  \n",
            "    == Nested\n",
        );
        let expected = concat!(
            "- [ ]  keep the authored leading space\n",
            "  --v id: nested\n",
            "  == Nested\n",
        );

        assert_eq!(format_source(source).unwrap(), expected);
    }

    #[test]
    fn formats_semantic_quote_bodies_recursively() {
        let source = concat!(
            ">   --^  id : line  \n",
            ">   == Nested\n",
            "\n",
            "---   quote\n",
            "  --^  id : container  \n",
            "  == Container\n",
            "---\n",
        );
        let expected = concat!(
            "> --^ id: line\n",
            "> == Nested\n",
            "\n",
            "---quote\n",
            "--^ id: container\n",
            "== Container\n",
            "---\n",
        );

        assert_eq!(format_source(source).unwrap(), expected);
    }

    #[test]
    fn preserves_quote_bodies_in_raw_modes() {
        let source = concat!(
            " --v  mode : pre \n",
            ">   --^  id : untouched  \n",
            "\n",
            " --v  mode : text \n",
            "---quote\n",
            "  == untouched  \n",
            "---\n",
        );
        let expected = concat!(
            "--v mode: pre\n",
            ">   --^  id : untouched  \n",
            "\n",
            "--v mode: text\n",
            "---quote\n",
            "  == untouched  \n",
            "---\n",
        );

        assert_eq!(format_source(source).unwrap(), expected);
    }

    #[test]
    fn refuses_malformed_semantic_quote_bodies() {
        let error = format_source("intro\n> = Good\n> ---code\n> raw\n").unwrap_err();

        assert!(matches!(error, FormatError::Parse { line: 3, .. }));
        assert!(error.to_string().contains("unclosed container"));
    }

    #[test]
    fn removes_only_the_redundant_trailing_space_from_empty_todos() {
        let source = "- [ ] \n- [x] \n- [ ]  \n- [x] keep\n";
        let expected = "- [ ]\n- [x]\n- [ ]  \n- [x] keep\n";

        assert_eq!(format_source(source).unwrap(), expected);
    }

    #[test]
    fn refuses_excessive_quote_nesting_without_overflowing_the_stack() {
        let source = format!("{}= Heading\n", "> ".repeat(MAX_FORMAT_NESTING_DEPTH + 1));

        assert_eq!(
            format_source(&source),
            Err(FormatError::NestingTooDeep {
                limit: MAX_FORMAT_NESTING_DEPTH,
            })
        );
    }

    #[test]
    fn formats_escaped_table_cells_without_touching_cell_content() {
        let source = "|a\\|b|  c |\n| --- | --- |\n| x |y\\|z|\n";
        let expected = "| a\\|b | c |\n|---+---|\n| x | y\\|z |\n";

        assert_eq!(format_source(source).unwrap(), expected);
    }

    #[test]
    fn preserves_fence_width_and_disambiguates_hyphen_leading_kinds() {
        let source = "-----   -code\t option \nbody\n-----\n---   \n---\n";
        let expected = "----- -code option\nbody\n-----\n---\n---\n";

        assert_eq!(format_source(source).unwrap(), expected);
    }

    #[test]
    fn refuses_malformed_input_without_producing_output() {
        let error = format_source("---code\nraw\n").unwrap_err();

        assert!(matches!(error, FormatError::Parse { line: 1, .. }));
        assert!(error.to_string().contains("unclosed container"));
    }

    #[test]
    fn leaves_unrecognized_near_syntax_literal_instead_of_guessing() {
        let source = concat!(
            " --code\n",
            "=missing delimiter\n",
            "-missing delimiter\n",
            "- [X] not a todo\n",
            "|not|a table|\n",
            "---code/bash\n",
        );

        assert!(matches!(format_source(source), Ok(Cow::Borrowed(_))));
        assert_eq!(format_source(source).unwrap(), source);
    }

    #[test]
    fn semantic_comparison_ignores_only_property_source_spacing() {
        let before = parser::parse(" --^  title : Draft  \n= Heading\n");
        let after = parser::parse("--^ title: Draft\n= Heading\n");

        assert!(before.diagnostics.is_empty());
        assert!(after.diagnostics.is_empty());
        assert!(documents_semantically_equal(
            &before.document,
            &after.document
        ));
    }

    #[test]
    fn parse_format_parse_keeps_semantic_analysis() {
        let source = concat!(
            " --^  title : Draft  \n",
            "  == Plan [site]<https://example.com>\n",
            "| task | due |\n",
            "| --- | --- |\n",
            "| ship | <2026-09-12> |\n",
            "---   text\n",
            "raw body\n",
            "---\n",
        );
        let formatted = format_source(source).unwrap();
        let before = analysis::analyze_document(Path::new("note.maki"), source);
        let after = analysis::analyze_document(Path::new("note.maki"), &formatted);

        assert_eq!(before.title, after.title);
        assert_eq!(
            before
                .blocks
                .iter()
                .map(|block| block.kind)
                .collect::<Vec<AnalysisBlockKind>>(),
            after
                .blocks
                .iter()
                .map(|block| block.kind)
                .collect::<Vec<AnalysisBlockKind>>()
        );
        assert_eq!(semantic_properties(&before), semantic_properties(&after));
        assert_eq!(
            before
                .headings
                .iter()
                .map(|heading| (heading.level, &heading.title, &heading.anchor))
                .collect::<Vec<_>>(),
            after
                .headings
                .iter()
                .map(|heading| (heading.level, &heading.title, &heading.anchor))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            before
                .url_links
                .iter()
                .map(|link| (&link.title, &link.target))
                .collect::<Vec<_>>(),
            after
                .url_links
                .iter()
                .map(|link| (&link.title, &link.target))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            before
                .dates
                .iter()
                .map(|date| (&date.kind, &date.target, &date.body, &date.origin))
                .collect::<Vec<_>>(),
            after
                .dates
                .iter()
                .map(|date| (&date.kind, &date.target, &date.body, &date.origin))
                .collect::<Vec<_>>()
        );
        assert_eq!(before.external_links, after.external_links);
    }

    fn semantic_properties(
        analysis: &analysis::DocumentAnalysis,
    ) -> Vec<(
        analysis::PropertyDirection,
        Option<AnalysisBlockKind>,
        &str,
        &str,
    )> {
        analysis
            .properties
            .iter()
            .map(|property| {
                let owner = match property.owner {
                    PropertyOwner::Document => None,
                    PropertyOwner::Block { kind, .. } => Some(kind),
                };
                (
                    property.direction,
                    owner,
                    property.key.as_str(),
                    property.value.as_str(),
                )
            })
            .collect()
    }
}
