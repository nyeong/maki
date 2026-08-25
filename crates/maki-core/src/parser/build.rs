use super::draft::{
    BlockDraft, ListItemDraft, PropertyKind, ReferenceDefinitionDraft,
    ReferenceDefinitionDraftKind, TableRowDraft,
};
use super::inline::{ReferenceLookup, parse_inline_with_references, parse_inlines_with_references};
use super::types::{
    Block, BlockKind, Document, ListItem, Properties, ReferenceDefinition, ReferenceDefinitions,
    TableCell, TableColumnAlignment, TableRow, TableRowKind,
};

fn collect_document_references<'draft, 'source, 'parent>(
    drafts: &'draft [BlockDraft<'source>],
    inherited: Option<&ReferenceDefinitions<'parent>>,
) -> (
    ReferenceLookup<'source>,
    Vec<&'draft ReferenceDefinitionDraft<'source>>,
)
where
    'parent: 'source,
{
    let mut lookup = ReferenceLookup::default();
    let mut unique_definitions = Vec::new();

    for draft in drafts {
        let BlockDraft::ReferenceDefinition {
            definitions: block_definitions,
        } = draft
        else {
            continue;
        };

        for definition in block_definitions {
            let is_first = match definition.kind {
                ReferenceDefinitionDraftKind::Link { title, target } => {
                    lookup.insert_link(title, target)
                }
                ReferenceDefinitionDraftKind::Footnote { label, .. } => {
                    lookup.insert_footnote(label)
                }
            };

            if is_first {
                unique_definitions.push(definition);
            }
        }
    }

    if let Some(inherited) = inherited {
        lookup.extend(inherited);
    }

    (lookup, unique_definitions)
}

fn build_reference_definition<'a>(
    draft: &ReferenceDefinitionDraft<'a>,
    references: &ReferenceLookup<'a>,
) -> ReferenceDefinition<'a> {
    match draft.kind {
        ReferenceDefinitionDraftKind::Link { title, target } => {
            ReferenceDefinition::Link { title, target }
        }
        ReferenceDefinitionDraftKind::Footnote { label, body } => ReferenceDefinition::Footnote {
            label,
            body: parse_inline_with_references(body, references),
            raw_body: body,
        },
    }
}

fn build_reference_definitions<'a>(
    drafts: &[&ReferenceDefinitionDraft<'a>],
    references: &ReferenceLookup<'a>,
) -> ReferenceDefinitions<'a> {
    let definitions = drafts
        .iter()
        .map(|definition| build_reference_definition(definition, references))
        .collect();

    ReferenceDefinitions::new(definitions)
}

fn build_blocks<'a>(drafts: &[BlockDraft<'a>], references: &ReferenceLookup<'a>) -> Vec<Block<'a>> {
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
                let block = build_block(draft, std::mem::take(&mut pending_props), references);
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
    let (reference_lookup, reference_drafts) = collect_document_references(drafts, inherited);
    let local_references = build_reference_definitions(&reference_drafts, &reference_lookup);
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
                let block =
                    build_block(draft, std::mem::take(&mut pending_props), &reference_lookup);
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

fn build_list_item<'a>(
    draft: &ListItemDraft<'a>,
    references: &ReferenceLookup<'a>,
) -> ListItem<'a> {
    ListItem {
        body: parse_inline_with_references(draft.body, references),
        kind: draft.kind,
        todo: draft.todo,
        children: build_blocks(&draft.children, references),
    }
}

fn build_table_row<'a>(
    kind: TableRowKind,
    cells: &[&'a str],
    references: &ReferenceLookup<'a>,
) -> TableRow<'a> {
    TableRow {
        kind,
        cells: cells
            .iter()
            .map(|cell| TableCell {
                body: parse_inline_with_references(cell, references),
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
    references: &ReferenceLookup<'a>,
) -> Block<'a> {
    Block {
        kind: BlockKind::Table {
            header: build_table_row(TableRowKind::Data, header, references),
            alignments: table_column_alignments(rows, header.len()),
            rows: rows
                .iter()
                .map(|row| build_table_row(row.kind, &row.cells, references))
                .collect(),
        },
        props,
    }
}

fn build_block<'a>(
    draft: &BlockDraft<'a>,
    props: Properties<'a>,
    references: &ReferenceLookup<'a>,
) -> Block<'a> {
    match draft {
        BlockDraft::Property { .. } => panic!("No Property Block!"),
        BlockDraft::Heading { level, body } => Block {
            kind: BlockKind::Heading {
                level: *level,
                body: parse_inline_with_references(body, references),
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
                body: parse_inlines_with_references(raw_lines, references),
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
        BlockDraft::Table { header, rows } => build_table_block(header, rows, props, references),
        BlockDraft::List { items } => Block {
            kind: BlockKind::List {
                items: items
                    .iter()
                    .map(|item| build_list_item(item, references))
                    .collect(),
            },
            props,
        },
        BlockDraft::ReferenceDefinition { definitions } => Block {
            kind: BlockKind::ReferenceDefinition {
                definitions: definitions
                    .iter()
                    .map(|definition| build_reference_definition(definition, references))
                    .collect(),
            },
            props,
        },
    }
}
