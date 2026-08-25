use super::draft::PropertyKind;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum LineToken<'a> {
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
pub(super) enum LinePrefix {
    EqualsRun(usize), // #, ##, ###, ...
    EnCaret,          // --^
    EnV,              // --v
    Hyphen,           // -
    HyphenFence(usize),
    Colon,
    Quote,
    Reference,
    NumberDot { width: usize }, // 1.
    None,
}

pub(super) fn scan_line(line: &str) -> LineToken<'_> {
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

    Some(len)
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
    if raw_text.starts_with('[') {
        return LinePrefix::Reference;
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

pub(super) fn scan_lines(source: &str) -> Vec<LineToken<'_>> {
    source.lines().map(scan_line).collect()
}

impl LinePrefix {
    pub(super) fn as_property_kind(&self) -> Option<PropertyKind> {
        match self {
            LinePrefix::EnCaret => Some(PropertyKind::Previous),
            LinePrefix::EnV => Some(PropertyKind::Next),
            _ => None,
        }
    }

    /// Returns the width of the prefix in characters.
    /// It contains the whitespaces after the prefix which serve as the prefix's delimiter.
    pub(super) fn width(&self) -> usize {
        match self {
            LinePrefix::EqualsRun(len) => *len + 1,
            LinePrefix::EnCaret => EN_CARET.len(),
            LinePrefix::EnV => EN_V.len(),
            LinePrefix::Hyphen => HYPHEN.len(),
            LinePrefix::HyphenFence(len) => *len,
            LinePrefix::Colon => COLON.len_utf8(),
            LinePrefix::Quote => QUOTE.len_utf8(),
            LinePrefix::Reference => '['.len_utf8(),
            LinePrefix::NumberDot { width, .. } => *width,
            LinePrefix::None => 0,
        }
    }
}

impl<'a> LineToken<'a> {
    pub(super) fn raw_line(&self) -> &'a str {
        match self {
            LineToken::Blank { raw_line, .. } => raw_line,
            LineToken::Line { raw_line, .. } => raw_line,
        }
    }

    pub(super) fn body(&self) -> Option<&'a str> {
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
