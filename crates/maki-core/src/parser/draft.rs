use super::diagnostic::{ParseDiagnostic, ParseDiagnosticKind};
use super::line::{LinePrefix, LineToken, scan_line, scan_line_at};
use super::types::{ListKind, ReferenceDefinitionSpelling, TableRowKind, TodoState};
use crate::source::SourceSpan;
use std::collections::BTreeSet;

#[derive(Debug, PartialEq)]
pub(super) enum PropertyKind {
    Previous,
    Next,
}

#[derive(Debug, PartialEq)]
pub(super) struct PropertyItemDraft<'a> {
    pub(super) key: &'a str,
    pub(super) value: &'a str,
}

#[derive(Debug, PartialEq)]
pub(super) struct ReferenceDefinitionDraft<'a> {
    pub(super) line: usize,
    pub(super) span: SourceSpan,
    pub(super) raw_line: &'a str,
    pub(super) key: &'a str,
    pub(super) raw_value: &'a str,
    pub(super) spelling: ReferenceDefinitionSpelling,
}

impl<'a> PropertyItemDraft<'a> {
    pub(super) fn new(key: &'a str, value: &'a str) -> Self {
        PropertyItemDraft { key, value }
    }
}

/// A draft of a block to be built into a [`Block`].
/// LineToken을 파싱하여 Block 구성하기 위한 정보를 모음.
/// Block과의 차이: BlockDraft는 아직 body를 파싱하지 않음
#[derive(Debug, PartialEq)]
pub(super) enum BlockDraft<'a> {
    /// --^, --v
    Property {
        line: usize,
        span: SourceSpan,
        raw_line: &'a str,
        indent: usize,
        kind: PropertyKind,
        items: Vec<PropertyItemDraft<'a>>,
    },
    /// =
    Heading {
        level: usize,
        body: &'a str,
    },

    /// 그 외 일반 텍스트
    Paragraph {
        raw_lines: Vec<&'a str>,
    },

    /// : prefixed
    Code {
        raw_lines: Vec<&'a str>,
    },

    /// --- <kind> [<args>]
    Container {
        kind: &'a str,
        args: Vec<&'a str>,
        raw_lines: Vec<&'a str>,
    },

    /// > prefixed
    Quote {
        raw_lines: Vec<&'a str>,
    },

    List {
        items: Vec<ListItemDraft<'a>>,
    },

    Table {
        header: Vec<&'a str>,
        rows: Vec<TableRowDraft<'a>>,
    },

    ReferenceDefinition {
        definitions: Vec<ReferenceDefinitionDraft<'a>>,
    },
}

#[derive(Debug, PartialEq)]
pub(super) struct TableRowDraft<'a> {
    pub(super) kind: TableRowKind,
    pub(super) cells: Vec<&'a str>,
}

#[derive(Debug, PartialEq)]
pub(super) struct ListItemDraft<'a> {
    pub(super) kind: ListKind,
    pub(super) todo: Option<TodoState>,
    pub(super) indent: usize,
    pub(super) body: &'a str,
    pub(super) children: Vec<BlockDraft<'a>>,
}

fn starts_block_after_paragraph(line: &LineToken<'_>) -> bool {
    let LineToken::Line { indent, kind, .. } = line else {
        return false;
    };

    match kind {
        LinePrefix::EnCaret | LinePrefix::EnV => true,
        LinePrefix::EqualsRun(level) => (1..=6).contains(level),
        LinePrefix::Hyphen
        | LinePrefix::NumberDot { .. }
        | LinePrefix::Colon
        | LinePrefix::Quote => *indent == 0,
        LinePrefix::HyphenFence(_) => *indent == 0,
        LinePrefix::Reference => *indent == 0 && parse_reference_definition_line(line, 0).is_some(),
        LinePrefix::None => false,
    }
}

struct LineCursor<'tokens, 'src> {
    lines: &'tokens [LineToken<'src>],
    pos: usize,
}

impl<'tokens, 'src> LineCursor<'tokens, 'src> {
    fn new(lines: &'tokens [LineToken<'src>]) -> Self {
        Self { lines, pos: 0 }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.lines.len()
    }

    fn peek(&self) -> Option<&LineToken<'src>> {
        self.lines.get(self.pos)
    }

    fn peek_after_leading_blanks(&self) -> Option<(usize, &LineToken<'src>)> {
        let mut index = self.pos;

        while matches!(self.lines.get(index), Some(LineToken::Blank { .. })) {
            index += 1;
        }

        self.lines.get(index).map(|line| (index, line))
    }

    fn line_number(&self) -> usize {
        self.pos + 1
    }

    fn next(&mut self) -> Option<&LineToken<'src>> {
        let line = self.lines.get(self.pos)?;
        self.pos += 1;
        Some(line)
    }

    fn consume_blank(&mut self) -> bool {
        if matches!(self.peek(), Some(LineToken::Blank { .. })) {
            self.next();
            true
        } else {
            false
        }
    }

    fn consume_blanks_before(&mut self, index: usize) {
        while self.pos < index && matches!(self.peek(), Some(LineToken::Blank { .. })) {
            self.next();
        }
    }
}

fn parse_paragraph_draft<'a>(cursor: &mut LineCursor<'_, 'a>) -> Option<BlockDraft<'a>> {
    let mut raw_lines = vec![];

    while !cursor.is_eof() {
        if cursor.consume_blank() {
            break;
        }
        if !raw_lines.is_empty()
            && (cursor.peek().is_some_and(starts_block_after_paragraph)
                || cursor_starts_table(cursor))
        {
            break;
        }
        raw_lines.push(paragraph_line(cursor.next()?));
    }

    Some(BlockDraft::Paragraph { raw_lines })
}

fn paragraph_line<'a>(line: &LineToken<'a>) -> &'a str {
    let LineToken::Line {
        indent: 0,
        kind: LinePrefix::None,
        raw_line,
        ..
    } = line
    else {
        return line.raw_line();
    };
    let Some(escaped) = raw_line.strip_prefix('\\') else {
        return raw_line;
    };
    let escaped_token = scan_line(escaped);
    let escapes_block_start = starts_block_after_paragraph(&escaped_token)
        || matches!(escaped_token, LineToken::Line { raw_line, .. } if raw_line.starts_with('|'));

    if escapes_block_start {
        escaped
    } else {
        raw_line
    }
}

fn parse_container_header(header: &str) -> Option<(&str, Vec<&str>)> {
    let mut parts = header.split_whitespace();
    let kind = parts.next().unwrap_or("");
    if !kind
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return None;
    }
    let args = parts.collect();

    Some((kind, args))
}

fn is_closing_fence(line: &LineToken<'_>, len: usize) -> bool {
    matches!(
        line,
        LineToken::Line {
            indent: 0,
            kind: LinePrefix::HyphenFence(line_len),
            ..
        } if *line_len == len && line.body() == Some("")
    )
}

fn is_root_line_with_prefix(line: &LineToken<'_>, expected: LinePrefix) -> bool {
    matches!(
        line,
        LineToken::Line {
            indent: 0,
            kind,
            ..
        } if *kind == expected
    )
}

fn parse_container_draft<'a>(
    cursor: &mut LineCursor<'_, 'a>,
    diagnostics: &mut Vec<ParseDiagnostic<'a>>,
) -> Option<BlockDraft<'a>> {
    let Some(LineToken::Line {
        kind: LinePrefix::HyphenFence(fence_len),
        indent: 0,
        ..
    }) = cursor.peek()
    else {
        return None;
    };

    let mut raw_lines = vec![];
    let line = cursor.line_number();
    let raw_line = cursor.peek()?.raw_line();
    let span = cursor.peek()?.span();
    let header = cursor.peek()?.body()?.trim();
    let (kind, args) = parse_container_header(header)?;
    let fence_len = *fence_len;
    cursor.next();
    let mut closed = false;

    while let Some(line) = cursor.next() {
        if is_closing_fence(line, fence_len) {
            closed = true;
            break;
        }
        raw_lines.push(line.raw_line());
    }

    if !closed {
        diagnostics.push(ParseDiagnostic {
            line,
            span,
            kind: ParseDiagnosticKind::UnclosedContainer { raw_line },
        });
    }

    Some(BlockDraft::Container {
        kind,
        args,
        raw_lines,
    })
}

// TODO: parse.. 함수들 모두 Result<Option<T>, E> 타입으로 바꾸기. Ok(None), Ok(Some(...)), Err(..)
fn parse_property_draft<'a>(
    cursor: &mut LineCursor<'_, 'a>,
    diagnostics: &mut Vec<ParseDiagnostic<'a>>,
) -> Option<BlockDraft<'a>> {
    let LineToken::Line { kind, indent, .. } = cursor.peek()? else {
        return None;
    };
    let property_kind = kind.as_property_kind()?;
    let line = cursor.line_number();
    let raw_line = cursor.peek()?.raw_line();
    let span = cursor.peek()?.span();
    let kind = *kind;
    let indent = *indent;
    let mut items = vec![];

    while let Some(LineToken::Line {
        kind: line_kind,
        indent: line_indent,
        ..
    }) = cursor.peek()
    {
        if *line_indent != indent || kind != *line_kind {
            break;
        }
        let line = cursor.line_number();
        let token = cursor.next()?;
        let raw_line = token.raw_line();
        let span = token.span();
        let body = token.body()?;

        let Some((key, value)) = body.split_once(':') else {
            diagnostics.push(ParseDiagnostic {
                line,
                span,
                kind: ParseDiagnosticKind::InvalidProperty { raw_line },
            });
            continue;
        };

        items.push(PropertyItemDraft::new(key.trim(), value.trim()))
    }

    Some(BlockDraft::Property {
        line,
        span,
        raw_line,
        indent,
        kind: property_kind,
        items,
    })
}

fn valid_reference_identifier(identifier: &str) -> bool {
    !identifier.is_empty() && !identifier.contains(['[', ']'])
}

fn parse_reference_definition_line<'a>(
    token: &LineToken<'a>,
    line: usize,
) -> Option<ReferenceDefinitionDraft<'a>> {
    let LineToken::Line {
        indent: 0,
        kind: LinePrefix::Reference,
        raw_line,
        ..
    } = token
    else {
        return None;
    };

    let after_open = raw_line.strip_prefix('[')?;
    let close = after_open.find(']')?;
    let identifier = &after_open[..close];
    let value = after_open[close + 1..].strip_prefix(':')?;
    let value = value.trim_start();

    let (key, raw_value, spelling) = if let Some(label) = identifier.strip_prefix('^') {
        if !valid_reference_identifier(label) || label.chars().any(char::is_whitespace) {
            return None;
        }
        (label, value, ReferenceDefinitionSpelling::FootnoteAlias)
    } else {
        if !valid_reference_identifier(identifier) || identifier.starts_with('^') {
            return None;
        }
        let target = value.trim();
        if target.is_empty() {
            return None;
        }
        (identifier, target, ReferenceDefinitionSpelling::Canonical)
    };

    Some(ReferenceDefinitionDraft {
        line,
        span: token.span(),
        raw_line,
        key,
        raw_value,
        spelling,
    })
}

fn parse_reference_definition_draft<'a>(cursor: &mut LineCursor<'_, 'a>) -> Option<BlockDraft<'a>> {
    parse_reference_definition_line(cursor.peek()?, cursor.line_number())?;
    let mut definitions = vec![];

    while let Some(definition) = cursor
        .peek()
        .and_then(|token| parse_reference_definition_line(token, cursor.line_number()))
    {
        cursor.next();
        definitions.push(definition);
    }

    Some(BlockDraft::ReferenceDefinition { definitions })
}

fn parse_heading_draft<'a>(cursor: &mut LineCursor<'_, 'a>) -> Option<BlockDraft<'a>> {
    let line = cursor.peek()?;
    let LineToken::Line {
        kind: LinePrefix::EqualsRun(level),
        ..
    } = line
    else {
        return None;
    };
    let level = *level;
    let body = line.body()?;

    if !(1..=6).contains(&level) {
        return None;
    }

    cursor.next();

    Some(BlockDraft::Heading { level, body })
}

fn list_marker(line: &LineToken<'_>) -> Option<(ListKind, usize, usize)> {
    let LineToken::Line {
        indent,
        kind: line_kind,
        ..
    } = line
    else {
        return None;
    };
    let kind = match line_kind {
        LinePrefix::Hyphen => ListKind::Unordered,
        LinePrefix::NumberDot { .. } => ListKind::Ordered,
        _ => return None,
    };

    Some((kind, *indent, *indent + line_kind.width()))
}

fn is_list_marker_at(line: &LineToken<'_>, indent: usize, kind: ListKind) -> bool {
    list_marker(line)
        .is_some_and(|(line_kind, line_indent, _)| line_indent == indent && line_kind == kind)
}

fn line_is_indented_at_least(line: &LineToken<'_>, indent: usize) -> bool {
    matches!(line, LineToken::Line { indent: line_indent, .. } if *line_indent >= indent)
}

fn strip_line_indent<'a>(line: &LineToken<'a>, indent: usize) -> LineToken<'a> {
    match line {
        LineToken::Blank { span, .. } => LineToken::Blank {
            raw_line: "",
            span: *span,
        },
        LineToken::Line {
            raw_line,
            indent: line_indent,
            ..
        } => {
            debug_assert!(*line_indent >= indent);
            let span = line.span();
            scan_line_at(
                &raw_line[indent..],
                SourceSpan::new(span.start + indent, span.end),
            )
        }
    }
}

fn parse_list_item_child_drafts<'a>(
    cursor: &mut LineCursor<'_, 'a>,
    content_indent: usize,
) -> Vec<BlockDraft<'a>> {
    let mut child_lines = vec![];

    while let Some((next_index, next_line)) = cursor.peek_after_leading_blanks() {
        if !line_is_indented_at_least(next_line, content_indent) {
            break;
        }

        while cursor.pos < next_index {
            let blank_span = cursor
                .next()
                .expect("peeked blank line should be available")
                .span();
            if !child_lines.is_empty() {
                child_lines.push(LineToken::Blank {
                    raw_line: "",
                    span: blank_span,
                });
            }
        }

        while let Some(line) = cursor.peek() {
            if matches!(line, LineToken::Blank { .. })
                || !line_is_indented_at_least(line, content_indent)
            {
                break;
            }

            let line = cursor
                .next()
                .expect("peeked list child line should be available");
            child_lines.push(strip_line_indent(line, content_indent));
        }
    }

    let mut diagnostics = vec![];
    build_drafts(&child_lines, &mut diagnostics)
}

fn parse_list_item_draft<'a>(
    cursor: &mut LineCursor<'_, 'a>,
) -> Option<(ListItemDraft<'a>, usize)> {
    let line = cursor.peek()?;
    let (kind, indent, content_indent) = list_marker(line)?;
    let body = line.body()?;
    let (todo, body) = parse_todo_item(kind, body);

    cursor.next();

    Some((
        ListItemDraft {
            kind,
            todo,
            indent,
            body,
            children: vec![],
        },
        content_indent,
    ))
}

fn parse_todo_item(kind: ListKind, body: &str) -> (Option<TodoState>, &str) {
    if kind != ListKind::Unordered {
        return (None, body);
    }

    for (marker, state) in [("[ ]", TodoState::Todo), ("[x]", TodoState::Done)] {
        let Some(rest) = body.strip_prefix(marker) else {
            continue;
        };

        if rest.is_empty() {
            return (Some(state), rest);
        }
        if let Some(body) = rest.strip_prefix(' ') {
            return (Some(state), body);
        }
    }

    (None, body)
}

fn parse_list_draft<'a>(cursor: &mut LineCursor<'_, 'a>) -> Option<BlockDraft<'a>> {
    let (list_kind, list_indent, _) = list_marker(cursor.peek()?)?;

    if list_indent != 0 {
        return None;
    }

    let mut items = vec![];

    while cursor
        .peek()
        .is_some_and(|line| is_list_marker_at(line, list_indent, list_kind))
    {
        let (mut item, content_indent) = parse_list_item_draft(cursor)?;
        item.children = parse_list_item_child_drafts(cursor, content_indent);
        items.push(item);

        if list_kind == ListKind::Ordered
            && let Some((next_index, next_line)) = cursor.peek_after_leading_blanks()
            && is_list_marker_at(next_line, list_indent, list_kind)
        {
            cursor.consume_blanks_before(next_index);
        }
    }

    Some(BlockDraft::List { items })
}

fn table_line_body<'a>(line: &LineToken<'a>) -> Option<&'a str> {
    let LineToken::Line {
        indent: 0,
        raw_line,
        ..
    } = line
    else {
        return None;
    };

    raw_line.trim_end().strip_prefix('|')?.strip_suffix('|')
}

fn parse_table_row_cells<'a>(line: &LineToken<'a>) -> Option<Vec<&'a str>> {
    let body = table_line_body(line)?;
    let mut cells = vec![];
    let mut start = 0;
    let mut escaped = false;

    for (index, ch) in body.char_indices() {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '|' {
            cells.push(body[start..index].trim());
            start = index + ch.len_utf8();
        }
    }
    cells.push(body[start..].trim());
    Some(cells)
}

fn is_table_separator_part(part: &str) -> bool {
    let part = part.trim();

    part.len() >= 3 && part.bytes().all(|byte| byte == b'-')
}

fn parse_table_separator_columns(line: &LineToken<'_>) -> Option<usize> {
    let body = table_line_body(line)?;
    let delimiter = match (body.contains('+'), body.contains('|')) {
        (true, false) => '+',
        (false, _) => '|',
        (true, true) => return None,
    };
    let parts = body.split(delimiter).collect::<Vec<_>>();

    parts
        .iter()
        .all(|part| is_table_separator_part(part))
        .then_some(parts.len())
}

fn parse_table_start<'a>(cursor: &LineCursor<'_, 'a>) -> Option<(Vec<&'a str>, usize)> {
    let header = cursor.peek().and_then(parse_table_row_cells)?;
    let separator_columns = cursor
        .lines
        .get(cursor.pos + 1)
        .and_then(parse_table_separator_columns)?;

    (header.len() == separator_columns).then_some((header, separator_columns))
}

fn cursor_starts_table(cursor: &LineCursor<'_, '_>) -> bool {
    parse_table_start(cursor).is_some()
}

fn parse_table_draft<'a>(cursor: &mut LineCursor<'_, 'a>) -> Option<BlockDraft<'a>> {
    let (header, separator_columns) = parse_table_start(cursor)?;

    cursor.next();
    cursor.next();

    let mut rows = vec![];
    while let Some(line) = cursor.peek() {
        if parse_table_separator_columns(line) == Some(separator_columns) {
            cursor.next();
            rows.push(TableRowDraft {
                kind: TableRowKind::Separator,
                cells: vec![],
            });
            continue;
        }
        let Some(cells) = parse_table_row_cells(line) else {
            break;
        };

        cursor.next();
        rows.push(TableRowDraft {
            kind: TableRowKind::Data,
            cells,
        });
    }

    Some(BlockDraft::Table { header, rows })
}

fn parse_root_prefixed_body_lines<'a>(
    cursor: &mut LineCursor<'_, 'a>,
    prefix: LinePrefix,
) -> Option<Vec<&'a str>> {
    if !cursor
        .peek()
        .is_some_and(|line| is_root_line_with_prefix(line, prefix))
    {
        return None;
    };

    let mut raw_lines = vec![];

    while cursor
        .peek()
        .is_some_and(|line| is_root_line_with_prefix(line, prefix))
    {
        raw_lines.push(cursor.next()?.body()?);
    }

    Some(raw_lines)
}

fn parse_code_draft<'a>(cursor: &mut LineCursor<'_, 'a>) -> Option<BlockDraft<'a>> {
    Some(BlockDraft::Code {
        raw_lines: parse_root_prefixed_body_lines(cursor, LinePrefix::Colon)?,
    })
}

fn parse_quote_draft<'a>(cursor: &mut LineCursor<'_, 'a>) -> Option<BlockDraft<'a>> {
    Some(BlockDraft::Quote {
        raw_lines: parse_root_prefixed_body_lines(cursor, LinePrefix::Quote)?,
    })
}

pub(super) fn build_drafts<'a>(
    lines: &[LineToken<'a>],
    diagnostics: &mut Vec<ParseDiagnostic<'a>>,
) -> Vec<BlockDraft<'a>> {
    let mut cursor = LineCursor::new(lines);
    let mut drafts = vec![];

    while !cursor.is_eof() {
        let draft = if let Some(draft) = parse_container_draft(&mut cursor, diagnostics) {
            Some(draft)
        } else if let Some(draft) = parse_code_draft(&mut cursor) {
            Some(draft)
        } else if let Some(draft) = parse_quote_draft(&mut cursor) {
            Some(draft)
        } else if let Some(draft) = parse_reference_definition_draft(&mut cursor) {
            Some(draft)
        } else if let Some(draft) = parse_property_draft(&mut cursor, diagnostics) {
            Some(draft)
        } else if let Some(draft) = parse_heading_draft(&mut cursor) {
            Some(draft)
        } else if let Some(draft) = parse_list_draft(&mut cursor) {
            Some(draft)
        } else if let Some(draft) = parse_table_draft(&mut cursor) {
            Some(draft)
        } else if cursor.consume_blank() {
            None
        } else {
            parse_paragraph_draft(&mut cursor)
        };

        let Some(draft) = draft else {
            continue;
        };

        if let (
            BlockDraft::Property {
                line,
                span,
                raw_line,
                ..
            },
            Some(BlockDraft::Property { .. }),
        ) = (&draft, drafts.last())
        {
            diagnostics.push(ParseDiagnostic {
                line: *line,
                span: *span,
                kind: ParseDiagnosticKind::PropertyOnProperty { raw_line },
            });
            continue;
        }

        drafts.push(draft);
    }

    drafts
}

fn collect_duplicate_reference_diagnostics<'a>(
    drafts: &[BlockDraft<'a>],
    diagnostics: &mut Vec<ParseDiagnostic<'a>>,
) {
    let mut keys = BTreeSet::new();

    for draft in drafts {
        let BlockDraft::ReferenceDefinition { definitions } = draft else {
            continue;
        };

        for definition in definitions {
            if !keys.insert(definition.key) {
                diagnostics.push(ParseDiagnostic {
                    line: definition.line,
                    span: definition.span,
                    kind: ParseDiagnosticKind::DuplicateReferenceDefinition {
                        raw_line: definition.raw_line,
                    },
                });
            }
        }
    }
}

pub(super) fn parse_drafts<'a>(
    lines: &[LineToken<'a>],
) -> (Vec<BlockDraft<'a>>, Vec<ParseDiagnostic<'a>>) {
    let mut diagnostics = vec![];
    let drafts = build_drafts(lines, &mut diagnostics);
    collect_duplicate_reference_diagnostics(&drafts, &mut diagnostics);
    diagnostics.sort_by_key(|diagnostic| diagnostic.line);

    (drafts, diagnostics)
}
