//! Maki parser

/// source
///   -> LineToken[]
///   -> BlockDraft[]
///
/// Later:
///   -> Block[]
///   -> Document
pub(crate) fn parse(source: &str) -> String {
    let lines = scan_lines(source);
    let drafts = build_drafts(&lines);

    format!("{drafts:#?}")
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
#[derive(Debug, Clone, PartialEq)]
enum LinePrefix {
    EqualsRun(usize), // #, ##, ###, ...
    EnCaret,          // --^
    EnV,              // --v
    Hyphen,           // -
    Backticks,        // ```
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

/// Accepts a text trimmed of leading whitespace.
fn scan_line_prefix(raw_text: &str) -> LinePrefix {
    if raw_text.starts_with(EN_CARET) {
        return LinePrefix::EnCaret;
    }
    if raw_text.starts_with(EN_V) {
        return LinePrefix::EnV;
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

#[derive(Debug, PartialEq)]
enum ListKind {
    Unordered,
    // Ordered
}

#[derive(Debug, PartialEq)]
enum PropertyKind {
    Previous,
    Next,
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
        body: Vec<&'a str>,
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

struct LineCursor<'a> {
    lines: &'a [LineToken<'a>],
    pos: usize,
}

impl<'a> LineCursor<'a> {
    fn new(lines: &'a [LineToken<'a>]) -> Self {
        Self { lines, pos: 0 }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.lines.len()
    }

    fn peek(&self) -> Option<&'a LineToken<'a>> {
        self.lines.get(self.pos)
    }

    fn next(&mut self) -> Option<&'a LineToken<'a>> {
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

fn parse_paragraph_draft<'a>(cursor: &mut LineCursor<'a>) -> Option<BlockDraft<'a>> {
    let mut raw_lines = vec![];

    while !cursor.is_eof() {
        if cursor.consume_blank() {
            break;
        }
        raw_lines.push(cursor.next()?.raw_line());
    }

    Some(BlockDraft::Paragraph { raw_lines })
}

fn parse_container_draft<'a>(cursor: &mut LineCursor<'a>) -> Option<BlockDraft<'a>> {
    if !matches!(
        cursor.peek(),
        Some(LineToken::Line {
            kind: LinePrefix::Backticks,
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

fn parse_property_draft<'a>(cursor: &mut LineCursor<'a>) -> Option<BlockDraft<'a>> {
    let LineToken::Line { kind, indent, .. } = cursor.peek()? else {
        return None;
    };
    let property_kind = kind.as_property_kind()?;
    let indent = *indent;
    let mut raw_lines = vec![];

    while let Some(LineToken::Line {
        kind: line_kind,
        indent: line_indent,
        ..
    }) = cursor.peek()
    {
        if *line_indent != indent || kind != line_kind {
            break;
        }
        raw_lines.push(cursor.next()?.body()?);
    }

    Some(BlockDraft::Property {
        indent,
        kind: property_kind,
        body: raw_lines,
    })
}

fn parse_heading_draft<'a>(cursor: &mut LineCursor<'a>) -> Option<BlockDraft<'a>> {
    let line @ LineToken::Line {
        kind: LinePrefix::EqualsRun(level),
        ..
    } = cursor.peek()?
    else {
        return None;
    };
    let level = *level;

    if !(1..=6).contains(&level) {
        return None;
    }

    cursor.next();

    Some(BlockDraft::Heading {
        level,
        body: line.body()?,
    })
}

fn parse_list_item_draft<'a>(cursor: &mut LineCursor<'a>) -> Option<ListItemDraft<'a>> {
    let line @ LineToken::Line {
        indent,
        kind: LinePrefix::Hyphen,
        ..
    } = cursor.peek()?
    else {
        return None;
    };

    cursor.next();

    Some(ListItemDraft {
        kind: ListKind::Unordered,
        indent: *indent,
        body: line.body()?,
    })
}

fn parse_list_draft<'a>(cursor: &mut LineCursor<'a>) -> Option<BlockDraft<'a>> {
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

fn parse_code_draft<'a>(cursor: &mut LineCursor<'a>) -> Option<BlockDraft<'a>> {
    let line = cursor.peek()?;
    if line.indent() < CODE_BLOCK_INDENT {
        return None;
    }

    let mut raw_lines = vec![];

    while let Some(line) = cursor.peek() {
        if line.indent() < CODE_BLOCK_INDENT {
            break;
        }
        raw_lines.push(&line.raw_line()[CODE_BLOCK_INDENT..]);
        cursor.next();
    }

    Some(BlockDraft::Code { raw_lines })
}

fn build_drafts<'a>(lines: &'a [LineToken<'a>]) -> Vec<BlockDraft<'a>> {
    let mut cursor = LineCursor::new(lines);
    let mut drafts = vec![];

    while !cursor.is_eof() {
        if let Some(draft) = parse_container_draft(&mut cursor) {
            drafts.push(draft);
        } else if let Some(draft) = parse_property_draft(&mut cursor) {
            drafts.push(draft);
        } else if let Some(draft) = parse_heading_draft(&mut cursor) {
            drafts.push(draft);
        } else if let Some(draft) = parse_list_draft(&mut cursor) {
            drafts.push(draft);
        } else if let Some(draft) = parse_code_draft(&mut cursor) {
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

```src
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
                    raw_line: "```src"
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
    fn test_block_build() {
        let source = r#"--^ title: Maki
--^ description: This is a simple example.
== Heading

- list
  - nested list

    This is Code Line

```src
Container Block
```

plain text"#;

        assert_eq!(
            build_drafts(&scan_lines(source)),
            vec![
                BlockDraft::Property {
                    indent: 0,
                    kind: PropertyKind::Previous,
                    body: vec!["title: Maki", "description: This is a simple example.",],
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
                    header: "src",
                    raw_lines: vec!["Container Block"],
                },
                BlockDraft::Paragraph {
                    raw_lines: vec!["plain text"],
                },
            ]
        );
    }
}
