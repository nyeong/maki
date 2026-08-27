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
}

const CLOSING_DELIMITER_COUNT: usize = ClosingDelimiter::Brace as usize + 1;

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum FormattingKind {
    Italic,
    Strong,
    Highlight,
}

impl FormattingKind {
    const COUNT: usize = Self::Highlight as usize + 1;

    fn index(self) -> usize {
        self as usize
    }
}

struct FormattingDelimiter<'a> {
    kind: FormattingKind,
    marker: &'a str,
    can_open: bool,
    can_close: bool,
}

struct FormattingFrame<'a> {
    kind: FormattingKind,
    marker: &'a str,
    opener_index: usize,
}

fn formatting_delimiter<'a>(cursor: &InlineCursor<'a>) -> Option<FormattingDelimiter<'a>> {
    let previous = cursor.previous_char();
    let rest = cursor.rest();

    if let Some(after) = rest.strip_prefix("::") {
        let next = after.chars().next();
        return Some(FormattingDelimiter {
            kind: FormattingKind::Highlight,
            marker: &rest[.."::".len()],
            can_open: previous != Some(':')
                && next.is_some_and(|ch| !ch.is_whitespace() && ch != ':'),
            can_close: previous.is_some_and(|ch| !ch.is_whitespace() && ch != ':')
                && next != Some(':'),
        });
    }

    let marker = &rest[..rest.chars().next()?.len_utf8()];
    let next = rest[marker.len()..].chars().next();
    match marker {
        "*" => Some(FormattingDelimiter {
            kind: FormattingKind::Strong,
            marker,
            can_open: previous != Some('*')
                && next.is_some_and(|ch| !ch.is_whitespace() && ch != '*'),
            can_close: previous.is_some_and(|ch| !ch.is_whitespace() && ch != '*')
                && next != Some('*'),
        }),
        "/" => Some(FormattingDelimiter {
            kind: FormattingKind::Italic,
            marker,
            can_open: is_italic_open_boundary(previous)
                && next.is_some_and(|ch| !ch.is_whitespace() && ch != '/'),
            can_close: previous.is_some_and(|ch| !ch.is_whitespace() && ch != '/')
                && is_italic_close_boundary(next),
        }),
        _ => None,
    }
}

fn close_formatting<'a>(
    inlines: &mut Vec<Inline<'a>>,
    frames: &mut Vec<FormattingFrame<'a>>,
    opener_counts: &mut [usize; FormattingKind::COUNT],
    kind: FormattingKind,
) -> bool {
    let Some(matching) = frames.iter().rposition(|frame| frame.kind == kind) else {
        return false;
    };

    let opener_index = frames[matching].opener_index;
    let marker = frames[matching].marker;
    while frames.len() > matching {
        let frame = frames.pop().expect("formatting frame must exist");
        opener_counts[frame.kind.index()] -= 1;
    }

    let body = inlines.split_off(opener_index + 1);
    debug_assert_eq!(inlines.pop(), Some(Inline::Text(marker)));
    let inline = match kind {
        FormattingKind::Italic => Inline::Italic(body),
        FormattingKind::Strong => Inline::Strong(body),
        FormattingKind::Highlight => Inline::Highlight(body),
    };
    inlines.push(inline);
    true
}

fn merge_contiguous_text<'a>(source: &'a str, left: &'a str, right: &'a str) -> Option<&'a str> {
    let source_start = source.as_ptr() as usize;
    let source_end = source_start.checked_add(source.len())?;
    let left_start = left.as_ptr() as usize;
    let left_end = left_start.checked_add(left.len())?;
    let right_start = right.as_ptr() as usize;
    let right_end = right_start.checked_add(right.len())?;

    if left_start < source_start || left_end != right_start || right_end > source_end {
        return None;
    }

    Some(&source[left_start - source_start..right_end - source_start])
}

fn normalize_inlines<'a>(source: &'a str, inlines: Vec<Inline<'a>>) -> Vec<Inline<'a>> {
    let mut normalized = Vec::with_capacity(inlines.len());

    for inline in inlines {
        let inline = match inline {
            Inline::Italic(body) => Inline::Italic(normalize_inlines(source, body)),
            Inline::Strong(body) => Inline::Strong(normalize_inlines(source, body)),
            Inline::Highlight(body) => Inline::Highlight(normalize_inlines(source, body)),
            inline => inline,
        };

        if let Inline::Text(right) = inline
            && let Some(Inline::Text(left)) = normalized.last_mut()
            && let Some(merged) = merge_contiguous_text(source, left, right)
        {
            *left = merged;
            continue;
        }

        normalized.push(inline);
    }

    normalized
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
    let mut formatting_frames: Vec<FormattingFrame<'a>> = vec![];
    let mut formatting_opener_counts = [0; FormattingKind::COUNT];
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
            .or_else(|| parse_braced_inline(&mut cursor, "^{", Inline::Superscript))
            .or_else(|| parse_braced_inline(&mut cursor, "_{", Inline::Subscript))
        {
            if text_start < start {
                inlines.push(Inline::Text(&source[text_start..start]));
            }

            inlines.push(inline);
            text_start = cursor.pos();
        } else if let Some(escaped) = cursor
            .rest()
            .strip_prefix('\\')
            .and_then(|rest| rest.chars().next())
            .filter(|ch| ch.is_ascii_punctuation())
        {
            if text_start < start {
                inlines.push(Inline::Text(&source[text_start..start]));
            }

            let escaped_start = start + '\\'.len_utf8();
            let escaped_end = escaped_start + escaped.len_utf8();
            inlines.push(Inline::Text(&source[escaped_start..escaped_end]));
            cursor.bump('\\'.len_utf8() + escaped.len_utf8());
            text_start = cursor.pos();
        } else if let Some(delimiter) = formatting_delimiter(&cursor) {
            let matching_opener =
                delimiter.can_close && formatting_opener_counts[delimiter.kind.index()] > 0;
            if !delimiter.can_open && !matching_opener {
                cursor.bump_char();
                continue;
            }

            if text_start < start {
                inlines.push(Inline::Text(&source[text_start..start]));
            }

            let closed = matching_opener
                && close_formatting(
                    &mut inlines,
                    &mut formatting_frames,
                    &mut formatting_opener_counts,
                    delimiter.kind,
                );
            if !closed && delimiter.can_open {
                let opener_index = inlines.len();
                inlines.push(Inline::Text(delimiter.marker));
                formatting_opener_counts[delimiter.kind.index()] += 1;
                formatting_frames.push(FormattingFrame {
                    kind: delimiter.kind,
                    marker: delimiter.marker,
                    opener_index,
                });
            }
            cursor.bump(delimiter.marker.len());
            text_start = cursor.pos();
        } else {
            cursor.bump_char();
        }
    }

    if text_start < source.len() {
        inlines.push(Inline::Text(&source[text_start..]));
    }

    normalize_inlines(source, inlines)
}

/// Parses inline syntax that does not require a Document reference definition.
pub fn parse_inline(source: &str) -> Vec<Inline<'_>> {
    parse_inline_with_references(source, &ReferenceLookup::default())
}
