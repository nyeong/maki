use std::collections::BTreeSet;

use super::draft::{
    BlockDraft, ListItemDraft, PropertyKind, ReferenceDefinitionDraft, TableRowDraft,
};
use super::inline::{parse_inline, parse_inlines};
use super::types::{
    Block, BlockKind, Document, ListItem, Properties, ReferenceDefinition, ReferenceDefinitions,
    TableCell, TableColumnAlignment, TableRow, TableRowKind,
};

fn collect_document_references<'draft, 'source>(
    drafts: &'draft [BlockDraft<'source>],
) -> Vec<&'draft ReferenceDefinitionDraft<'source>> {
    let mut keys = BTreeSet::new();
    let mut unique_definitions = Vec::new();

    for draft in drafts {
        let BlockDraft::ReferenceDefinition {
            definitions: block_definitions,
        } = draft
        else {
            continue;
        };

        for definition in block_definitions {
            if keys.insert(definition.key) {
                unique_definitions.push(definition);
            }
        }
    }

    unique_definitions
}

fn build_reference_definition<'a>(draft: &ReferenceDefinitionDraft<'a>) -> ReferenceDefinition<'a> {
    ReferenceDefinition {
        key: draft.key,
        raw_value: draft.raw_value,
        value: parse_inline(draft.raw_value),
    }
}

fn build_reference_definitions<'a>(
    drafts: &[&ReferenceDefinitionDraft<'a>],
) -> ReferenceDefinitions<'a> {
    let definitions = drafts
        .iter()
        .map(|definition| build_reference_definition(definition))
        .collect();

    ReferenceDefinitions::new(definitions)
}

fn build_blocks<'a>(drafts: &[BlockDraft<'a>]) -> Vec<Block<'a>> {
    let mut blocks: Vec<Block> = vec![];
    let mut pending_props = Properties::new();

    for draft in drafts {
        match draft {
            BlockDraft::Property {
                kind: PropertyKind::Previous,
                items,
                ..
            } => {
                if let Some(block) = blocks.last_mut() {
                    block.props.extend(items)
                }
            }
            BlockDraft::Property {
                kind: PropertyKind::Next,
                items,
                ..
            } => {
                pending_props.extend(items);
            }
            draft => {
                let block = build_block(draft, std::mem::take(&mut pending_props));
                blocks.push(block);
            }
        }
    }

    blocks
}

pub(super) fn build_documents<'a>(drafts: &[BlockDraft<'a>]) -> Document<'a> {
    build_documents_with_references(drafts, None::<&ReferenceDefinitions<'a>>)
}

pub(super) fn build_documents_with_references<'source, 'parent>(
    drafts: &[BlockDraft<'source>],
    inherited: Option<&ReferenceDefinitions<'parent>>,
) -> Document<'source>
where
    'parent: 'source,
{
    let reference_drafts = collect_document_references(drafts);
    let local_references = build_reference_definitions(&reference_drafts);
    let references = if let Some(inherited) = inherited {
        ReferenceDefinitions::with_inherited(local_references.iter().cloned().collect(), inherited)
    } else {
        local_references
    };
    let mut blocks: Vec<Block> = vec![];
    let mut doc_props = Properties::new();
    let mut pending_props = Properties::new();

    for draft in drafts {
        match draft {
            BlockDraft::Property {
                kind: PropertyKind::Previous,
                items,
                ..
            } => {
                if let Some(block) = blocks.last_mut() {
                    block.props.extend(items)
                } else {
                    doc_props.extend(items);
                }
            }
            BlockDraft::Property {
                kind: PropertyKind::Next,
                items,
                ..
            } => {
                pending_props.extend(items);
            }
            draft => {
                let block = build_block(draft, std::mem::take(&mut pending_props));
                blocks.push(block);
            }
        }
    }

    Document {
        props: doc_props,
        references,
        blocks,
    }
}

fn build_list_item<'a>(draft: &ListItemDraft<'a>) -> ListItem<'a> {
    ListItem {
        body: parse_inline(draft.body),
        kind: draft.kind,
        todo: draft.todo,
        children: build_blocks(&draft.children),
    }
}

fn build_table_row<'a>(kind: TableRowKind, cells: &[&'a str]) -> TableRow<'a> {
    TableRow {
        kind,
        cells: cells
            .iter()
            .map(|cell| TableCell {
                body: parse_inline(cell),
            })
            .collect(),
    }
}

fn is_integer_table_cell(cell: &str) -> bool {
    let cell = cell.trim();

    !cell.is_empty() && cell.bytes().all(|byte| byte.is_ascii_digit())
}

fn table_column_alignments(
    rows: &[TableRowDraft<'_>],
    column_count: usize,
) -> Vec<TableColumnAlignment> {
    (0..column_count)
        .map(|column| {
            let mut has_data_rows = false;

            if rows.iter().all(|row| {
                if row.kind == TableRowKind::Separator {
                    return true;
                }
                has_data_rows = true;
                row.cells
                    .get(column)
                    .is_some_and(|cell| is_integer_table_cell(cell))
            }) && has_data_rows
            {
                TableColumnAlignment::Number
            } else {
                TableColumnAlignment::Text
            }
        })
        .collect()
}

fn build_table_block<'a>(
    header: &[&'a str],
    rows: &[TableRowDraft<'a>],
    props: Properties<'a>,
) -> Block<'a> {
    Block {
        kind: BlockKind::Table {
            header: build_table_row(TableRowKind::Data, header),
            alignments: table_column_alignments(rows, header.len()),
            rows: rows
                .iter()
                .map(|row| build_table_row(row.kind, &row.cells))
                .collect(),
        },
        props,
    }
}

fn build_block<'a>(draft: &BlockDraft<'a>, props: Properties<'a>) -> Block<'a> {
    match draft {
        BlockDraft::Property { .. } => panic!("No Property Block!"),
        BlockDraft::Heading { level, body } => Block {
            kind: BlockKind::Heading {
                level: *level,
                body: parse_inline(body),
                raw_body: body,
            },
            props,
        },
        BlockDraft::Code { raw_lines } => Block {
            kind: BlockKind::Code {
                lines: raw_lines.clone(),
                lang: props.get_one("lang"),
            },
            props,
        },
        BlockDraft::Paragraph { raw_lines } => Block {
            kind: BlockKind::Paragraph {
                body: parse_inlines(raw_lines),
            },
            props,
        },
        BlockDraft::Container {
            kind,
            args,
            raw_lines,
        } => Block {
            kind: BlockKind::Container {
                kind,
                args: args.clone(),
                lines: raw_lines.clone(),
            },
            props,
        },
        BlockDraft::Quote { raw_lines } => Block {
            kind: BlockKind::Quote {
                lines: raw_lines.clone(),
            },
            props,
        },
        BlockDraft::Table { header, rows } => build_table_block(header, rows, props),
        BlockDraft::List { items } => Block {
            kind: BlockKind::List {
                items: items.iter().map(build_list_item).collect(),
            },
            props,
        },
        BlockDraft::ReferenceDefinition { definitions } => Block {
            kind: BlockKind::ReferenceDefinition {
                definitions: definitions.iter().map(build_reference_definition).collect(),
            },
            props,
        },
    }
}
