use crate::parser::{self, BlockKind, Inline};

use super::marker::{date_range_raw, date_stamp_raw};

const DATE_CONTEXT_MAX_CHARS: usize = 500;

#[derive(Debug, Clone)]
pub(super) struct DateHeadingContext {
    level: usize,
    context: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct DateTraversalContext {
    headings: Vec<DateHeadingContext>,
    top_list_item: Option<String>,
}

impl DateTraversalContext {
    pub(super) fn current_heading_context(&self) -> Option<&str> {
        self.headings.last().map(|heading| heading.context.as_str())
    }

    pub(super) fn parent_heading_context(&self, level: usize) -> Option<&str> {
        self.headings
            .iter()
            .rev()
            .find(|heading| heading.level < level)
            .map(|heading| heading.context.as_str())
    }

    pub(super) fn enter_heading(&mut self, level: usize, body: &str) {
        self.headings.retain(|heading| heading.level < level);
        self.headings.push(DateHeadingContext {
            level,
            context: heading_date_context(level, body),
        });
    }

    pub(super) fn with_top_list_item(&self, top_list_item: String) -> Self {
        let mut context = self.clone();
        if context.top_list_item.is_none() {
            context.top_list_item = Some(top_list_item);
        }
        context
    }

    pub(super) fn contextualize(&self, local_context: &str) -> String {
        date_context_with_scope(
            self.current_heading_context(),
            self.top_list_item.as_deref(),
            local_context,
        )
    }

    pub(super) fn contextualize_heading(&self, level: usize, local_context: &str) -> String {
        date_context_with_scope(
            self.parent_heading_context(level),
            self.top_list_item.as_deref(),
            local_context,
        )
    }
}

fn truncate_date_context(mut input: String) -> String {
    if let Some((byte_index, _)) = input.char_indices().nth(DATE_CONTEXT_MAX_CHARS) {
        input.truncate(byte_index);
        input.push_str("...");
    }

    input
}

fn push_date_context_part(context: &mut String, part: &str, indent: usize) {
    let part = part.trim_end();
    if part.trim().is_empty() {
        return;
    }

    if !context.is_empty() {
        context.push('\n');
    }

    let prefix = " ".repeat(indent);
    for (index, line) in part.lines().enumerate() {
        if index > 0 {
            context.push('\n');
        }
        if indent > 0 && !line.is_empty() {
            context.push_str(&prefix);
        }
        context.push_str(line);
    }
}

fn date_context_with_scope(
    heading_context: Option<&str>,
    top_list_item: Option<&str>,
    local_context: &str,
) -> String {
    let mut context = String::new();
    if let Some(heading_context) = heading_context {
        push_date_context_part(&mut context, heading_context, 0);
    }
    if let Some(top_list_item) = top_list_item {
        push_date_context_part(&mut context, top_list_item, 0);
    }

    let local_context = local_context.trim_end();
    let duplicates_top_list_item =
        top_list_item.is_some_and(|top_list_item| top_list_item.trim_end() == local_context);
    if !duplicates_top_list_item {
        let indent = if top_list_item.is_some() { 2 } else { 0 };
        push_date_context_part(&mut context, local_context, indent);
    }

    truncate_date_context(context)
}

fn push_inline_date_context(context: &mut String, inlines: &[Inline<'_>]) {
    for inline in inlines {
        match inline {
            Inline::NoteLink { target } => {
                context.push_str("[[");
                context.push_str(target);
                context.push_str("]]");
            }
            Inline::Link { title, .. } => {
                context.push('[');
                context.push_str(title);
                context.push(']');
            }
            Inline::Footnote { label } => {
                context.push_str("[^");
                context.push_str(label);
                context.push(']');
            }
            Inline::HyperLink { target } => {
                context.push('<');
                context.push_str(target);
                context.push('>');
            }
            Inline::DateStamp(stamp) => context.push_str(&date_stamp_raw(*stamp)),
            Inline::DateRange(range) => context.push_str(&date_range_raw(*range)),
            Inline::Text(text) => context.push_str(text),
            Inline::SoftBreak => context.push(' '),
            Inline::Code(text) => {
                context.push('`');
                context.push_str(text);
                context.push('`');
            }
            Inline::Italic(body) => {
                context.push('/');
                push_inline_date_context(context, body);
                context.push('/');
            }
            Inline::Strong(body) => {
                context.push('*');
                push_inline_date_context(context, body);
                context.push('*');
            }
            Inline::Superscript(text) => {
                context.push_str("^{");
                context.push_str(text);
                context.push('}');
            }
            Inline::Subscript(text) => {
                context.push_str("_{");
                context.push_str(text);
                context.push('}');
            }
            Inline::Highlight(body) => {
                context.push('=');
                push_inline_date_context(context, body);
                context.push('=');
            }
        }
    }
}

fn inline_date_context(inlines: &[Inline<'_>]) -> String {
    let mut context = String::new();
    push_inline_date_context(&mut context, inlines);

    truncate_date_context(context)
}

fn heading_date_context(level: usize, body: &str) -> String {
    format!("{} {body}", "=".repeat(level))
}

fn list_item_marker_prefix(item: &parser::ListItem<'_>) -> &'static str {
    match (item.kind, item.todo) {
        (parser::ListKind::Unordered, Some(parser::TodoState::Todo)) => "- [ ] ",
        (parser::ListKind::Unordered, Some(parser::TodoState::Done)) => "- [x] ",
        (parser::ListKind::Unordered, None) => "- ",
        (parser::ListKind::Ordered, _) => "1. ",
    }
}

pub(super) fn list_item_line_date_context(item: &parser::ListItem<'_>) -> String {
    let mut context = String::new();
    context.push_str(list_item_marker_prefix(item));
    context.push_str(&inline_date_context(&item.body));

    truncate_date_context(context)
}

fn list_item_date_context(item: &parser::ListItem<'_>) -> String {
    let mut context = String::new();
    context.push_str(list_item_marker_prefix(item));
    context.push_str(&inline_date_context(&item.body));

    for child in &item.children {
        let child_context = block_date_context(child);
        if child_context.trim().is_empty() {
            continue;
        }
        for line in child_context.lines() {
            context.push('\n');
            context.push_str("  ");
            context.push_str(line);
        }
    }

    truncate_date_context(context)
}

pub(super) fn table_row_date_context(row: &parser::TableRow<'_>) -> String {
    if row.is_separator() {
        return String::from("| --- |");
    }

    let mut context = String::from("| ");
    context.push_str(
        &row.cells
            .iter()
            .map(|cell| inline_date_context(&cell.body))
            .collect::<Vec<_>>()
            .join(" | "),
    );
    context.push_str(" |");

    truncate_date_context(context)
}

fn table_date_context(header: &parser::TableRow<'_>, rows: &[parser::TableRow<'_>]) -> String {
    let mut context = table_row_date_context(header);

    for row in rows {
        context.push('\n');
        context.push_str(&table_row_date_context(row));
    }

    truncate_date_context(context)
}

pub(super) fn table_body_row_date_context(
    header_context: &str,
    row: &parser::TableRow<'_>,
) -> String {
    let mut context = header_context.to_string();
    context.push('\n');
    context.push_str(&table_row_date_context(row));

    truncate_date_context(context)
}

pub(super) fn block_date_context(block: &parser::Block<'_>) -> String {
    let context = match &block.kind {
        BlockKind::Paragraph { body } => inline_date_context(body),
        BlockKind::Code { lines, .. } => lines.join("\n"),
        BlockKind::Heading {
            level, raw_body, ..
        } => heading_date_context(*level, raw_body),
        BlockKind::List { items } => items
            .iter()
            .map(list_item_date_context)
            .collect::<Vec<_>>()
            .join("\n"),
        BlockKind::Quote { lines } => lines.join("\n"),
        BlockKind::Table { header, rows, .. } => table_date_context(header, rows),
        BlockKind::Container { kind, args, lines } => {
            let mut context = String::from("--- ");
            context.push_str(kind);
            if !args.is_empty() {
                context.push(' ');
                context.push_str(&args.join(" "));
            }
            if !lines.is_empty() {
                context.push('\n');
                context.push_str(&lines.join("\n"));
            }
            context
        }
        BlockKind::ReferenceDefinition { .. } => String::new(),
    };

    truncate_date_context(context)
}

pub(super) fn property_date_context(key: &str, value: &str, owner_context: &str) -> String {
    let mut context = format!("{key}: {value}");
    if !owner_context.trim().is_empty() {
        context.push('\n');
        context.push_str(owner_context);
    }

    truncate_date_context(context)
}

pub(super) fn document_date_context(
    document: &parser::Document<'_>,
    fallback_title: &str,
) -> String {
    truncate_date_context(document.title().unwrap_or(fallback_title).to_string())
}
