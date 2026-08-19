//! Maki parser
//!
//! ### Properties
//!
//! 문법적으로, 각 property에는 별 의미가 없다.
//! 그러나 maki, renderer 등에서 특별한 의미를 담을 수도 있다.

use std::collections::BTreeMap;
use std::fmt;

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
    Strong(Vec<Inline<'a>>),
    DateStamp(DateStamp<'a>),
    DateRange(DateRange<'a>),
    Text(&'a str),
    SoftBreak,
    Code(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Date {
    year: u16,
    month: u8,
    day: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DateStampKind {
    Date,
    Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DateStamp<'a> {
    kind: DateStampKind,
    date: Date,
    body: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DateRange<'a> {
    start: DateStamp<'a>,
    end: DateStamp<'a>,
}

impl Date {
    fn new(year: u16, month: u8, day: u8) -> Option<Self> {
        if year == 0 || month == 0 || month > 12 {
            return None;
        }
        if day == 0 || day > days_in_month(year, month) {
            return None;
        }

        Some(Self { year, month, day })
    }

    fn parse_prefix(source: &str) -> Option<(Self, usize)> {
        let bytes = source.as_bytes();
        if bytes.len() < "yyyy-mm-dd".len() {
            return None;
        }
        if bytes[4] != b'-' || bytes[7] != b'-' {
            return None;
        }
        if !bytes[..4].iter().all(u8::is_ascii_digit)
            || !bytes[5..7].iter().all(u8::is_ascii_digit)
            || !bytes[8..10].iter().all(u8::is_ascii_digit)
        {
            return None;
        }

        let year = source[..4].parse::<u16>().ok()?;
        let month = source[5..7].parse::<u8>().ok()?;
        let day = source[8..10].parse::<u8>().ok()?;

        Self::new(year, month, day).map(|date| (date, 10))
    }

    pub(crate) fn parse(source: &str) -> Option<Self> {
        let (date, len) = Self::parse_prefix(source)?;

        (len == source.len()).then_some(date)
    }

    pub(crate) fn next_day(self) -> Option<Self> {
        if self.day < days_in_month(self.year, self.month) {
            return Self::new(self.year, self.month, self.day + 1);
        }
        if self.month < 12 {
            return Self::new(self.year, self.month + 1, 1);
        }
        self.year
            .checked_add(1)
            .and_then(|year| Self::new(year, 1, 1))
    }

    pub(crate) fn previous_day(self) -> Option<Self> {
        if self.day > 1 {
            return Self::new(self.year, self.month, self.day - 1);
        }
        if self.month > 1 {
            let month = self.month - 1;
            return Self::new(self.year, month, days_in_month(self.year, month));
        }
        self.year
            .checked_sub(1)
            .and_then(|year| Self::new(year, 12, 31))
    }

    #[allow(dead_code)]
    pub(crate) fn year(&self) -> u16 {
        self.year
    }

    #[allow(dead_code)]
    pub(crate) fn month(&self) -> u8 {
        self.month
    }

    #[allow(dead_code)]
    pub(crate) fn day(&self) -> u8 {
        self.day
    }

    pub(crate) fn weekday_abbrev(&self) -> &'static str {
        const MONTH_OFFSETS: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

        // Sakamoto's algorithm, with 0 representing Sunday.
        let month = i32::from(self.month);
        let mut year = i32::from(self.year);
        if month < 3 {
            year -= 1;
        }
        let index = (year + year / 4 - year / 100
            + year / 400
            + MONTH_OFFSETS[(month - 1) as usize]
            + i32::from(self.day))
        .rem_euclid(7);

        WEEKDAYS[index as usize]
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl<'a> DateStamp<'a> {
    pub(crate) fn kind(&self) -> DateStampKind {
        self.kind
    }

    pub(crate) fn date(&self) -> Date {
        self.date
    }

    pub(crate) fn body(&self) -> &'a str {
        self.body
    }
}

impl<'a> DateRange<'a> {
    pub(crate) fn kind(&self) -> DateStampKind {
        self.start.kind()
    }

    pub(crate) fn start(&self) -> DateStamp<'a> {
        self.start
    }

    pub(crate) fn end(&self) -> DateStamp<'a> {
        self.end
    }
}

fn is_leap_year(year: u16) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
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

fn build_table_row<'a>(kind: TableRowKind, cells: &[&'a str]) -> TableRow<'a> {
    TableRow {
        kind,
        cells: cells
            .iter()
            .map(|cell| TableCell {
                body: parse_inline(cell),
            })
            .collect(),
    }
}

fn is_integer_table_cell(cell: &str) -> bool {
    let cell = cell.trim();

    !cell.is_empty() && cell.bytes().all(|byte| byte.is_ascii_digit())
}

fn table_column_alignments(
    rows: &[TableRowDraft<'_>],
    column_count: usize,
) -> Vec<TableColumnAlignment> {
    (0..column_count)
        .map(|column| {
            let mut has_data_rows = false;

            if rows.iter().all(|row| {
                if row.kind == TableRowKind::Separator {
                    return true;
                }
                has_data_rows = true;
                row.cells
                    .get(column)
                    .is_some_and(|cell| is_integer_table_cell(cell))
            }) && has_data_rows
            {
                TableColumnAlignment::Number
            } else {
                TableColumnAlignment::Text
            }
        })
        .collect()
}

fn build_table_block<'a>(
    header: &[&'a str],
    rows: &[TableRowDraft<'a>],
    props: Properties<'a>,
) -> Block<'a> {
    Block {
        kind: BlockKind::Table {
            header: build_table_row(TableRowKind::Data, header),
            alignments: table_column_alignments(rows, header.len()),
            rows: rows
                .iter()
                .map(|row| build_table_row(row.kind, &row.cells))
                .collect(),
        },
        props,
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
        BlockDraft::Table { header, rows } => build_table_block(header, rows, props),
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

    fn iter(&self) -> impl Iterator<Item = (&str, &'a str)> {
        self.values
            .iter()
            .map(|(key, value)| (key.as_str(), *value))
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

    pub(crate) fn properties(&self) -> impl Iterator<Item = (&str, &'a str)> {
        self.props.iter()
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct Block<'a> {
    props: Properties<'a>,
    pub(crate) kind: BlockKind<'a>,
}

impl<'a> Block<'a> {
    pub(crate) fn properties(&self) -> impl Iterator<Item = (&str, &'a str)> {
        self.props.iter()
    }
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
    Table {
        header: TableRow<'a>,
        alignments: Vec<TableColumnAlignment>,
        rows: Vec<TableRow<'a>>,
    },
    Container {
        kind: &'a str,
        args: Vec<&'a str>,
        lines: Vec<&'a str>,
    },
}

#[derive(Debug, PartialEq)]
pub(crate) struct TableRow<'a> {
    pub(crate) kind: TableRowKind,
    pub(crate) cells: Vec<TableCell<'a>>,
}

impl TableRow<'_> {
    pub(crate) fn is_separator(&self) -> bool {
        self.kind == TableRowKind::Separator
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct TableCell<'a> {
    pub(crate) body: Vec<Inline<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum TableColumnAlignment {
    Text,
    Number,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum TableRowKind {
    Data,
    Separator,
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

    Table {
        header: Vec<&'a str>,
        rows: Vec<TableRowDraft<'a>>,
    },
}

#[derive(Debug, PartialEq)]
struct TableRowDraft<'a> {
    kind: TableRowKind,
    cells: Vec<&'a str>,
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
        if !raw_lines.is_empty()
            && (cursor.peek().is_some_and(starts_block_after_paragraph)
                || cursor_starts_table(cursor))
        {
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
    Some(table_line_body(line)?.split('|').map(str::trim).collect())
}

fn is_table_separator_part(part: &str) -> bool {
    let part = part.trim();

    part.len() >= 3 && part.bytes().all(|byte| byte == b'-')
}

fn parse_table_separator_columns(line: &LineToken<'_>) -> Option<usize> {
    let parts = table_line_body(line)?.split('+').collect::<Vec<_>>();

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
        } else if let Some(draft) = parse_table_draft(&mut cursor) {
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
    fn parse_inline_supports_star_delimited_strong_text() {
        assert_eq!(
            parse_inline("Use *bold `code` and [link](/target)* now."),
            vec![
                Inline::Text("Use "),
                Inline::Strong(vec![
                    Inline::Text("bold "),
                    Inline::Code("code"),
                    Inline::Text(" and "),
                    Inline::Link {
                        title: "link",
                        target: "/target"
                    }
                ]),
                Inline::Text(" now.")
            ]
        );
    }

    #[test]
    fn parse_inline_keeps_loose_stars_as_text() {
        assert_eq!(
            parse_inline("Do not parse * loose * or **double**."),
            vec![Inline::Text("Do not parse * loose * or **double**.")]
        );
    }

    #[test]
    fn parse_inline_autolinks_plain_http_urls() {
        assert_eq!(
            parse_inline("Read https://example.com/docs, then `https://example.com/code`."),
            vec![
                Inline::Text("Read "),
                Inline::Link {
                    title: "https://example.com/docs",
                    target: "https://example.com/docs"
                },
                Inline::Text(", then "),
                Inline::Code("https://example.com/code"),
                Inline::Text(".")
            ]
        );
    }

    #[test]
    fn parse_inline_date_stamps_and_ranges() {
        assert_eq!(
            parse_inline("On [2026-08-15 토] and <2026-08-16> through [2026-08-17]--[2026-08-19]."),
            vec![
                Inline::Text("On "),
                Inline::DateStamp(DateStamp {
                    kind: DateStampKind::Date,
                    date: Date::new(2026, 8, 15).unwrap(),
                    body: "2026-08-15 토",
                }),
                Inline::Text(" and "),
                Inline::DateStamp(DateStamp {
                    kind: DateStampKind::Event,
                    date: Date::new(2026, 8, 16).unwrap(),
                    body: "2026-08-16",
                }),
                Inline::Text(" through "),
                Inline::DateRange(DateRange {
                    start: DateStamp {
                        kind: DateStampKind::Date,
                        date: Date::new(2026, 8, 17).unwrap(),
                        body: "2026-08-17",
                    },
                    end: DateStamp {
                        kind: DateStampKind::Date,
                        date: Date::new(2026, 8, 19).unwrap(),
                        body: "2026-08-19",
                    },
                }),
                Inline::Text("."),
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
    fn parse_table_with_inline_cells_and_numeric_alignment() {
        let parsed = parse(
            r#"| 이름 | 점수 | 취득일 |
|---+---+---|
| `Alice` | 10 | [2026-08-15] |
| Bob | 2 | [2026-08-16] |"#,
        );

        assert!(parsed.diagnostics.is_empty());
        assert_eq!(parsed.document.blocks.len(), 1);

        let BlockKind::Table {
            header,
            alignments,
            rows,
        } = &parsed.document.blocks[0].kind
        else {
            panic!("expected a table block");
        };

        assert_eq!(header.cells[0].body, vec![Inline::Text("이름")]);
        assert_eq!(header.cells[1].body, vec![Inline::Text("점수")]);
        assert_eq!(
            alignments,
            &vec![
                TableColumnAlignment::Text,
                TableColumnAlignment::Number,
                TableColumnAlignment::Text
            ]
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, TableRowKind::Data);
        assert_eq!(rows[0].cells[0].body, vec![Inline::Code("Alice")]);
        assert_eq!(rows[0].cells[1].body, vec![Inline::Text("10")]);
        assert_eq!(
            rows[0].cells[2].body,
            vec![Inline::DateStamp(DateStamp {
                kind: DateStampKind::Date,
                date: Date::new(2026, 8, 15).unwrap(),
                body: "2026-08-15",
            })]
        );
    }

    #[test]
    fn parse_table_starts_after_paragraph_without_blank_line() {
        let parsed = parse(
            r#"intro
| 이름 | 점수 |
|---+---|
| Alice | 10 |"#,
        );

        assert!(parsed.diagnostics.is_empty());
        assert_eq!(parsed.document.blocks.len(), 2);

        let BlockKind::Paragraph { body } = &parsed.document.blocks[0].kind else {
            panic!("expected the first block to stay paragraph");
        };
        assert_eq!(body, &vec![Inline::Text("intro")]);

        let BlockKind::Table { rows, .. } = &parsed.document.blocks[1].kind else {
            panic!("expected the second block to become table");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, TableRowKind::Data);
    }

    #[test]
    fn parse_table_keeps_hyphen_cell_when_separator_width_differs() {
        let parsed = parse(
            r#"| 이름 | 값 |
|---+---|
| dash | --- |"#,
        );

        assert!(parsed.diagnostics.is_empty());

        let BlockKind::Table { rows, .. } = &parsed.document.blocks[0].kind else {
            panic!("expected a table block");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, TableRowKind::Data);
        assert_eq!(rows[0].cells[1].body, vec![Inline::Text("---")]);
    }

    #[test]
    fn parse_table_keeps_middle_separator_inside_table() {
        let parsed = parse(
            r#"| 일시 | 시간 |
|---+---|
| [2025-11-05 Wed] | 5H |
|---+---|
| [2026-04-04 Sat] | 5H |"#,
        );

        assert!(parsed.diagnostics.is_empty());
        assert_eq!(parsed.document.blocks.len(), 1);

        let BlockKind::Table {
            alignments, rows, ..
        } = &parsed.document.blocks[0].kind
        else {
            panic!("expected a table block");
        };

        assert_eq!(
            alignments,
            &vec![TableColumnAlignment::Text, TableColumnAlignment::Text]
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, TableRowKind::Data);
        assert_eq!(rows[1].kind, TableRowKind::Separator);
        assert!(rows[1].cells.is_empty());
        assert_eq!(rows[2].kind, TableRowKind::Data);
        assert_eq!(
            rows[2].cells[0].body,
            vec![Inline::DateStamp(DateStamp {
                kind: DateStampKind::Date,
                date: Date::new(2026, 4, 4).unwrap(),
                body: "2026-04-04 Sat",
            })]
        );
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
