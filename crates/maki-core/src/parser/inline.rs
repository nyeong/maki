use super::types::{DateRange, DateStamp, DateStampKind, DateStampTarget, Inline};

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
    parenthesis_openings: Vec<usize>,
    parenthesis_ends: Vec<Option<usize>>,
}

impl DelimiterIndex {
    fn new(source: &str) -> Self {
        let mut positions: [Vec<usize>; CLOSING_DELIMITER_COUNT] =
            std::array::from_fn(|_| Vec::new());
        let mut parenthesis_openings = Vec::new();
        let mut parenthesis_ends = Vec::new();
        let mut parenthesis_stack = Vec::new();
        let bytes = source.as_bytes();
        let mut escaped = false;

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

            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '(' => {
                    let slot = parenthesis_openings.len();
                    parenthesis_openings.push(index);
                    parenthesis_ends.push(None);
                    parenthesis_stack.push(slot);
                }
                ')' => {
                    if let Some(slot) = parenthesis_stack.pop() {
                        parenthesis_ends[slot] = Some(index);
                    }
                }
                '\n' | '\r' => parenthesis_stack.clear(),
                _ => {}
            }
        }

        Self {
            positions,
            parenthesis_openings,
            parenthesis_ends,
        }
    }

    fn find_at_or_after(&self, delimiter: ClosingDelimiter, start: usize) -> Option<usize> {
        let positions = &self.positions[delimiter as usize];
        let index = positions.partition_point(|position| *position < start);
        positions.get(index).copied()
    }

    fn matching_parenthesis(&self, opening: usize) -> Option<usize> {
        let slot = self.parenthesis_openings.binary_search(&opening).ok()?;
        self.parenthesis_ends[slot]
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

fn bracket_contents_at<'a>(cursor: &InlineCursor<'a>, offset: usize) -> Option<(&'a str, usize)> {
    let rest = cursor.rest();
    rest.get(offset..)?.strip_prefix('[')?;
    let body_start = offset + '['.len_utf8();
    let close = cursor.find_closing(ClosingDelimiter::Bracket, body_start)?;
    let body = &rest[body_start..close];

    if body.contains('[') {
        return None;
    }

    Some((body, close + ']'.len_utf8()))
}

fn valid_reference_key(key: &str) -> bool {
    !key.is_empty() && !key.starts_with('^') && !key.contains(['[', ']'])
}

fn reference_key(raw: &str) -> Option<&str> {
    let key = raw.trim();
    valid_reference_key(key).then_some(key)
}

fn link_title(raw: &str) -> Option<&str> {
    let title = raw.trim();
    (!title.is_empty() && !title.starts_with('^')).then_some(title)
}

fn parse_inline_note_link<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    let rest = cursor.rest();
    let (title, target_start) = if rest.starts_with(INLINE_NOTE_LINK_BEGIN) {
        (None, INLINE_NOTE_LINK_BEGIN.len())
    } else {
        let (raw_title, title_end) = bracket_contents_at(cursor, 0)?;
        let title = link_title(raw_title)?;
        rest.get(title_end..)?
            .strip_prefix(INLINE_NOTE_LINK_BEGIN)?;
        (Some(title), title_end + INLINE_NOTE_LINK_BEGIN.len())
    };
    let target_end = cursor.find_closing(ClosingDelimiter::NoteLink, target_start)?;
    let target = &rest[target_start..target_end];
    if target.is_empty() {
        return None;
    }
    let end = target_end + INLINE_NOTE_LINK_END.len();
    let raw = &rest[..end];

    cursor.bump(end);
    Some(Inline::NoteLink { raw, title, target })
}

fn parse_inline_footnote<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    cursor.rest().strip_prefix("[^")?;
    let (raw_title, first_end) = bracket_contents_at(cursor, 0)?;
    let raw_title = raw_title.strip_prefix('^')?;
    let (raw_key, second_end) = bracket_contents_at(cursor, first_end)?;

    let (title, key) = if raw_key.is_empty() {
        let key = reference_key(raw_title)?;
        (Some(key), key)
    } else {
        let key = reference_key(raw_key)?;
        let title = if raw_title.is_empty() {
            None
        } else {
            let title = raw_title.trim();
            if title.is_empty() || title.starts_with('^') {
                return None;
            }
            Some(title)
        };
        (title, key)
    };
    let raw = &cursor.rest()[..second_end];

    cursor.bump(second_end);
    Some(Inline::Footnote { raw, title, key })
}

fn parse_inline_reference<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    let (raw_title, first_end) = bracket_contents_at(cursor, 0)?;
    let title = link_title(raw_title)?;
    let (raw_key, second_end) = bracket_contents_at(cursor, first_end)?;

    let (title, key) = if raw_key.is_empty() {
        let key = reference_key(raw_title)?;
        (key, key)
    } else {
        let key = reference_key(raw_key)?;
        (title, key)
    };
    let raw = &cursor.rest()[..second_end];

    cursor.bump(second_end);
    Some(Inline::Reference { raw, title, key })
}

fn parse_inline_direct_link<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    let (raw_title, title_end) = bracket_contents_at(cursor, 0)?;
    let title = link_title(raw_title)?;
    let opening = cursor.pos().checked_add(title_end)?;
    let destination = cursor.rest().get(title_end..)?.strip_prefix('(')?;
    let closing = cursor.delimiters.matching_parenthesis(opening)?;
    let destination_end = closing.checked_sub(opening + '('.len_utf8())?;
    let authored_target = &destination[..destination_end];
    let target = authored_target.trim();
    let end = title_end + '('.len_utf8() + destination_end + ')'.len_utf8();
    let raw = &cursor.rest()[..end];

    cursor.bump(end);
    if is_local_link_target(authored_target) {
        Some(Inline::DirectLink { raw, title, target })
    } else {
        Some(Inline::Text(raw))
    }
}

pub(crate) fn uri_scheme(target: &str) -> Option<&str> {
    let (scheme, _rest) = target.split_once(':')?;
    let mut characters = scheme.chars();
    (characters.next().is_some_and(|ch| ch.is_ascii_alphabetic())
        && characters.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.')))
    .then_some(scheme)
}

pub(crate) fn is_local_link_target(target: &str) -> bool {
    let has_control = target.chars().any(char::is_control);
    let target = target.trim();
    let mut leading = target.chars();
    let has_authority_prefix =
        matches!(leading.next(), Some('/' | '\\')) && matches!(leading.next(), Some('/' | '\\'));

    !target.is_empty() && !has_authority_prefix && uri_scheme(target).is_none() && !has_control
}

fn is_http_url_target(target: &str) -> bool {
    let Some((scheme, body)) = target.split_once("://") else {
        return false;
    };
    !body.is_empty()
        && (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        && !target.contains('<')
        && !target
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
}

fn parse_inline_hyper_link<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    let rest = cursor.rest();
    let (title, target_start) = if rest.starts_with('<') {
        if !starts_http_url_opener(rest) {
            return None;
        }
        (None, '<'.len_utf8())
    } else {
        let (raw_title, title_end) = bracket_contents_at(cursor, 0)?;
        let title = link_title(raw_title)?;
        let destination = rest.get(title_end..)?;
        if !starts_http_url_opener(destination) {
            return None;
        }
        (Some(title), title_end + '<'.len_utf8())
    };
    let Some(target_end) = cursor.find_closing(ClosingDelimiter::Angle, target_start) else {
        cursor.bump(rest.len());
        return Some(Inline::Text(rest));
    };
    let target = &rest[target_start..target_end];
    let end = target_end + '>'.len_utf8();
    let raw = &rest[..end];

    cursor.bump(end);
    if is_http_url_target(target) {
        Some(Inline::HyperLink { raw, title, target })
    } else {
        Some(Inline::Text(raw))
    }
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

fn parse_link_compound<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    // Each parser must leave the cursor unchanged when returning `None`, so the
    // next family sees the same opener and recovery can classify it afterward.
    parse_inline_note_link(cursor)
        .or_else(|| parse_inline_direct_link(cursor))
        .or_else(|| parse_inline_hyper_link(cursor))
        .or_else(|| parse_inline_footnote(cursor))
        .or_else(|| parse_inline_reference(cursor))
}

fn parse_escaped_inline<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    let escaped = cursor
        .rest()
        .strip_prefix('\\')?
        .chars()
        .next()
        .filter(|ch| ch.is_ascii_punctuation())?;
    cursor.bump('\\'.len_utf8());
    let escaped_start = cursor.pos();

    if parse_link_compound(cursor).is_none() {
        match link_compound_recovery(cursor) {
            LinkCompoundRecovery::Closed(end) => cursor.bump(end),
            LinkCompoundRecovery::Incomplete => cursor.bump(cursor.rest().len()),
            LinkCompoundRecovery::Unrecognized => cursor.bump(escaped.len_utf8()),
        }
    }

    Some(Inline::Text(&cursor.source[escaped_start..cursor.pos()]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkCompoundRecovery {
    Unrecognized,
    Closed(usize),
    Incomplete,
}

impl LinkCompoundRecovery {
    fn from_end(end: Option<usize>) -> Self {
        end.map_or(Self::Incomplete, Self::Closed)
    }
}

fn link_compound_recovery(cursor: &InlineCursor<'_>) -> LinkCompoundRecovery {
    let rest = cursor.rest();
    if rest.starts_with(INLINE_NOTE_LINK_BEGIN) {
        let end = cursor
            .find_closing(ClosingDelimiter::NoteLink, INLINE_NOTE_LINK_BEGIN.len())
            .map(|target_end| target_end + INLINE_NOTE_LINK_END.len());
        return LinkCompoundRecovery::from_end(end);
    }
    if starts_http_url_opener(rest) {
        let end = cursor
            .find_closing(ClosingDelimiter::Angle, '<'.len_utf8())
            .map(|target_end| target_end + '>'.len_utf8());
        return LinkCompoundRecovery::from_end(end);
    }

    let Some((raw_title, title_end)) = bracket_contents_at(cursor, 0) else {
        return LinkCompoundRecovery::Unrecognized;
    };
    let after_title = &rest[title_end..];
    if let Some(raw_footnote_title) = raw_title.strip_prefix('^') {
        let title = raw_footnote_title.trim();
        let valid_title =
            raw_footnote_title.is_empty() || (!title.is_empty() && !title.starts_with('^'));
        if !valid_title || !after_title.starts_with('[') {
            return LinkCompoundRecovery::Unrecognized;
        }
        let end = bracket_contents_at(cursor, title_end).map(|(_, end)| end);
        return LinkCompoundRecovery::from_end(end);
    }
    if link_title(raw_title).is_none() {
        return LinkCompoundRecovery::Unrecognized;
    }

    if after_title.starts_with(INLINE_NOTE_LINK_BEGIN) {
        let target_start = title_end + INLINE_NOTE_LINK_BEGIN.len();
        let end = cursor
            .find_closing(ClosingDelimiter::NoteLink, target_start)
            .map(|target_end| target_end + INLINE_NOTE_LINK_END.len());
        return LinkCompoundRecovery::from_end(end);
    }
    if after_title.starts_with('(') {
        let end = cursor
            .pos()
            .checked_add(title_end)
            .and_then(|opening| cursor.delimiters.matching_parenthesis(opening))
            .and_then(|closing| closing.checked_sub(cursor.pos()))
            .and_then(|end| end.checked_add(')'.len_utf8()));
        return LinkCompoundRecovery::from_end(end);
    }
    if starts_http_url_opener(after_title) {
        let target_start = title_end + '<'.len_utf8();
        let end = cursor
            .find_closing(ClosingDelimiter::Angle, target_start)
            .map(|target_end| target_end + '>'.len_utf8());
        return LinkCompoundRecovery::from_end(end);
    }
    if after_title.starts_with('[') {
        let end = bracket_contents_at(cursor, title_end).map(|(_, end)| end);
        return LinkCompoundRecovery::from_end(end);
    }

    LinkCompoundRecovery::Unrecognized
}

fn starts_http_url_opener(source: &str) -> bool {
    ["<http://", "<https://"].iter().any(|prefix| {
        source
            .as_bytes()
            .get(..prefix.len())
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix.as_bytes()))
    })
}

fn parse_incomplete_link_compound<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    if link_compound_recovery(cursor) != LinkCompoundRecovery::Incomplete {
        return None;
    }

    let rest = cursor.rest();
    cursor.bump(rest.len());
    Some(Inline::Text(rest))
}

pub(super) fn parse_inlines<'a>(source: &[&'a str]) -> Vec<Inline<'a>> {
    let mut inlines = vec![];

    for (index, line) in source.iter().enumerate() {
        if index > 0 {
            inlines.push(Inline::SoftBreak);
        }
        inlines.extend(parse_inline(line));
    }

    inlines
}

pub fn parse_inline<'a>(source: &'a str) -> Vec<Inline<'a>> {
    let mut cursor = InlineCursor::new(source);
    let mut inlines = vec![];
    let mut formatting_frames: Vec<FormattingFrame<'a>> = vec![];
    let mut formatting_opener_counts = [0; FormattingKind::COUNT];
    let mut text_start = 0;

    while !cursor.is_eol() {
        let start = cursor.pos();

        if let Some(inline) = parse_inline_code(&mut cursor)
            .or_else(|| parse_link_compound(&mut cursor))
            .or_else(|| parse_incomplete_link_compound(&mut cursor))
            .or_else(|| parse_inline_date_range(&mut cursor))
            .or_else(|| parse_inline_date_stamp(&mut cursor))
            .or_else(|| parse_braced_inline(&mut cursor, "^{", Inline::Superscript))
            .or_else(|| parse_braced_inline(&mut cursor, "_{", Inline::Subscript))
            .or_else(|| parse_braced_inline(&mut cursor, "+{", Inline::Insertion))
            .or_else(|| parse_braced_inline(&mut cursor, "-{", Inline::Deletion))
        {
            if text_start < start {
                inlines.push(Inline::Text(&source[text_start..start]));
            }

            inlines.push(inline);
            text_start = cursor.pos();
        } else if let Some(escaped) = parse_escaped_inline(&mut cursor) {
            if text_start < start {
                inlines.push(Inline::Text(&source[text_start..start]));
            }

            inlines.push(escaped);
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
