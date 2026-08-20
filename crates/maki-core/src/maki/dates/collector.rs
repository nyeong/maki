use std::collections::BTreeMap;
use std::path::Path;

use crate::parser::{self, BlockKind, DateRange, DateStamp, Inline};

use super::super::note::{Note, NoteRef};
use super::context::{
    DateTraversalContext, block_date_context, document_date_context, list_item_line_date_context,
    property_date_context, table_body_row_date_context, table_row_date_context,
};
use super::ids::{inline_date_occurrence_id, property_date_occurrence_id};
use super::marker::{date_range_marker, date_stamp_marker};
use super::types::{DateIndex, DateMarker, DateOccurrence, DateOrigin};

struct DateIndexCollector<'a> {
    index: &'a mut DateIndex,
    source_path: &'a Path,
    note_ref: NoteRef,
    note_title: String,
    inline_ordinal: usize,
    property_ordinal: usize,
}

impl<'a> DateIndexCollector<'a> {
    fn new(
        index: &'a mut DateIndex,
        source_path: &'a Path,
        note_ref: NoteRef,
        note_title: String,
    ) -> Self {
        Self {
            index,
            source_path,
            note_ref,
            note_title,
            inline_ordinal: 0,
            property_ordinal: 0,
        }
    }

    fn push_occurrence(
        &mut self,
        id: String,
        origin: DateOrigin,
        marker: DateMarker,
        context: &str,
    ) {
        self.index.insert_occurrence(DateOccurrence {
            id,
            source_path: self.source_path.to_path_buf(),
            note_ref: self.note_ref.clone(),
            note_title: self.note_title.clone(),
            origin,
            marker,
            context: context.to_string(),
        });
    }

    fn push_inline_stamp(&mut self, stamp: DateStamp<'_>, context: &str) {
        self.inline_ordinal += 1;
        self.push_occurrence(
            inline_date_occurrence_id(self.source_path, self.inline_ordinal),
            DateOrigin::Inline,
            date_stamp_marker(stamp),
            context,
        );
    }

    fn push_inline_range(&mut self, range: DateRange<'_>, context: &str) {
        self.inline_ordinal += 1;
        self.push_occurrence(
            inline_date_occurrence_id(self.source_path, self.inline_ordinal),
            DateOrigin::Inline,
            date_range_marker(range),
            context,
        );
    }

    fn push_property_stamp(&mut self, key: &str, stamp: DateStamp<'_>, context: &str) {
        self.property_ordinal += 1;
        self.push_occurrence(
            property_date_occurrence_id(self.source_path, self.property_ordinal),
            DateOrigin::Property {
                key: key.to_string(),
            },
            date_stamp_marker(stamp),
            context,
        );
    }

    fn push_property_range(&mut self, key: &str, range: DateRange<'_>, context: &str) {
        self.property_ordinal += 1;
        self.push_occurrence(
            property_date_occurrence_id(self.source_path, self.property_ordinal),
            DateOrigin::Property {
                key: key.to_string(),
            },
            date_range_marker(range),
            context,
        );
    }
}
fn collect_inline_dates(
    collector: &mut DateIndexCollector<'_>,
    inlines: &[Inline<'_>],
    context: &str,
) {
    for inline in inlines {
        match inline {
            Inline::DateStamp(stamp) => collector.push_inline_stamp(*stamp, context),
            Inline::DateRange(range) => collector.push_inline_range(*range, context),
            Inline::Strong(body) => collect_inline_dates(collector, body, context),
            Inline::NoteLink { .. }
            | Inline::Link { .. }
            | Inline::Text(_)
            | Inline::SoftBreak
            | Inline::Code(_) => {}
        }
    }
}

fn collect_property_inline_dates(
    collector: &mut DateIndexCollector<'_>,
    key: &str,
    inlines: &[Inline<'_>],
    context: &str,
) {
    for inline in inlines {
        match inline {
            Inline::DateStamp(stamp) => collector.push_property_stamp(key, *stamp, context),
            Inline::DateRange(range) => collector.push_property_range(key, *range, context),
            Inline::Strong(body) => collect_property_inline_dates(collector, key, body, context),
            Inline::NoteLink { .. }
            | Inline::Link { .. }
            | Inline::Text(_)
            | Inline::SoftBreak
            | Inline::Code(_) => {}
        }
    }
}

fn collect_property_dates<'a>(
    collector: &mut DateIndexCollector<'_>,
    properties: impl Iterator<Item = (&'a str, &'a str)>,
    owner_context: &str,
) {
    for (key, value) in properties {
        let context = property_date_context(key, value, owner_context);
        let inlines = parser::parse_inline(value);
        collect_property_inline_dates(collector, key, &inlines, &context);
    }
}

fn collect_list_item_dates(
    collector: &mut DateIndexCollector<'_>,
    item: &parser::ListItem<'_>,
    context: &DateTraversalContext,
) {
    let item_line_context = list_item_line_date_context(item);
    let mut item_context = context.with_top_list_item(item_line_context.clone());
    let occurrence_context = item_context.contextualize(&item_line_context);

    collect_inline_dates(collector, &item.body, &occurrence_context);
    for child in &item.children {
        collect_block_dates(collector, child, &mut item_context);
    }
}

fn collect_table_row_dates(
    collector: &mut DateIndexCollector<'_>,
    row: &parser::TableRow<'_>,
    context: &str,
) {
    if row.is_separator() {
        return;
    }

    for cell in &row.cells {
        collect_inline_dates(collector, &cell.body, context);
    }
}

fn collect_block_dates(
    collector: &mut DateIndexCollector<'_>,
    block: &parser::Block<'_>,
    context: &mut DateTraversalContext,
) {
    let local_context = block_date_context(block);
    let block_context = match &block.kind {
        BlockKind::Heading { level, .. } => context.contextualize_heading(*level, &local_context),
        _ => context.contextualize(&local_context),
    };
    collect_property_dates(collector, block.properties(), &block_context);

    match &block.kind {
        BlockKind::Paragraph { body } => collect_inline_dates(collector, body, &block_context),
        BlockKind::Heading { level, body } => {
            let inlines = parser::parse_inline(body);
            collect_inline_dates(collector, &inlines, &block_context);
            context.enter_heading(*level, body);
        }
        BlockKind::List { items } => {
            for item in items {
                collect_list_item_dates(collector, item, context);
            }
        }
        BlockKind::Quote { lines } => collect_maki_lines_dates(collector, lines, context),
        BlockKind::Table { header, rows, .. } => {
            let table_header_context = table_row_date_context(header);
            let header_context = context.contextualize(&table_header_context);
            collect_table_row_dates(collector, header, &header_context);
            for row in rows {
                let row_context =
                    context.contextualize(&table_body_row_date_context(&table_header_context, row));
                collect_table_row_dates(collector, row, &row_context);
            }
        }
        BlockKind::Container { kind, lines, .. } if *kind == "quote" => {
            collect_maki_lines_dates(collector, lines, context)
        }
        BlockKind::Code { .. } | BlockKind::Container { .. } => {}
    }
}

fn collect_maki_lines_dates(
    collector: &mut DateIndexCollector<'_>,
    lines: &[&str],
    context: &DateTraversalContext,
) {
    let source = lines.join("\n");
    let parsed = parser::parse(&source);
    let mut nested_context = context.clone();
    collect_document_dates_with_context(collector, &parsed.document, &mut nested_context);
}

fn collect_document_dates_with_context(
    collector: &mut DateIndexCollector<'_>,
    document: &parser::Document<'_>,
    context: &mut DateTraversalContext,
) {
    let document_context =
        context.contextualize(&document_date_context(document, &collector.note_title));

    collect_property_dates(collector, document.properties(), &document_context);
    for block in &document.blocks {
        collect_block_dates(collector, block, context);
    }
}

fn collect_document_dates(collector: &mut DateIndexCollector<'_>, document: &parser::Document<'_>) {
    let mut context = DateTraversalContext::default();
    collect_document_dates_with_context(collector, document, &mut context);
}

pub(in crate::maki) fn collect_date_index(notes: &BTreeMap<NoteRef, Note>) -> DateIndex {
    let mut date_index = DateIndex::default();

    for note in notes.values() {
        let Ok(source) = std::fs::read_to_string(&note.absolute_path) else {
            continue;
        };
        let parsed = parser::parse(&source);
        let note_ref = note.note_ref();
        let note_title = parsed
            .document
            .title()
            .unwrap_or(note.file_stem())
            .to_string();
        let mut collector =
            DateIndexCollector::new(&mut date_index, note.source_path(), note_ref, note_title);
        collect_document_dates(&mut collector, &parsed.document);
    }

    date_index.sort_backlinks();
    date_index
}
