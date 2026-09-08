use crate::parser::{self, Block, BlockKind, ListItem, ReferenceDefinitions};
use crate::source::{SourceSpan, slice_span};

#[derive(Debug)]
pub(crate) struct MappedSource {
    pub(crate) text: String,
    segments: Vec<MappedSourceSegment>,
}

#[derive(Debug, Clone, Copy)]
struct MappedSourceSegment {
    synthetic: SourceSpan,
    root: SourceSpan,
}

impl MappedSource {
    fn from_lines(
        current_source: &str,
        lines: &[&str],
        current_coordinates: Option<&Self>,
    ) -> Option<Self> {
        let mut text = String::new();
        let mut segments = Vec::with_capacity(lines.len());

        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            let start = text.len();
            text.push_str(line);
            let current_span = slice_span(current_source, line)?;
            let root = match current_coordinates {
                Some(coordinates) => coordinates.map_span(current_span)?,
                None => current_span,
            };
            segments.push(MappedSourceSegment {
                synthetic: SourceSpan::new(start, text.len()),
                root,
            });
        }

        Some(Self { text, segments })
    }

    pub(crate) fn map_span(&self, span: SourceSpan) -> Option<SourceSpan> {
        let touches = |segment: &&MappedSourceSegment| {
            if span.start == span.end {
                segment.synthetic.start <= span.start && span.start <= segment.synthetic.end
            } else {
                segment.synthetic.start < span.end && span.start < segment.synthetic.end
            }
        };
        let first = self.segments.iter().find(touches)?;
        let last = self.segments.iter().rev().find(touches)?;
        let start = first.root.start
            + span
                .start
                .saturating_sub(first.synthetic.start)
                .min(first.synthetic.end - first.synthetic.start);
        let end = last.root.start
            + span
                .end
                .saturating_sub(last.synthetic.start)
                .min(last.synthetic.end - last.synthetic.start);
        Some(SourceSpan::new(start, end))
    }
}

pub(crate) trait NestedDocumentObserver {
    fn enter(&mut self, coordinates: &MappedSource, parsed: &parser::ParseResult<'_>);
    fn exit(&mut self);
}

pub(crate) trait NestedTraversalVisitor {
    type Context: Clone;

    fn enter_document(&mut self, document: &parser::Document<'_>, context: &mut Self::Context);

    fn visit_block(
        &mut self,
        block: &Block<'_>,
        references: &ReferenceDefinitions<'_>,
        context: &mut Self::Context,
    );

    fn enter_list_item(
        &mut self,
        item: &ListItem<'_>,
        references: &ReferenceDefinitions<'_>,
        context: &Self::Context,
    ) -> Self::Context;

    fn exit_document(&mut self, document: &parser::Document<'_>, context: &mut Self::Context);
}

struct ObserverOnlyTraversal;

impl NestedTraversalVisitor for ObserverOnlyTraversal {
    type Context = ();

    fn enter_document(&mut self, _document: &parser::Document<'_>, _context: &mut Self::Context) {}

    fn visit_block(
        &mut self,
        _block: &Block<'_>,
        _references: &ReferenceDefinitions<'_>,
        _context: &mut Self::Context,
    ) {
    }

    fn enter_list_item(
        &mut self,
        _item: &ListItem<'_>,
        _references: &ReferenceDefinitions<'_>,
        context: &Self::Context,
    ) -> Self::Context {
        *context
    }

    fn exit_document(&mut self, _document: &parser::Document<'_>, _context: &mut Self::Context) {}
}

pub(crate) fn traverse_nested_documents(
    source: &str,
    parsed: &parser::ParseResult<'_>,
    observer: &mut dyn NestedDocumentObserver,
) {
    traverse_parsed_document(source, parsed, (), &mut ObserverOnlyTraversal, observer);
}

pub(crate) fn traverse_parsed_document<V: NestedTraversalVisitor>(
    source: &str,
    parsed: &parser::ParseResult<'_>,
    mut context: V::Context,
    visitor: &mut V,
    observer: &mut dyn NestedDocumentObserver,
) {
    Traversal { visitor, observer }.document(source, parsed, None, &mut context);
}

struct Traversal<'a, V> {
    visitor: &'a mut V,
    observer: &'a mut dyn NestedDocumentObserver,
}

impl<V: NestedTraversalVisitor> Traversal<'_, V> {
    fn document(
        &mut self,
        source: &str,
        parsed: &parser::ParseResult<'_>,
        coordinates: Option<&MappedSource>,
        context: &mut V::Context,
    ) {
        self.visitor.enter_document(&parsed.document, context);
        self.blocks(
            source,
            &parsed.document.blocks,
            parsed.document.reference_definitions(),
            coordinates,
            context,
        );
        self.visitor.exit_document(&parsed.document, context);
    }

    fn blocks(
        &mut self,
        source: &str,
        blocks: &[Block<'_>],
        references: &ReferenceDefinitions<'_>,
        coordinates: Option<&MappedSource>,
        context: &mut V::Context,
    ) {
        for block in blocks {
            self.visitor.visit_block(block, references, context);
            match &block.kind {
                BlockKind::List { items } => {
                    for item in items {
                        let mut item_context =
                            self.visitor.enter_list_item(item, references, context);
                        self.blocks(
                            source,
                            &item.children,
                            references,
                            coordinates,
                            &mut item_context,
                        );
                    }
                }
                BlockKind::Quote { lines } if !quote_mode_is_raw(block.property("mode")) => {
                    self.nested_lines(source, lines, references, coordinates, context);
                }
                BlockKind::Container { kind, lines, .. }
                    if *kind == "quote" && !quote_mode_is_raw(block.property("mode")) =>
                {
                    self.nested_lines(source, lines, references, coordinates, context);
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
    }

    fn nested_lines(
        &mut self,
        source: &str,
        lines: &[&str],
        references: &ReferenceDefinitions<'_>,
        coordinates: Option<&MappedSource>,
        context: &V::Context,
    ) {
        let Some(mapped) = MappedSource::from_lines(source, lines, coordinates) else {
            return;
        };
        let parsed = parser::parse_with_references(&mapped.text, references);
        self.observer.enter(&mapped, &parsed);
        let mut nested_context = context.clone();
        self.document(&mapped.text, &parsed, Some(&mapped), &mut nested_context);
        self.observer.exit();
    }
}

fn quote_mode_is_raw(mode: Option<&str>) -> bool {
    matches!(mode, Some("pre" | "text"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingObserver {
        documents: Vec<String>,
        open_documents: usize,
    }

    impl NestedDocumentObserver for RecordingObserver {
        fn enter(&mut self, coordinates: &MappedSource, _parsed: &parser::ParseResult<'_>) {
            self.documents.push(coordinates.text.clone());
            self.open_documents += 1;
        }

        fn exit(&mut self) {
            self.open_documents -= 1;
        }
    }

    #[test]
    fn traversal_owns_quote_container_recursion_and_raw_mode_filtering() {
        let source = r#"> outer
> > deep

--v mode: pre
> raw

--- quote
container
---"#;
        let parsed = parser::parse(source);
        let mut observer = RecordingObserver::default();

        traverse_nested_documents(source, &parsed, &mut observer);

        assert_eq!(
            observer.documents,
            vec!["outer\n> deep", "deep", "container"]
        );
        assert_eq!(observer.open_documents, 0);
    }
}
