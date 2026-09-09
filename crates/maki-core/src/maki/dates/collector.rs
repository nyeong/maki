use std::collections::BTreeSet;
use std::path::Path;

use crate::nested::{NestedDocumentObserver, NestedTraversalVisitor, traverse_parsed_document};
use crate::parser::{self, BlockKind, DateRange, DateStamp, Inline};

use super::super::note::NoteRef;
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
    note_title: &'a str,
    inline_ordinal: usize,
    property_ordinal: usize,
}

#[derive(Default)]
struct FootnoteDefinitionOrder {
    keys: Vec<String>,
    seen: BTreeSet<String>,
}

impl FootnoteDefinitionOrder {
    fn register(&mut self, key: &str) {
        if self.seen.insert(key.to_string()) {
            self.keys.push(key.to_string());
        }
    }
}

impl<'a> DateIndexCollector<'a> {
    fn new(
        index: &'a mut DateIndex,
        source_path: &'a Path,
        note_ref: NoteRef,
        note_title: &'a str,
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
            note_title: self.note_title.to_string(),
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
    references: &parser::ReferenceDefinitions<'_>,
) {
    for inline in inlines {
        match inline {
            Inline::DateStamp(stamp) => collector.push_inline_stamp(*stamp, context),
            Inline::DateRange(range) => collector.push_inline_range(*range, context),
            Inline::Reference { raw, key, .. } => {
                let Some(definition) = references.get(key) else {
                    continue;
                };
                match definition.value.as_slice() {
                    [Inline::DateStamp(stamp)] => collector.push_inline_stamp(*stamp, context),
                    [Inline::DateRange(range)] if raw.ends_with("][]") => {
                        collector.push_inline_range(*range, context)
                    }
                    _ => {}
                }
            }
            _ => {
                if let Some(body) = inline.nested_inlines() {
                    collect_inline_dates(collector, body, context, references);
                }
            }
        }
    }
}

fn collect_footnote_definition_keys(
    inlines: &[Inline<'_>],
    footnote_order: &mut FootnoteDefinitionOrder,
) {
    for inline in inlines {
        match inline {
            Inline::Footnote { key, .. } => footnote_order.register(key),
            _ => {
                if let Some(body) = inline.nested_inlines() {
                    collect_footnote_definition_keys(body, footnote_order);
                }
            }
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
            _ => {
                if let Some(body) = inline.nested_inlines() {
                    collect_property_inline_dates(collector, key, body, context);
                }
            }
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

fn collect_table_row_dates(
    collector: &mut DateIndexCollector<'_>,
    row: &parser::TableRow<'_>,
    context: &str,
    references: &parser::ReferenceDefinitions<'_>,
    footnote_order: &mut FootnoteDefinitionOrder,
) {
    if row.is_separator() {
        return;
    }

    for cell in &row.cells {
        collect_inline_dates(collector, &cell.body, context, references);
        collect_footnote_definition_keys(&cell.body, footnote_order);
    }
}

struct DateTraversal<'a> {
    collector: DateIndexCollector<'a>,
    footnote_orders: Vec<FootnoteDefinitionOrder>,
}

impl<'a> DateTraversal<'a> {
    fn new(collector: DateIndexCollector<'a>) -> Self {
        Self {
            collector,
            footnote_orders: Vec::new(),
        }
    }
}

impl NestedTraversalVisitor for DateTraversal<'_> {
    type Context = DateTraversalContext;

    fn enter_document(&mut self, document: &parser::Document<'_>, context: &mut Self::Context) {
        self.footnote_orders
            .push(FootnoteDefinitionOrder::default());
        let document_context =
            context.contextualize(&document_date_context(document, self.collector.note_title));
        collect_property_dates(
            &mut self.collector,
            document.properties(),
            &document_context,
        );
    }

    fn visit_block(
        &mut self,
        block: &parser::Block<'_>,
        references: &parser::ReferenceDefinitions<'_>,
        context: &mut Self::Context,
    ) {
        let local_context = block_date_context(block);
        let block_context = match &block.kind {
            BlockKind::Heading { level, .. } => {
                context.contextualize_heading(*level, &local_context)
            }
            _ => context.contextualize(&local_context),
        };
        collect_property_dates(&mut self.collector, block.properties(), &block_context);

        let footnote_order = self
            .footnote_orders
            .last_mut()
            .expect("document traversal should own a footnote order");
        match &block.kind {
            BlockKind::Paragraph { body } => {
                collect_inline_dates(&mut self.collector, body, &block_context, references);
                collect_footnote_definition_keys(body, footnote_order);
            }
            BlockKind::Heading {
                level,
                body,
                raw_body,
            } => {
                collect_inline_dates(&mut self.collector, body, &block_context, references);
                collect_footnote_definition_keys(body, footnote_order);
                context.enter_heading(*level, raw_body);
            }
            BlockKind::Table { header, rows, .. } => {
                let table_header_context = table_row_date_context(header);
                let header_context = context.contextualize(&table_header_context);
                collect_table_row_dates(
                    &mut self.collector,
                    header,
                    &header_context,
                    references,
                    footnote_order,
                );
                for row in rows {
                    let row_context = context
                        .contextualize(&table_body_row_date_context(&table_header_context, row));
                    collect_table_row_dates(
                        &mut self.collector,
                        row,
                        &row_context,
                        references,
                        footnote_order,
                    );
                }
            }
            BlockKind::List { .. }
            | BlockKind::Quote { .. }
            | BlockKind::Code { .. }
            | BlockKind::Container { .. }
            | BlockKind::ReferenceDefinition { .. } => {}
        }
    }

    fn enter_list_item(
        &mut self,
        item: &parser::ListItem<'_>,
        references: &parser::ReferenceDefinitions<'_>,
        context: &Self::Context,
    ) -> Self::Context {
        let item_line_context = list_item_line_date_context(item);
        let item_context = context.with_top_list_item(item_line_context.clone());
        let occurrence_context = item_context.contextualize(&item_line_context);
        collect_inline_dates(
            &mut self.collector,
            &item.body,
            &occurrence_context,
            references,
        );
        collect_footnote_definition_keys(
            &item.body,
            self.footnote_orders
                .last_mut()
                .expect("document traversal should own a footnote order"),
        );
        item_context
    }

    fn exit_document(&mut self, document: &parser::Document<'_>, context: &mut Self::Context) {
        let mut footnote_order = self
            .footnote_orders
            .pop()
            .expect("document traversal should own a footnote order");
        let mut note_index = 0;
        while note_index < footnote_order.keys.len() {
            let key = footnote_order.keys[note_index].clone();
            note_index += 1;
            let Some(definition) = document.reference(&key) else {
                continue;
            };
            if definition.value_kind() != parser::ReferenceValueKind::Prose {
                continue;
            }
            let marker = format!("[{}]", definition.key);
            let reference_source = format!("{marker}: {}", definition.raw_value);
            let reference_context = context.contextualize(&reference_source);
            collect_inline_dates(
                &mut self.collector,
                &definition.value,
                &reference_context,
                document.reference_definitions(),
            );
            collect_footnote_definition_keys(&definition.value, &mut footnote_order);
        }
    }
}

pub(crate) fn collect_parsed_document_dates(
    date_index: &mut DateIndex,
    source_path: &Path,
    note_ref: NoteRef,
    note_title: &str,
    source: &str,
    parsed: &parser::ParseResult<'_>,
    nested_observer: &mut dyn NestedDocumentObserver,
) {
    let collector = DateIndexCollector::new(date_index, source_path, note_ref, note_title);
    let mut traversal = DateTraversal::new(collector);
    traverse_parsed_document(
        source,
        parsed,
        DateTraversalContext::default(),
        &mut traversal,
        nested_observer,
    );
    debug_assert!(traversal.footnote_orders.is_empty());
}
