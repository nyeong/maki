//! Maki parser

use std::collections::BTreeMap;

/// source
///   -> LineToken[]
///   -> BlockDraft[]
///
/// Later:
///   -> Block[]
///   -> Document
pub(crate) fn parse(source: &str) -> Document<'_> {
    let lines = scan_lines(source);
    let drafts = build_drafts(&lines);
    build_documents(&drafts)
}

#[derive(Debug, PartialEq)]
pub(crate) enum Inline<'a> {
    NoteLink { target: &'a str },
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
fn parse_inline<'a>(source: &'a str) -> Vec<Inline<'a>> {
    let mut cursor = InlineCursor::new(source);
    let mut inlines = vec![];
    let mut text_start = 0;

    while !cursor.is_eol() {
        let start = cursor.pos();

        if let Some(inline) =
            parse_inline_code(&mut cursor).or_else(|| parse_inline_note_link(&mut cursor))
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
            header: _,
            raw_lines,
        } => Block {
            kind: BlockKind::Code {
                lines: raw_lines.clone(),
                lang: None,
            },
            props,
        },
        BlockDraft::List { items } => Block {
            kind: BlockKind::List {
                items: items
                    .iter()
                    .map(|draft| ListItem {
                        kind: draft.kind,
                        indent: draft.indent,
                        body: parse_inline(draft.body),
                    })
                    .collect(),
            },
            props,
        },
        BlockDraft::Tbd { items } => Block {
            kind: BlockKind::Code {
                lines: items.clone(),
                lang: Some("maki"),
            },
            props,
        },
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

#[derive(Debug)]
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
}

#[derive(Debug, PartialEq)]
pub(crate) struct ListItem<'a> {
    pub(crate) body: Vec<Inline<'a>>,
    indent: usize,
    kind: ListKind,
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
    Backticks,        // ```
    NumberDot(usize), // 1.
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
const BACKTICKS: &str = "```";
const EQUALS: char = '=';
const CODE_BLOCK_INDENT: usize = 4;

fn parse_number_dot_prefix(source: &str) -> Option<usize> {
    let (digits, rest) = source.split_once('.')?;

    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) || !rest.starts_with(' ') {
        return None;
    }

    digits.parse().ok()
}

/// Accepts a text trimmed of leading whitespace.
fn scan_line_prefix(raw_text: &str) -> LinePrefix {
    if raw_text.starts_with(EN_CARET) {
        return LinePrefix::EnCaret;
    }
    if raw_text.starts_with(EN_V) {
        return LinePrefix::EnV;
    }
    if let Some(n) = parse_number_dot_prefix(raw_text) {
        return LinePrefix::NumberDot(n);
    }
    if raw_text.starts_with(HYPHEN) {
        return LinePrefix::Hyphen;
    }
    if raw_text.starts_with(BACKTICKS) {
        return LinePrefix::Backticks;
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
    // Ordered
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

    /// 4-length-indented
    Code {
        raw_lines: Vec<&'a str>,
    },

    /// ```
    Container {
        header: &'a str,
        raw_lines: Vec<&'a str>,
    },

    List {
        items: Vec<ListItemDraft<'a>>,
    },

    Tbd {
        items: Vec<&'a str>,
    },
}

#[derive(Debug, PartialEq)]
struct ListItemDraft<'a> {
    kind: ListKind,
    indent: usize,
    body: &'a str,
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
            LinePrefix::Backticks => BACKTICKS.len(),
            LinePrefix::NumberDot(num) => num / 10 + 1,
            LinePrefix::None => 0,
        }
    }
}

impl<'a> LineToken<'a> {
    fn indent(&self) -> usize {
        match self {
            LineToken::Blank { .. } => 0,
            LineToken::Line { indent, .. } => *indent,
        }
    }

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
                Some(&content[kind.width()..])
            }
        }
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
}

fn parse_paragraph_draft<'a>(cursor: &mut LineCursor<'_, 'a>) -> Option<BlockDraft<'a>> {
    let mut raw_lines = vec![];

    while !cursor.is_eof() {
        if cursor.consume_blank() {
            break;
        }
        raw_lines.push(cursor.next()?.raw_line());
    }

    Some(BlockDraft::Paragraph { raw_lines })
}

fn parse_container_draft<'a>(cursor: &mut LineCursor<'_, 'a>) -> Option<BlockDraft<'a>> {
    if !matches!(
        cursor.peek(),
        Some(LineToken::Line {
            kind: LinePrefix::Backticks,
            indent: 0,
            ..
        }),
    ) {
        return None;
    };

    let mut raw_lines = vec![];
    let header = cursor.next()?.body()?;

    while let Some(line) = cursor.next() {
        if matches!(
            line,
            LineToken::Line {
                kind: LinePrefix::Backticks,
                ..
            }
        ) {
            break;
        }
        raw_lines.push(line.raw_line());
    }

    Some(BlockDraft::Container { header, raw_lines })
}

// TODO: parse.. 함수들 모두 Result<Option<T>, E> 타입으로 바꾸기. Ok(None), Ok(Some(...)), Err(..)
fn parse_property_draft<'a>(cursor: &mut LineCursor<'_, 'a>) -> Option<BlockDraft<'a>> {
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
        let raw_line = cursor.next()?.body()?;
        let (key, value) = raw_line
            .split_once(':')
            .unwrap_or_else(|| panic!("invalid property: {:?}", raw_line));
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

fn parse_list_item_draft<'a>(cursor: &mut LineCursor<'_, 'a>) -> Option<ListItemDraft<'a>> {
    let line = cursor.peek()?;
    let LineToken::Line {
        indent,
        kind: LinePrefix::Hyphen,
        ..
    } = line
    else {
        return None;
    };
    let indent = *indent;
    let body = line.body()?;

    cursor.next();

    Some(ListItemDraft {
        kind: ListKind::Unordered,
        indent,
        body,
    })
}

fn parse_list_draft<'a>(cursor: &mut LineCursor<'_, 'a>) -> Option<BlockDraft<'a>> {
    let LineToken::Line {
        indent: 0,
        kind: LinePrefix::Hyphen,
        ..
    } = cursor.peek()?
    else {
        return None;
    };

    let mut items = vec![];

    while let Some(line) = parse_list_item_draft(cursor) {
        items.push(line);
    }

    Some(BlockDraft::List { items })
}

fn parse_tbd_draft<'a>(cursor: &mut LineCursor<'_, 'a>) -> Option<BlockDraft<'a>> {
    if !matches!(
        cursor.peek(),
        Some(LineToken::Line {
            indent: 0,
            kind: LinePrefix::NumberDot(_),
            ..
        })
    ) {
        return None;
    }

    let mut items = vec![];

    while let Some(LineToken::Line {
        indent: 0,
        kind: LinePrefix::NumberDot(_),
        ..
    }) = cursor.peek()
    {
        items.push(cursor.next()?.raw_line());
    }

    Some(BlockDraft::Tbd { items })
}

fn parse_code_draft<'a>(cursor: &mut LineCursor<'_, 'a>) -> Option<BlockDraft<'a>> {
    let line = cursor.peek()?;
    if line.indent() < CODE_BLOCK_INDENT {
        return None;
    }

    let mut raw_lines = vec![];

    while let Some(line) = cursor.peek() {
        if line.indent() < CODE_BLOCK_INDENT {
            break;
        } else if matches!(line, LineToken::Blank { .. }) {
            raw_lines.push(line.raw_line());
        } else {
            raw_lines.push(&line.raw_line()[CODE_BLOCK_INDENT..]);
        }
        cursor.next();
    }

    if raw_lines.last().is_some_and(|l| l.is_empty()) {
        raw_lines.pop();
    }

    Some(BlockDraft::Code { raw_lines })
}

fn build_drafts<'a>(lines: &[LineToken<'a>]) -> Vec<BlockDraft<'a>> {
    let mut cursor = LineCursor::new(lines);
    let mut drafts = vec![];

    while !cursor.is_eof() {
        if let Some(draft) = parse_container_draft(&mut cursor) {
            drafts.push(draft);
        } else if let Some(draft) = parse_code_draft(&mut cursor) {
            drafts.push(draft);
        } else if let Some(draft) = parse_property_draft(&mut cursor) {
            drafts.push(draft);
        } else if let Some(draft) = parse_heading_draft(&mut cursor) {
            drafts.push(draft);
        } else if let Some(draft) = parse_list_draft(&mut cursor) {
            drafts.push(draft);
        } else if let Some(draft) = parse_tbd_draft(&mut cursor) {
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
    fn test_scan_lines() {
        let source = r#"--^ title: Maki
== Heading

- list
  - nested list

    This is Code Line

```code
Container Block
```

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
                    indent: 4,
                    kind: LinePrefix::None,
                    raw_line: "    This is Code Line"
                },
                LineToken::Blank { raw_line: "" },
                LineToken::Line {
                    indent: 0,
                    kind: LinePrefix::Backticks,
                    raw_line: "```code"
                },
                LineToken::Line {
                    indent: 0,
                    kind: LinePrefix::None,
                    raw_line: "Container Block"
                },
                LineToken::Line {
                    indent: 0,
                    kind: LinePrefix::Backticks,
                    raw_line: "```"
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

    This is Code Line

```code
Container Block
```

plain text"#;

        assert_eq!(
            build_drafts(&scan_lines(source)),
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
                    items: vec![
                        ListItemDraft {
                            kind: ListKind::Unordered,
                            indent: 0,
                            body: "list",
                        },
                        ListItemDraft {
                            kind: ListKind::Unordered,
                            indent: 2,
                            body: "nested list",
                        },
                    ],
                },
                BlockDraft::Code {
                    raw_lines: vec!["This is Code Line"],
                },
                BlockDraft::Container {
                    header: "code",
                    raw_lines: vec!["Container Block"],
                },
                BlockDraft::Paragraph {
                    raw_lines: vec!["plain text"],
                },
            ]
        );
    }
}
