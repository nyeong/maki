use super::types::{Date, DateRange, DateStamp, DateStampKind, Inline};

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

    fn previous_char(&self) -> Option<char> {
        self.source[..self.pos].chars().next_back()
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
const INLINE_STRONG_BEGIN_END: &str = "*";
const INLINE_LINK_BEGIN: &str = "[";
const INLINE_LINK_SEPARATOR: &str = "](";
const INLINE_LINK_END: &str = ")";
const INLINE_DATE_RANGE_SEPARATOR: &str = "--";
const PLAIN_URL_PREFIXES: &[&str] = &["https://", "http://"];

fn parse_inline_code<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    let rest = cursor.rest();
    let body = rest.strip_prefix(INLINE_CODE_BEGIN_END)?;
    let end = body.find(INLINE_CODE_BEGIN_END)?;

    let contents = &body[..end];

    cursor.bump(INLINE_CODE_BEGIN_END.len() + contents.len() + INLINE_CODE_BEGIN_END.len());

    Some(Inline::Code(contents))
}

fn parse_inline_strong<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    if cursor.previous_char() == Some('*') {
        return None;
    }

    let rest = cursor.rest();
    let body = rest.strip_prefix(INLINE_STRONG_BEGIN_END)?;
    let first = body.chars().next()?;

    if first.is_whitespace() || first == '*' {
        return None;
    }

    let end = body.char_indices().find_map(|(index, ch)| {
        if ch != '*' {
            return None;
        }

        let contents = &body[..index];
        let before = contents.chars().next_back()?;
        let after = body[index + ch.len_utf8()..].chars().next();

        (!before.is_whitespace() && before != '*' && after != Some('*')).then_some(index)
    })?;
    let contents = &body[..end];

    cursor.bump(INLINE_STRONG_BEGIN_END.len() + contents.len() + INLINE_STRONG_BEGIN_END.len());

    Some(Inline::Strong(parse_inline(contents)))
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

fn parse_date_stamp_at(source: &str) -> Option<(DateStamp<'_>, usize)> {
    let (kind, open, close) = match source.chars().next()? {
        '[' => (DateStampKind::Date, '[', ']'),
        '<' => (DateStampKind::Event, '<', '>'),
        _ => return None,
    };
    let body_start = open.len_utf8();
    let body_source = &source[body_start..];
    let body_end = body_source.find(close)?;
    let body = &body_source[..body_end];
    let (date, date_len) = Date::parse_prefix(body)?;
    let rest = &body[date_len..];

    if !rest.is_empty() && !rest.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }

    Some((
        DateStamp { kind, date, body },
        body_start + body.len() + close.len_utf8(),
    ))
}

fn parse_inline_date_range<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    let rest = cursor.rest();
    let (start, start_len) = parse_date_stamp_at(rest)?;
    let rest_after_start = &rest[start_len..];
    let rest_after_separator = rest_after_start.strip_prefix(INLINE_DATE_RANGE_SEPARATOR)?;
    let (end, end_len) = parse_date_stamp_at(rest_after_separator)?;

    if start.kind() != end.kind() || start.date() > end.date() {
        return None;
    }

    cursor.bump(start_len + INLINE_DATE_RANGE_SEPARATOR.len() + end_len);

    Some(Inline::DateRange(DateRange { start, end }))
}

fn parse_inline_date_stamp<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    let (stamp, len) = parse_date_stamp_at(cursor.rest())?;

    cursor.bump(len);

    Some(Inline::DateStamp(stamp))
}

fn parse_plain_url<'a>(cursor: &mut InlineCursor<'a>) -> Option<Inline<'a>> {
    let rest = cursor.rest();
    let prefix = PLAIN_URL_PREFIXES
        .iter()
        .find(|prefix| rest.starts_with(**prefix))?;
    let raw_end = rest
        .char_indices()
        .find_map(|(index, ch)| (ch.is_whitespace() || ch == '<').then_some(index))
        .unwrap_or(rest.len());
    let target = trim_plain_url_suffix(&rest[..raw_end]);

    if target.len() <= prefix.len() {
        return None;
    }

    cursor.bump(target.len());

    Some(Inline::Link {
        title: target,
        target,
    })
}

fn trim_plain_url_suffix(mut url: &str) -> &str {
    while let Some(ch) = url.chars().next_back() {
        if matches!(ch, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}') {
            url = &url[..url.len() - ch.len_utf8()];
        } else {
            break;
        }
    }

    url
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

/// Parses a given line into Vec<Inline>
pub fn parse_inline<'a>(source: &'a str) -> Vec<Inline<'a>> {
    let mut cursor = InlineCursor::new(source);
    let mut inlines = vec![];
    let mut text_start = 0;

    while !cursor.is_eol() {
        let start = cursor.pos();

        if let Some(inline) = parse_inline_code(&mut cursor)
            .or_else(|| parse_inline_strong(&mut cursor))
            .or_else(|| parse_inline_note_link(&mut cursor))
            .or_else(|| parse_inline_link(&mut cursor))
            .or_else(|| parse_inline_date_range(&mut cursor))
            .or_else(|| parse_inline_date_stamp(&mut cursor))
            .or_else(|| parse_plain_url(&mut cursor))
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
