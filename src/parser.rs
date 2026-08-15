//! Maki parser
//!
//! ### Properties
//!
//! 문법적으로, 각 property에는 별 의미가 없다.
//! 그러나 maki, renderer 등에서 특별한 의미를 담을 수도 있다.

use std::collections::BTreeMap;

pub(crate) fn parse(source: &str) -> ParseResult<'_> {
    let lines = scan_lines(source);
    let mut diagnostics = vec![];
    let drafts = build_drafts(&lines, &mut diagnostics);
    let document = build_documents(&drafts);

    ParseResult {
        document,
        diagnostics,
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct ParseResult<'a> {
    pub(crate) document: Document<'a>,
    pub(crate) diagnostics: Vec<ParseDiagnostic<'a>>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct ParseDiagnostic<'a> {
    pub(crate) line: usize,
    pub(crate) kind: ParseDiagnosticKind<'a>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum ParseDiagnosticKind<'a> {
    InvalidProperty { raw_line: &'a str },
    UnclosedContainer { raw_line: &'a str },
}

#[derive(Debug, PartialEq)]
pub(crate) enum Inline<'a> {
    NoteLink { target: &'a str },
    Link { title: &'a str, target: &'a str },
    Text(&'a str),
    SoftBreak,
    Code(&'a str),
}

struct InlineCursor<'a> {
    source: &'a str,
    pos: usize,
}

impl<'a> InlineCursor<'a> {
    fn new(source: &'a str) -> Self {
        Self { source, pos: 0 }
    }

    fn pos(&self) -> usize {
        self.pos
    }

    fn is_eol(&self) -> bool {
        self.pos >= self.source.len()
    }

    fn rest(&self) -> &'a str {
        &self.source[self.pos..]
    }

    fn bump(&mut self, n: usize) {
        self.pos += n;
    }

    fn bump_char(&mut self) {
        if let Some(ch) = self.rest().chars().next() {
            self.pos += ch.len_utf8();
        }
    }
}

const INLINE_NOTE_LINK_BEGIN: &str = "[[";
const INLINE_NOTE_LINK_END: &str = "]]";
const INLINE_CODE_BEGIN_END: &str = "`";
const INLINE_LINK_BEGIN: &str = "[";
const INLINE_LINK_SEPARATOR: &str = "](";
const INLINE_LINK_END: &str = ")";

fn parse_inline_code<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    let rest = cursor.rest();
    let body = rest.strip_prefix(INLINE_CODE_BEGIN_END)?;
    let end = body.find(INLINE_CODE_BEGIN_END)?;

    let contents = &body[..end];

    cursor.bump(INLINE_CODE_BEGIN_END.len() + contents.len() + INLINE_CODE_BEGIN_END.len());

    Some(Inline::Code(contents))
}

fn parse_inline_note_link<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    let rest = cursor.rest();
    let body = rest.strip_prefix(INLINE_NOTE_LINK_BEGIN)?;
    let end = body.find(INLINE_NOTE_LINK_END)?;

    let target = &body[..end];

    cursor.bump(INLINE_NOTE_LINK_BEGIN.len() + target.len() + INLINE_NOTE_LINK_END.len());

    Some(Inline::NoteLink { target })
}

fn parse_inline_link<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    let rest = cursor.rest();

    if rest.starts_with(INLINE_NOTE_LINK_BEGIN) {
        return None;
    }

    let body = rest.strip_prefix(INLINE_LINK_BEGIN)?;
    let title_end = body.find(INLINE_LINK_SEPARATOR)?;
    let title = &body[..title_end];
    let target_body = &body[title_end + INLINE_LINK_SEPARATOR.len()..];
    let target_end = target_body.find(INLINE_LINK_END)?;
    let target = &target_body[..target_end];

    if title.is_empty() || target.is_empty() {
        return None;
    }

    cursor.bump(
        INLINE_LINK_BEGIN.len()
            + title.len()
            + INLINE_LINK_SEPARATOR.len()
            + target.len()
            + INLINE_LINK_END.len(),
    );

    Some(Inline::Link { title, target })
}

fn parse_inlines<'a>(source: &[&'a str]) -> Vec<Inline<'a>> {
    let mut inlines = vec![];

    for (index, line) in source.iter().enumerate() {
        if index > 0 {
            inlines.push(Inline::SoftBreak);
        }
        inlines.extend(parse_inline(line));
    }

    inlines
}

/// Parses a given line into Vec<Inline>
pub(crate) fn parse_inline<'a>(source: &'a str) -> Vec<Inline<'a>> {
    let mut cursor = InlineCursor::new(source);
    let mut inlines = vec![];
    let mut text_start = 0;

    while !cursor.is_eol() {
        let start = cursor.pos();

        if let Some(inline) = parse_inline_code(&mut cursor)
            .or_else(|| parse_inline_note_link(&mut cursor))
            .or_else(|| parse_inline_link(&mut cursor))
        {
            if text_start < start {
                inlines.push(Inline::Text(&source[text_start..start]));
            }

            inlines.push(inline);
            text_start = cursor.pos();
        } else {
            cursor.bump_char();
        }
    }

    if text_start < source.len() {
        inlines.push(Inline::Text(&source[text_start..]));
    }

    inlines
}

pub(crate) fn format_parse_diagnostic_kind(kind: &ParseDiagnosticKind<'_>) -> String {
    match kind {
        ParseDiagnosticKind::InvalidProperty { raw_line } => {
            format!("invalid property: {raw_line}")
        }
        ParseDiagnosticKind::UnclosedContainer { raw_line } => {
            format!("unclosed container block: {raw_line}")
        }
    }
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

fn build_documents<'a>(drafts: &[BlockDraft<'a>]) -> Document<'a> {
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
        blocks,
    }
}

fn build_list_item<'a>(draft: &ListItemDraft<'a>) -> ListItem<'a> {
    ListItem {
        body: parse_inline(draft.body),
        kind: draft.kind,
        children: build_blocks(&draft.children),
    }
}

fn build_list_items<'a>(items: &[ListItemDraft<'a>]) -> Vec<ListItem<'a>> {
    items.iter().map(build_list_item).collect()
}

fn build_list_block<'a>(draft: &BlockDraft<'a>, props: Properties<'a>) -> Option<Block<'a>> {
    let BlockDraft::List { items } = draft else {
        return None;
    };

    Some(Block {
        kind: BlockKind::List {
            items: build_list_items(items),
        },
        props,
    })
}

fn build_block<'a>(draft: &BlockDraft<'a>, props: Properties<'a>) -> Block<'a> {
    match draft {
        BlockDraft::Property { .. } => panic!("No Property Block!"),
        BlockDraft::Heading { level, body } => Block {
            kind: BlockKind::Heading {
                level: *level,
                body,
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
        BlockDraft::List { .. } => build_list_block(draft, props).unwrap(),
    }
}

#[derive(Debug, PartialEq, Default)]
struct Properties<'a> {
    values: BTreeMap<String, &'a str>,
}

impl<'a> Properties<'a> {
    fn new() -> Self {
        Self {
            values: BTreeMap::new(),
        }
    }

    // TODO: PropertyDraft만 받도록 바꾸기
    fn extend(&mut self, props: &[PropertyItemDraft<'a>]) {
        for prop in props {
            let key = prop.key.to_lowercase();
            let value = prop.value;
            self.values.insert(key, value);
        }
    }

    fn get_one(&self, key: &str) -> Option<&'a str> {
        self.values.get(key).copied()
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct Document<'a> {
    props: Properties<'a>,
    pub(crate) blocks: Vec<Block<'a>>,
}

impl<'a> Document<'a> {
    pub(crate) fn title(&self) -> Option<&'a str> {
        self.props.get_one("title")
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct Block<'a> {
    props: Properties<'a>,
    pub(crate) kind: BlockKind<'a>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum BlockKind<'a> {
    Paragraph {
        body: Vec<Inline<'a>>,
    },
    Code {
        lines: Vec<&'a str>,
        lang: Option<&'a str>,
    },
    Heading {
        level: usize,
        body: &'a str,
    },
    List {
        items: Vec<ListItem<'a>>,
    },
    Quote {
        lines: Vec<&'a str>,
    },
    Container {
        kind: &'a str,
        args: Vec<&'a str>,
        lines: Vec<&'a str>,
    },
}

#[derive(Debug, PartialEq)]
pub(crate) struct ListItem<'a> {
    pub(crate) body: Vec<Inline<'a>>,
    pub(crate) kind: ListKind,
    pub(crate) children: Vec<Block<'a>>, // List를 포함하기 위함
}

#[derive(Debug, Clone, PartialEq)]
enum LineToken<'a> {
    Blank {
        raw_line: &'a str,
    },
    Line {
        indent: usize,
        kind: LinePrefix,
        raw_line: &'a str,
    },
}

/// Run means a sequence of characters.
#[derive(Debug, Clone, Copy, PartialEq)]
enum LinePrefix {
    EqualsRun(usize), // #, ##, ###, ...
    EnCaret,          // --^
    EnV,              // --v
    Hyphen,           // -
    HyphenFence(usize),
    Colon,
    Quote,
    NumberDot { width: usize }, // 1.
    None,
}

fn scan_line(line: &str) -> LineToken<'_> {
    // TODO: 현재는 들여쓰기를 space만 지원하는데, 필요시 탭도 지원하도록
    let indent = line.chars().take_while(|&c| c == ' ').count();
    if line.trim().is_empty() {
        return LineToken::Blank { raw_line: line };
    }

    let prefix = scan_line_prefix(&line[indent..]);

    LineToken::Line {
        indent,
        kind: prefix,
        raw_line: line,
    }
}

const EN_CARET: &str = "--^ ";
const EN_V: &str = "--v ";
const HYPHEN: &str = "- ";
const COLON: char = ':';
const QUOTE: char = '>';
const EQUALS: char = '=';
const HYPHEN_FENCE_MIN_LEN: usize = 3;

fn parse_number_dot_prefix(source: &str) -> Option<usize> {
    let (digits, rest) = source.split_once('.')?;

    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) || !rest.starts_with(' ') {
        return None;
    }

    Some(digits.len() + ". ".len())
}

fn parse_hyphen_fence_prefix(source: &str) -> Option<usize> {
    let len = source.chars().take_while(|&c| c == '-').count();
    if len < HYPHEN_FENCE_MIN_LEN {
        return None;
    }

    let rest = &source[len..];
    (rest.is_empty() || rest.starts_with(' ')).then_some(len)
}

/// Accepts a text trimmed of leading whitespace.
fn scan_line_prefix(raw_text: &str) -> LinePrefix {
    if raw_text.starts_with(EN_CARET) {
        return LinePrefix::EnCaret;
    }
    if raw_text.starts_with(EN_V) {
        return LinePrefix::EnV;
    }
    if let Some(width) = parse_number_dot_prefix(raw_text) {
        return LinePrefix::NumberDot { width };
    }
    if let Some(len) = parse_hyphen_fence_prefix(raw_text) {
        return LinePrefix::HyphenFence(len);
    }
    if raw_text == ":" || raw_text.starts_with(": ") {
        return LinePrefix::Colon;
    }
    if raw_text == ">" || raw_text.starts_with("> ") {
        return LinePrefix::Quote;
    }
    if raw_text.starts_with(HYPHEN) {
        return LinePrefix::Hyphen;
    }
    if let Some(len) = count_prefix_run(raw_text, EQUALS, ' ') {
        return LinePrefix::EqualsRun(len);
    }

    LinePrefix::None
}

// prefix가 연속되고 마지막에 delimiter가 하나 나와야함
// 구성에 맞다면 Some(prefix의 개수), 구성에 맞지 않다면 None
fn count_prefix_run(raw_line: &str, prefix: char, delimiter: char) -> Option<usize> {
    let mut count = 0;

    for c in raw_line.chars() {
        if c == prefix {
            count += 1;
        } else if c == delimiter {
            break;
        } else {
            return None;
        }
    }
    (count > 0).then_some(count)
}

fn scan_lines(source: &str) -> Vec<LineToken<'_>> {
    source.lines().map(scan_line).collect()
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum ListKind {
    Unordered,
    Ordered,
}

#[derive(Debug, PartialEq)]
enum PropertyKind {
    Previous,
    Next,
}

#[derive(Debug, PartialEq)]
struct PropertyItemDraft<'a> {
    key: &'a str,
    value: &'a str,
}

impl<'a> PropertyItemDraft<'a> {
    fn new(key: &'a str, value: &'a str) -> Self {
        PropertyItemDraft { key, value }
    }
}

/// A draft of a block to be built into a [`Block`].
/// LineToken을 파싱하여 Block 구성하기 위한 정보를 모음.
/// Block과의 차이: BlockDraft는 아직 body를 파싱하지 않음
#[derive(Debug, PartialEq)]
enum BlockDraft<'a> {
    /// --^, --v
    Property {
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
}

#[derive(Debug, PartialEq)]
struct ListItemDraft<'a> {
    kind: ListKind,
    indent: usize,
    body: &'a str,
    children: Vec<BlockDraft<'a>>,
}

impl LinePrefix {
    fn as_property_kind(&self) -> Option<PropertyKind> {
        match self {
            LinePrefix::EnCaret => Some(PropertyKind::Previous),
            LinePrefix::EnV => Some(PropertyKind::Next),
            _ => None,
        }
    }

    /// Returns the width of the prefix in characters.
    /// It contains the whitespaces after the prefix which serve as the prefix's delimiter.
    fn width(&self) -> usize {
        match self {
            LinePrefix::EqualsRun(len) => *len + 1,
            LinePrefix::EnCaret => EN_CARET.len(),
            LinePrefix::EnV => EN_V.len(),
            LinePrefix::Hyphen => HYPHEN.len(),
            LinePrefix::HyphenFence(len) => *len,
            LinePrefix::Colon => COLON.len_utf8(),
            LinePrefix::Quote => QUOTE.len_utf8(),
            LinePrefix::NumberDot { width, .. } => *width,
            LinePrefix::None => 0,
        }
    }
}

impl<'a> LineToken<'a> {
    fn raw_line(&self) -> &'a str {
        match self {
            LineToken::Blank { raw_line, .. } => raw_line,
            LineToken::Line { raw_line, .. } => raw_line,
        }
    }

    fn body(&self) -> Option<&'a str> {
        match self {
            LineToken::Blank { .. } => None,
            LineToken::Line {
                raw_line,
                indent,
                kind,
            } => {
                let content = &raw_line[*indent..];
                let body = &content[kind.width()..];

                match kind {
                    LinePrefix::Colon | LinePrefix::Quote | LinePrefix::HyphenFence(_) => {
                        Some(body.strip_prefix(' ').unwrap_or(body))
                    }
                    _ => Some(body),
                }
            }
        }
    }
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
        LinePrefix::HyphenFence(_) => {
            *indent == 0 && line.body().is_some_and(|body| !body.trim().is_empty())
        }
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
        if !raw_lines.is_empty() && cursor.peek().is_some_and(starts_block_after_paragraph) {
            break;
        }
        raw_lines.push(cursor.next()?.raw_line());
    }

    Some(BlockDraft::Paragraph { raw_lines })
}

fn parse_container_header(header: &str) -> Option<(&str, Vec<&str>)> {
    let mut parts = header.split_whitespace();
    let kind = parts.next()?;
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
        let body = token.body()?;

        let Some((key, value)) = body.split_once(':') else {
            diagnostics.push(ParseDiagnostic {
                line,
                kind: ParseDiagnosticKind::InvalidProperty { raw_line },
            });
            continue;
        };

        items.push(PropertyItemDraft::new(key.trim(), value.trim()))
    }

    Some(BlockDraft::Property {
        indent,
        kind: property_kind,
        items,
    })
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
        LineToken::Blank { .. } => LineToken::Blank { raw_line: "" },
        LineToken::Line {
            raw_line,
            indent: line_indent,
            ..
        } => {
            debug_assert!(*line_indent >= indent);
            scan_line(&raw_line[indent..])
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
            cursor.next();
            if !child_lines.is_empty() {
                child_lines.push(LineToken::Blank { raw_line: "" });
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

    cursor.next();

    Some((
        ListItemDraft {
            kind,
            indent,
            body,
            children: vec![],
        },
        content_indent,
    ))
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

fn build_drafts<'a>(
    lines: &[LineToken<'a>],
    diagnostics: &mut Vec<ParseDiagnostic<'a>>,
) -> Vec<BlockDraft<'a>> {
    let mut cursor = LineCursor::new(lines);
    let mut drafts = vec![];

    while !cursor.is_eof() {
        if let Some(draft) = parse_container_draft(&mut cursor, diagnostics) {
            drafts.push(draft);
        } else if let Some(draft) = parse_code_draft(&mut cursor) {
            drafts.push(draft);
        } else if let Some(draft) = parse_quote_draft(&mut cursor) {
            drafts.push(draft);
        } else if let Some(draft) = parse_property_draft(&mut cursor, diagnostics) {
            drafts.push(draft);
        } else if let Some(draft) = parse_heading_draft(&mut cursor) {
            drafts.push(draft);
        } else if let Some(draft) = parse_list_draft(&mut cursor) {
            drafts.push(draft);
        } else if cursor.consume_blank() {
            continue;
        } else if let Some(draft) = parse_paragraph_draft(&mut cursor) {
            drafts.push(draft);
        }
    }

    drafts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_unordered_list() {
        let source = r#"- first
  - second
  - second-sibling
    - third
    - third-sibling
  - fourth but second depth

- another list"#;

        let parsed = parse(source);
        let doc = parsed.document;

        assert_eq!(doc.blocks.len(), 2);

        let BlockKind::List { items } = &doc.blocks[0].kind else {
            panic!("expected first block to be a list");
        };

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].body, vec![Inline::Text("first")]);
        assert_eq!(items[0].children.len(), 1);

        let BlockKind::List { items } = &items[0].children[0].kind else {
            panic!("expected first item to contain a nested list");
        };

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].body, vec![Inline::Text("second")]);
        assert_eq!(items[1].body, vec![Inline::Text("second-sibling")]);
        assert_eq!(items[2].body, vec![Inline::Text("fourth but second depth")]);
        assert!(items[0].children.is_empty());
        assert!(items[2].children.is_empty());
        assert_eq!(items[1].children.len(), 1);

        let BlockKind::List { items } = &items[1].children[0].kind else {
            panic!("expected second-sibling to contain a nested list");
        };

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].body, vec![Inline::Text("third")]);
        assert_eq!(items[1].body, vec![Inline::Text("third-sibling")]);
        assert!(items[0].children.is_empty());
        assert!(items[1].children.is_empty());

        let BlockKind::List { items } = &doc.blocks[1].kind else {
            panic!("expected blank-separated list to become a separate block");
        };

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].body, vec![Inline::Text("another list")]);
        assert!(items[0].children.is_empty());
    }

    #[test]
    fn parse_inline_supports_markdown_style_links() {
        assert_eq!(
            parse_inline("Read [djot](https://github.com/jgm/djot) and [[Maki]]."),
            vec![
                Inline::Text("Read "),
                Inline::Link {
                    title: "djot",
                    target: "https://github.com/jgm/djot"
                },
                Inline::Text(" and "),
                Inline::NoteLink { target: "Maki" },
                Inline::Text(".")
            ]
        );
    }

    #[test]
    fn test_scan_lines() {
        let source = r#"--^ title: Maki
== Heading

- list
  - nested list

: This is Code Line

--- code
Container Block
---

plain text"#;

        assert_eq!(
            scan_lines(source),
            vec![
                LineToken::Line {
                    indent: 0,
                    kind: LinePrefix::EnCaret,
                    raw_line: "--^ title: Maki"
                },
                LineToken::Line {
                    indent: 0,
                    kind: LinePrefix::EqualsRun(2),
                    raw_line: "== Heading"
                },
                LineToken::Blank { raw_line: "" },
                LineToken::Line {
                    indent: 0,
                    kind: LinePrefix::Hyphen,
                    raw_line: "- list"
                },
                LineToken::Line {
                    indent: 2,
                    kind: LinePrefix::Hyphen,
                    raw_line: "  - nested list"
                },
                LineToken::Blank { raw_line: "" },
                LineToken::Line {
                    indent: 0,
                    kind: LinePrefix::Colon,
                    raw_line: ": This is Code Line"
                },
                LineToken::Blank { raw_line: "" },
                LineToken::Line {
                    indent: 0,
                    kind: LinePrefix::HyphenFence(3),
                    raw_line: "--- code"
                },
                LineToken::Line {
                    indent: 0,
                    kind: LinePrefix::None,
                    raw_line: "Container Block"
                },
                LineToken::Line {
                    indent: 0,
                    kind: LinePrefix::HyphenFence(3),
                    raw_line: "---"
                },
                LineToken::Blank { raw_line: "" },
                LineToken::Line {
                    indent: 0,
                    kind: LinePrefix::None,
                    raw_line: "plain text"
                },
            ]
        );
    }

    #[test]
    fn test_build_drafts() {
        let source = r#"--^ title: Maki
--^ description: This is a simple example.
== Heading

- list
  - nested list

: This is Code Line

--- code
Container Block
---

plain text"#;

        let lines = scan_lines(source);
        let mut diagnostics = vec![];

        assert_eq!(
            build_drafts(&lines, &mut diagnostics),
            vec![
                BlockDraft::Property {
                    indent: 0,
                    kind: PropertyKind::Previous,
                    items: vec![
                        PropertyItemDraft::new("title", "Maki"),
                        PropertyItemDraft::new("description", "This is a simple example.")
                    ],
                },
                BlockDraft::Heading {
                    level: 2,
                    body: "Heading",
                },
                BlockDraft::List {
                    items: vec![ListItemDraft {
                        kind: ListKind::Unordered,
                        indent: 0,
                        body: "list",
                        children: vec![BlockDraft::List {
                            items: vec![ListItemDraft {
                                kind: ListKind::Unordered,
                                indent: 0,
                                body: "nested list",
                                children: vec![],
                            }],
                        }],
                    }],
                },
                BlockDraft::Code {
                    raw_lines: vec!["This is Code Line"],
                },
                BlockDraft::Container {
                    kind: "code",
                    args: vec![],
                    raw_lines: vec!["Container Block"],
                },
                BlockDraft::Paragraph {
                    raw_lines: vec!["plain text"],
                },
            ]
        );
        assert_eq!(diagnostics, vec![]);
    }

    #[test]
    fn parse_reports_no_diagnostics_for_supported_document() {
        let parsed = parse(
            r#"--^ title: Maki

= Heading

- list
  - nested

--v lang: html
: <main></main>"#,
        );

        assert!(parsed.diagnostics.is_empty());
        assert_eq!(parsed.document.title(), Some("Maki"));
    }

    #[test]
    fn parse_quote_lines_strip_prefix_for_inner_maki() {
        let parsed = parse(
            r#"> = Quoted
>
> Body with `code`
> - item
> > nested"#,
        );

        assert!(parsed.diagnostics.is_empty());
        assert_eq!(parsed.document.blocks.len(), 1);

        let BlockKind::Quote { lines } = &parsed.document.blocks[0].kind else {
            panic!("expected a quote block");
        };

        assert_eq!(
            lines,
            &vec!["= Quoted", "", "Body with `code`", "- item", "> nested"]
        );
    }

    #[test]
    fn parse_reports_invalid_property_without_panicking() {
        let parsed = parse(
            r#"--^ invalid-property
--^ title: Maki

= Heading"#,
        );

        assert_eq!(parsed.document.title(), Some("Maki"));
        assert_eq!(
            parsed.diagnostics,
            vec![ParseDiagnostic {
                line: 1,
                kind: ParseDiagnosticKind::InvalidProperty {
                    raw_line: "--^ invalid-property"
                },
            }]
        );
    }

    #[test]
    fn parse_reports_unclosed_container() {
        let parsed = parse(
            r#"--- code
fn main() {}"#,
        );

        assert_eq!(
            parsed.diagnostics,
            vec![ParseDiagnostic {
                line: 1,
                kind: ParseDiagnosticKind::UnclosedContainer {
                    raw_line: "--- code"
                },
            }]
        );
    }

    #[test]
    fn parse_preserves_shorter_fence_inside_long_container() {
        let parsed = parse(
            r#"----- code
---
body
-----"#,
        );

        assert!(parsed.diagnostics.is_empty());
        assert_eq!(parsed.document.blocks.len(), 1);

        let BlockKind::Container { kind, args, lines } = &parsed.document.blocks[0].kind else {
            panic!("expected a container block");
        };

        assert_eq!(*kind, "code");
        assert!(args.is_empty());
        assert_eq!(lines, &vec!["---", "body"]);
    }

    #[test]
    fn parse_treats_headerless_hyphen_run_as_paragraph() {
        let parsed = parse(
            r#"---
plain"#,
        );

        assert!(parsed.diagnostics.is_empty());
        assert_eq!(parsed.document.blocks.len(), 1);

        let BlockKind::Paragraph { body } = &parsed.document.blocks[0].kind else {
            panic!("expected a paragraph block");
        };

        assert_eq!(
            body,
            &vec![
                Inline::Text("---"),
                Inline::SoftBreak,
                Inline::Text("plain")
            ]
        );
    }

    #[test]
    fn ordered_list_strips_marker_width_from_body() {
        let parsed = parse(
            r#"9. ninth
10. tenth"#,
        );

        assert!(parsed.diagnostics.is_empty());

        let BlockKind::List { items } = &parsed.document.blocks[0].kind else {
            panic!("expected an ordered list block");
        };

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, ListKind::Ordered);
        assert_eq!(items[0].body, vec![Inline::Text("ninth")]);
        assert_eq!(items[1].kind, ListKind::Ordered);
        assert_eq!(items[1].body, vec![Inline::Text("tenth")]);
    }

    #[test]
    fn ordered_list_keeps_indented_paragraphs_inside_items() {
        let parsed = parse(
            r#"1. Glider 활용 증진

   현재 Glider의 CloudData는 여러 제약사항으로 다양한 곳에 활용하지 못하고 있습니다.

2. Datadog 활용 증진

   Datadog 사용을 위해 고비용을 지불하고 있습니다.

3. 사내 라이브러리 도입

   사내 서버 개발 미팅을 통해 도출된 반복되는 업무를 라이브러리화합니다."#,
        );

        assert!(parsed.diagnostics.is_empty());
        assert_eq!(parsed.document.blocks.len(), 1);

        let BlockKind::List { items } = &parsed.document.blocks[0].kind else {
            panic!("expected an ordered list block");
        };

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].kind, ListKind::Ordered);
        assert_eq!(items[0].body, vec![Inline::Text("Glider 활용 증진")]);
        assert_eq!(items[0].children.len(), 1);

        let BlockKind::Paragraph { body } = &items[0].children[0].kind else {
            panic!("expected the indented line to become a child paragraph");
        };

        assert_eq!(
            body,
            &vec![Inline::Text(
                "현재 Glider의 CloudData는 여러 제약사항으로 다양한 곳에 활용하지 못하고 있습니다."
            )]
        );
        assert_eq!(items[1].body, vec![Inline::Text("Datadog 활용 증진")]);
        assert_eq!(items[1].children.len(), 1);
        assert_eq!(items[2].body, vec![Inline::Text("사내 라이브러리 도입")]);
        assert_eq!(items[2].children.len(), 1);
    }
}
