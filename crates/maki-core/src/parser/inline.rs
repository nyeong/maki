use std::collections::{BTreeMap, BTreeSet};

use super::types::{
    DateRange, DateStamp, DateStampKind, DateStampTarget, Inline, ReferenceDefinition,
    ReferenceDefinitions,
};

#[derive(Default)]
pub(super) struct ReferenceLookup<'a> {
    links: BTreeMap<&'a str, &'a str>,
    footnotes: BTreeSet<&'a str>,
}

impl<'a> ReferenceLookup<'a> {
    pub(super) fn insert_link(&mut self, title: &'a str, target: &'a str) -> bool {
        if self.links.contains_key(title) {
            return false;
        }
        self.links.insert(title, target);
        true
    }

    pub(super) fn insert_footnote(&mut self, label: &'a str) -> bool {
        self.footnotes.insert(label)
    }

    pub(super) fn extend<'parent>(&mut self, references: &ReferenceDefinitions<'parent>)
    where
        'parent: 'a,
    {
        for definition in references.all() {
            match definition {
                ReferenceDefinition::Link { title, target } => {
                    self.insert_link(title, target);
                }
                ReferenceDefinition::Footnote { label, .. } => {
                    self.insert_footnote(label);
                }
            }
        }
    }

    fn link_target(&self, title: &str) -> Option<&'a str> {
        self.links.get(title).copied()
    }

    fn has_footnote(&self, label: &str) -> bool {
        self.footnotes.contains(label)
    }
}

#[derive(Clone, Copy)]
enum ClosingDelimiter {
    Backtick,
    NoteLink,
    Bracket,
    Angle,
    Brace,
    Strong,
    Italic,
    Highlight,
}

const CLOSING_DELIMITER_COUNT: usize = ClosingDelimiter::Highlight as usize + 1;

/// Closing positions shared by all speculative inline parsers at a cursor.
///
/// Without this index, every unmatched opener searches the remaining suffix,
/// making delimiter-heavy malformed input quadratic in the source length.
struct DelimiterIndex {
    positions: [Vec<usize>; CLOSING_DELIMITER_COUNT],
}

impl DelimiterIndex {
    fn new(source: &str) -> Self {
        let mut positions: [Vec<usize>; CLOSING_DELIMITER_COUNT] =
            std::array::from_fn(|_| Vec::new());
        let bytes = source.as_bytes();

        for (index, ch) in source.char_indices() {
            let delimiter = match ch {
                '`' => Some(ClosingDelimiter::Backtick),
                ']' => Some(ClosingDelimiter::Bracket),
                '>' => Some(ClosingDelimiter::Angle),
                '}' => Some(ClosingDelimiter::Brace),
                '*' if is_symmetric_close_at(source, index, '*') => Some(ClosingDelimiter::Strong),
                '/' if is_italic_close_at(source, index) => Some(ClosingDelimiter::Italic),
                '=' if is_symmetric_close_at(source, index, '=') => {
                    Some(ClosingDelimiter::Highlight)
                }
                _ => None,
            };

            if let Some(delimiter) = delimiter {
                positions[delimiter as usize].push(index);
            }
            if ch == ']' && bytes.get(index + 1) == Some(&b']') {
                positions[ClosingDelimiter::NoteLink as usize].push(index);
            }
        }

        Self { positions }
    }

    fn find_at_or_after(&self, delimiter: ClosingDelimiter, start: usize) -> Option<usize> {
        let positions = &self.positions[delimiter as usize];
        let index = positions.partition_point(|position| *position < start);
        positions.get(index).copied()
    }
}

fn is_symmetric_close_at(source: &str, index: usize, delimiter: char) -> bool {
    let before = source[..index].chars().next_back();
    let after = source[index + delimiter.len_utf8()..].chars().next();

    before.is_some_and(|ch| !ch.is_whitespace() && ch != delimiter) && after != Some(delimiter)
}

fn is_italic_close_at(source: &str, index: usize) -> bool {
    let before = source[..index].chars().next_back();
    let after = source[index + '/'.len_utf8()..].chars().next();

    before.is_some_and(|ch| !ch.is_whitespace() && ch != '/') && is_italic_close_boundary(after)
}

struct InlineCursor<'a> {
    source: &'a str,
    pos: usize,
    delimiters: DelimiterIndex,
}

impl<'a> InlineCursor<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            pos: 0,
            delimiters: DelimiterIndex::new(source),
        }
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

    fn previous_char(&self) -> Option<char> {
        self.source[..self.pos].chars().next_back()
    }

    fn find_closing(&self, delimiter: ClosingDelimiter, offset: usize) -> Option<usize> {
        let start = self.pos.checked_add(offset)?;
        self.delimiters
            .find_at_or_after(delimiter, start)
            .map(|position| position - self.pos)
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
const INLINE_CODE_DELIMITER: char = '`';
const INLINE_DATE_RANGE_SEPARATOR: &str = "--";

fn parse_inline_code<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    let rest = cursor.rest();
    let body = rest.strip_prefix(INLINE_CODE_DELIMITER)?;
    let end = cursor.find_closing(ClosingDelimiter::Backtick, INLINE_CODE_DELIMITER.len_utf8())?
        - INLINE_CODE_DELIMITER.len_utf8();
    let contents = &body[..end];

    cursor
        .bump(INLINE_CODE_DELIMITER.len_utf8() + contents.len() + INLINE_CODE_DELIMITER.len_utf8());

    Some(Inline::Code(contents))
}

fn parse_inline_note_link<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    let rest = cursor.rest();
    let body = rest.strip_prefix(INLINE_NOTE_LINK_BEGIN)?;
    let end = cursor.find_closing(ClosingDelimiter::NoteLink, INLINE_NOTE_LINK_BEGIN.len())?
        - INLINE_NOTE_LINK_BEGIN.len();
    let target = &body[..end];

    cursor.bump(INLINE_NOTE_LINK_BEGIN.len() + target.len() + INLINE_NOTE_LINK_END.len());

    Some(Inline::NoteLink { target })
}

fn parse_date_stamp_at<'a>(
    cursor: &InlineCursor<'a>,
    offset: usize,
) -> Option<(DateStamp<'a>, usize)> {
    let source = &cursor.rest()[offset..];
    let (kind, open, close, delimiter) = match source.chars().next()? {
        '[' => (DateStampKind::Date, '[', ']', ClosingDelimiter::Bracket),
        '<' => (DateStampKind::Event, '<', '>', ClosingDelimiter::Angle),
        _ => return None,
    };
    let body_start = open.len_utf8();
    let body_source = &source[body_start..];
    let body_end = cursor.find_closing(delimiter, offset + body_start)? - offset - body_start;
    let body = &body_source[..body_end];
    let (target, target_len) = DateStampTarget::parse_prefix(body)?;
    let rest = &body[target_len..];

    if !rest.is_empty() && !rest.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }

    Some((
        DateStamp { kind, target, body },
        body_start + body.len() + close.len_utf8(),
    ))
}

fn parse_inline_date_range<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    let rest = cursor.rest();
    let (start, start_len) = parse_date_stamp_at(cursor, 0)?;
    let rest_after_start = &rest[start_len..];
    rest_after_start.strip_prefix(INLINE_DATE_RANGE_SEPARATOR)?;
    let end_offset = start_len + INLINE_DATE_RANGE_SEPARATOR.len();
    let (end, end_len) = parse_date_stamp_at(cursor, end_offset)?;

    let (Some(start_date), Some(end_date)) = (start.date(), end.date()) else {
        return None;
    };

    if start.kind() != end.kind() || start_date > end_date {
        return None;
    }

    cursor.bump(start_len + INLINE_DATE_RANGE_SEPARATOR.len() + end_len);

    Some(Inline::DateRange(DateRange { start, end }))
}

fn parse_inline_date_stamp<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    let (stamp, len) = parse_date_stamp_at(cursor, 0)?;
    cursor.bump(len);
    Some(Inline::DateStamp(stamp))
}

fn parse_inline_footnote<'a>(
    cursor: &mut InlineCursor<'a>,
    references: &ReferenceLookup<'a>,
) -> Option<Inline<'a>> {
    let body = cursor.rest().strip_prefix("[^")?;
    let end = cursor.find_closing(ClosingDelimiter::Bracket, "[^".len())? - "[^".len();
    let label = &body[..end];

    if label.is_empty()
        || label.contains(['[', ']'])
        || label.chars().any(char::is_whitespace)
        || !references.has_footnote(label)
    {
        return None;
    }

    cursor.bump("[^".len() + label.len() + ']'.len_utf8());
    Some(Inline::Footnote { label })
}

fn parse_inline_link<'a>(
    cursor: &mut InlineCursor<'a>,
    references: &ReferenceLookup<'a>,
) -> Option<Inline<'a>> {
    let body = cursor.rest().strip_prefix('[')?;
    let end = cursor.find_closing(ClosingDelimiter::Bracket, '['.len_utf8())? - '['.len_utf8();
    let title = &body[..end];

    if title.is_empty() || title.starts_with('^') || title.contains(['[', ']']) {
        return None;
    }

    let target = references.link_target(title)?;
    cursor.bump('['.len_utf8() + title.len() + ']'.len_utf8());

    Some(Inline::Link { title, target })
}

fn parse_inline_hyper_link<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    let body = cursor.rest().strip_prefix('<')?;
    let end = cursor.find_closing(ClosingDelimiter::Angle, '<'.len_utf8())? - '<'.len_utf8();
    let target = &body[..end];
    let target_body = target
        .strip_prefix("https://")
        .or_else(|| target.strip_prefix("http://"))?;

    if target_body.is_empty() || target.contains('<') || target.chars().any(char::is_whitespace) {
        return None;
    }

    cursor.bump('<'.len_utf8() + target.len() + '>'.len_utf8());
    Some(Inline::HyperLink { target })
}

fn parse_symmetric_inline<'a>(
    cursor: &mut InlineCursor<'a>,
    delimiter: char,
    references: &ReferenceLookup<'a>,
    wrap: fn(Vec<Inline<'a>>) -> Inline<'a>,
) -> Option<Inline<'a>> {
    if cursor.previous_char() == Some(delimiter) {
        return None;
    }

    let rest = cursor.rest();
    let body = rest.strip_prefix(delimiter)?;
    let first = body.chars().next()?;
    if first.is_whitespace() || first == delimiter {
        return None;
    }

    let closing = match delimiter {
        '*' => ClosingDelimiter::Strong,
        '=' => ClosingDelimiter::Highlight,
        _ => return None,
    };
    let end = cursor.find_closing(closing, delimiter.len_utf8())? - delimiter.len_utf8();
    let contents = &body[..end];

    cursor.bump(delimiter.len_utf8() + contents.len() + delimiter.len_utf8());
    Some(wrap(parse_inline_with_references(contents, references)))
}

fn parse_inline_italic<'a>(
    cursor: &mut InlineCursor<'a>,
    references: &ReferenceLookup<'a>,
) -> Option<Inline<'a>> {
    if !is_italic_open_boundary(cursor.previous_char()) {
        return None;
    }

    let body = cursor.rest().strip_prefix('/')?;
    let first = body.chars().next()?;
    if first.is_whitespace() || first == '/' {
        return None;
    }

    let end = cursor.find_closing(ClosingDelimiter::Italic, '/'.len_utf8())? - '/'.len_utf8();
    let contents = &body[..end];

    cursor.bump('/'.len_utf8() + contents.len() + '/'.len_utf8());
    Some(Inline::Italic(parse_inline_with_references(
        contents, references,
    )))
}

fn is_italic_open_boundary(previous: Option<char>) -> bool {
    previous.is_none_or(|ch| ch.is_whitespace() || matches!(ch, '(' | '[' | '{' | '"' | '\''))
}

fn is_italic_close_boundary(after: Option<char>) -> bool {
    after.is_none_or(|ch| {
        ch.is_whitespace()
            || matches!(
                ch,
                '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '"' | '\''
            )
    })
}

fn parse_braced_inline<'a>(
    cursor: &mut InlineCursor<'a>,
    prefix: &str,
    wrap: fn(&'a str) -> Inline<'a>,
) -> Option<Inline<'a>> {
    let body = cursor.rest().strip_prefix(prefix)?;
    let end = cursor.find_closing(ClosingDelimiter::Brace, prefix.len())? - prefix.len();
    let contents = &body[..end];

    if contents.is_empty() || contents.contains(['{', '}']) {
        return None;
    }

    cursor.bump(prefix.len() + contents.len() + '}'.len_utf8());
    Some(wrap(contents))
}

pub(super) fn parse_inlines_with_references<'a>(
    source: &[&'a str],
    references: &ReferenceLookup<'a>,
) -> Vec<Inline<'a>> {
    let mut inlines = vec![];

    for (index, line) in source.iter().enumerate() {
        if index > 0 {
            inlines.push(Inline::SoftBreak);
        }
        inlines.extend(parse_inline_with_references(line, references));
    }

    inlines
}

pub(super) fn parse_inline_with_references<'a>(
    source: &'a str,
    references: &ReferenceLookup<'a>,
) -> Vec<Inline<'a>> {
    let mut cursor = InlineCursor::new(source);
    let mut inlines = vec![];
    let mut text_start = 0;

    while !cursor.is_eol() {
        let start = cursor.pos();

        if let Some(inline) = parse_inline_code(&mut cursor)
            .or_else(|| parse_inline_note_link(&mut cursor))
            .or_else(|| parse_inline_date_range(&mut cursor))
            .or_else(|| parse_inline_date_stamp(&mut cursor))
            .or_else(|| parse_inline_footnote(&mut cursor, references))
            .or_else(|| parse_inline_link(&mut cursor, references))
            .or_else(|| parse_inline_hyper_link(&mut cursor))
            .or_else(|| parse_inline_italic(&mut cursor, references))
            .or_else(|| parse_symmetric_inline(&mut cursor, '*', references, Inline::Strong))
            .or_else(|| parse_braced_inline(&mut cursor, "^{", Inline::Superscript))
            .or_else(|| parse_braced_inline(&mut cursor, "_{", Inline::Subscript))
            .or_else(|| parse_symmetric_inline(&mut cursor, '=', references, Inline::Highlight))
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

/// Parses inline syntax that does not require a Document reference definition.
pub fn parse_inline(source: &str) -> Vec<Inline<'_>> {
    parse_inline_with_references(source, &ReferenceLookup::default())
}
