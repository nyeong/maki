use std::{collections::BTreeMap, fmt};

use super::draft::PropertyItemDraft;

#[derive(Debug, Clone, PartialEq)]
pub enum Inline<'a> {
    NoteLink { target: &'a str },
    Link { title: &'a str, target: &'a str },
    Footnote { label: &'a str },
    HyperLink { target: &'a str },
    Italic(Vec<Inline<'a>>),
    Strong(Vec<Inline<'a>>),
    Superscript(&'a str),
    Subscript(&'a str),
    Insertion(&'a str),
    Deletion(&'a str),
    Highlight(Vec<Inline<'a>>),
    DateStamp(DateStamp<'a>),
    DateRange(DateRange<'a>),
    Text(&'a str),
    SoftBreak,
    Code(&'a str),
}

impl<'a> Inline<'a> {
    pub(crate) fn nested_inlines(&self) -> Option<&[Inline<'a>]> {
        match self {
            Self::Italic(body) | Self::Strong(body) | Self::Highlight(body) => Some(body),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceDefinitionSpelling {
    Canonical,
    FootnoteAlias,
}

pub fn reference_value_is_link_shaped(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && (value.starts_with('/') || !value.chars().any(char::is_whitespace))
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceDefinition<'a> {
    pub key: &'a str,
    pub raw_value: &'a str,
    pub value: Vec<Inline<'a>>,
    pub spelling: ReferenceDefinitionSpelling,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReferenceDefinitions<'a> {
    definitions: Vec<ReferenceDefinition<'a>>,
    local_len: usize,
    by_key: BTreeMap<&'a str, usize>,
}

impl<'a> ReferenceDefinitions<'a> {
    pub(super) fn new(definitions: Vec<ReferenceDefinition<'a>>) -> Self {
        let local_len = definitions.len();
        Self::from_definitions(definitions, local_len)
    }

    pub(super) fn with_inherited<'parent>(
        mut definitions: Vec<ReferenceDefinition<'a>>,
        inherited: &ReferenceDefinitions<'parent>,
    ) -> Self
    where
        'parent: 'a,
    {
        let local_len = definitions.len();
        definitions.extend(inherited.definitions.iter().cloned());
        Self::from_definitions(definitions, local_len)
    }

    fn from_definitions(definitions: Vec<ReferenceDefinition<'a>>, local_len: usize) -> Self {
        let mut by_key = BTreeMap::new();

        for (index, definition) in definitions.iter().enumerate() {
            by_key.entry(definition.key).or_insert(index);
        }

        Self {
            definitions,
            local_len,
            by_key,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &ReferenceDefinition<'a>> {
        self.definitions[..self.local_len].iter()
    }

    pub(super) fn all(&self) -> impl Iterator<Item = &ReferenceDefinition<'a>> {
        self.definitions.iter()
    }

    pub fn get(&self, key: &str) -> Option<&ReferenceDefinition<'a>> {
        self.by_key
            .get(key)
            .and_then(|index| self.definitions.get(*index))
    }

    pub fn link_target(&self, key: &str) -> Option<&'a str> {
        self.get(key).map(|definition| definition.raw_value.trim())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    year: u16,
    month: u8,
    day: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DateMonth {
    year: u16,
    month: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IsoWeek {
    year: u16,
    week: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateStampTarget {
    Date(Date),
    Month(DateMonth),
    IsoWeek(IsoWeek),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateStampKind {
    Date,
    Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateStamp<'a> {
    pub(super) kind: DateStampKind,
    pub(super) target: DateStampTarget,
    pub(super) body: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRange<'a> {
    pub(super) start: DateStamp<'a>,
    pub(super) end: DateStamp<'a>,
}

impl Date {
    const MIN_YEAR: u16 = 1;
    const MAX_YEAR: u16 = 9999;
    const MAX: Self = Self {
        year: Self::MAX_YEAR,
        month: 12,
        day: 31,
    };

    pub(super) fn new(year: u16, month: u8, day: u8) -> Option<Self> {
        if !(Self::MIN_YEAR..=Self::MAX_YEAR).contains(&year) || month == 0 || month > 12 {
            return None;
        }
        if day == 0 || day > days_in_month(year, month) {
            return None;
        }

        Some(Self { year, month, day })
    }

    pub(super) fn parse_prefix(source: &str) -> Option<(Self, usize)> {
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

    pub fn parse(source: &str) -> Option<Self> {
        let (date, len) = Self::parse_prefix(source)?;

        (len == source.len()).then_some(date)
    }

    pub fn next_day(self) -> Option<Self> {
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

    pub fn previous_day(self) -> Option<Self> {
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
    pub fn year(&self) -> u16 {
        self.year
    }

    #[allow(dead_code)]
    pub fn month(&self) -> u8 {
        self.month
    }

    #[allow(dead_code)]
    pub fn day(&self) -> u8 {
        self.day
    }

    pub fn iso_weekday_number(&self) -> u8 {
        match self.weekday_sunday_index() {
            0 => 7,
            index => index,
        }
    }

    pub fn weekday_abbrev(&self) -> &'static str {
        const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

        WEEKDAYS[self.weekday_sunday_index() as usize]
    }

    fn weekday_sunday_index(&self) -> u8 {
        const MONTH_OFFSETS: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];

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

        index as u8
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl DateMonth {
    pub fn new(year: u16, month: u8) -> Option<Self> {
        Date::new(year, month, 1)?;

        Some(Self { year, month })
    }

    pub(super) fn parse_prefix(source: &str) -> Option<(Self, usize)> {
        let bytes = source.as_bytes();
        if bytes.len() < "yyyy-mm".len() || bytes[4] != b'-' {
            return None;
        }
        if !bytes[..4].iter().all(u8::is_ascii_digit) || !bytes[5..7].iter().all(u8::is_ascii_digit)
        {
            return None;
        }

        let year = source[..4].parse::<u16>().ok()?;
        let month = source[5..7].parse::<u8>().ok()?;

        Self::new(year, month).map(|month| (month, 7))
    }

    pub fn year(&self) -> u16 {
        self.year
    }

    pub fn month(&self) -> u8 {
        self.month
    }

    pub fn first_day(&self) -> Date {
        Date::new(self.year, self.month, 1).expect("valid DateMonth has a first day")
    }

    pub fn last_day(&self) -> Date {
        Date::new(self.year, self.month, days_in_month(self.year, self.month))
            .expect("valid DateMonth has a last day")
    }
}

impl fmt::Display for DateMonth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}", self.year, self.month)
    }
}

impl IsoWeek {
    pub fn new(year: u16, week: u8) -> Option<Self> {
        let max_week = weeks_in_iso_year(year)?;
        if week == 0 || week > max_week {
            return None;
        }

        let iso_week = Self { year, week };
        iso_week.date_for_weekday(1)?;

        Some(iso_week)
    }

    pub(super) fn parse_prefix(source: &str) -> Option<(Self, usize)> {
        let bytes = source.as_bytes();
        if bytes.len() < "yyyy-Www".len() || bytes[4] != b'-' || bytes[5] != b'W' {
            return None;
        }
        if !bytes[..4].iter().all(u8::is_ascii_digit) || !bytes[6..8].iter().all(u8::is_ascii_digit)
        {
            return None;
        }

        let year = source[..4].parse::<u16>().ok()?;
        let week = source[6..8].parse::<u8>().ok()?;

        Self::new(year, week).map(|week| (week, 8))
    }

    pub(super) fn parse_weekday_date_prefix(source: &str) -> Option<(Date, usize)> {
        let bytes = source.as_bytes();
        if bytes.len() < "yyyy-Www-d".len()
            || bytes[4] != b'-'
            || bytes[5] != b'W'
            || bytes[8] != b'-'
        {
            return None;
        }
        if !bytes[..4].iter().all(u8::is_ascii_digit)
            || !bytes[6..8].iter().all(u8::is_ascii_digit)
            || !bytes[9].is_ascii_digit()
        {
            return None;
        }

        let year = source[..4].parse::<u16>().ok()?;
        let week = source[6..8].parse::<u8>().ok()?;
        let weekday = source[9..10].parse::<u8>().ok()?;
        let date = Self::new(year, week)?.date_for_weekday(weekday)?;

        Some((date, 10))
    }

    pub fn year(&self) -> u16 {
        self.year
    }

    pub fn week(&self) -> u8 {
        self.week
    }

    pub fn monday(&self) -> Date {
        self.date_for_weekday(1)
            .expect("valid IsoWeek has a Monday")
    }

    pub fn sunday(&self) -> Option<Date> {
        self.date_for_weekday(7)
    }

    pub fn representable_date_range(&self) -> (Date, Date) {
        let start = self.monday();
        let end = self.sunday().unwrap_or(Date::MAX);

        (start, end)
    }

    pub fn date_for_weekday(&self, weekday: u8) -> Option<Date> {
        if !(1..=7).contains(&weekday) {
            return None;
        }

        let mut date = iso_week_one_monday(self.year)?;
        for _ in 1..self.week {
            for _ in 0..7 {
                date = date.next_day()?;
            }
        }
        for _ in 1..weekday {
            date = date.next_day()?;
        }

        Some(date)
    }

    pub fn previous(&self) -> Option<Self> {
        if self.week > 1 {
            return Self::new(self.year, self.week - 1);
        }

        let year = self.year.checked_sub(1)?;
        let week = weeks_in_iso_year(year)?;
        Self::new(year, week)
    }

    pub fn next(&self) -> Option<Self> {
        if self.week < weeks_in_iso_year(self.year)? {
            return Self::new(self.year, self.week + 1);
        }

        self.year.checked_add(1).and_then(|year| Self::new(year, 1))
    }
}

impl fmt::Display for IsoWeek {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-W{:02}", self.year, self.week)
    }
}

impl DateStampTarget {
    pub fn parse_prefix(source: &str) -> Option<(Self, usize)> {
        IsoWeek::parse_weekday_date_prefix(source)
            .map(|(date, len)| (Self::Date(date), len))
            .or_else(|| Date::parse_prefix(source).map(|(date, len)| (Self::Date(date), len)))
            .or_else(|| IsoWeek::parse_prefix(source).map(|(week, len)| (Self::IsoWeek(week), len)))
            .or_else(|| {
                DateMonth::parse_prefix(source).map(|(month, len)| (Self::Month(month), len))
            })
    }

    pub fn exact_date(&self) -> Option<Date> {
        match self {
            Self::Date(date) => Some(*date),
            Self::Month(_) | Self::IsoWeek(_) => None,
        }
    }
}

impl<'a> DateStamp<'a> {
    pub fn kind(&self) -> DateStampKind {
        self.kind
    }

    pub fn target(&self) -> DateStampTarget {
        self.target
    }

    pub fn date(&self) -> Option<Date> {
        self.target.exact_date()
    }

    pub fn body(&self) -> &'a str {
        self.body
    }
}

impl<'a> DateRange<'a> {
    pub fn kind(&self) -> DateStampKind {
        self.start.kind()
    }

    pub fn start(&self) -> DateStamp<'a> {
        self.start
    }

    pub fn end(&self) -> DateStamp<'a> {
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

fn weeks_in_iso_year(year: u16) -> Option<u8> {
    let jan_1 = Date::new(year, 1, 1)?;
    let weekday = jan_1.iso_weekday_number();

    Some(if weekday == 4 || (weekday == 3 && is_leap_year(year)) {
        53
    } else {
        52
    })
}

fn iso_week_one_monday(year: u16) -> Option<Date> {
    let mut date = Date::new(year, 1, 4)?;
    for _ in 1..date.iso_weekday_number() {
        date = date.previous_day()?;
    }

    Some(date)
}

#[derive(Debug, PartialEq, Default)]
pub(super) struct Properties<'a> {
    values: BTreeMap<String, &'a str>,
}

impl<'a> Properties<'a> {
    pub(super) fn new() -> Self {
        Self {
            values: BTreeMap::new(),
        }
    }

    // TODO: PropertyDraft만 받도록 바꾸기
    pub(super) fn extend(&mut self, props: &[PropertyItemDraft<'a>]) {
        for prop in props {
            let key = prop.key.to_lowercase();
            let value = prop.value;
            self.values.insert(key, value);
        }
    }

    pub(super) fn get_one(&self, key: &str) -> Option<&'a str> {
        self.values.get(key).copied()
    }

    fn iter(&self) -> impl Iterator<Item = (&str, &'a str)> {
        self.values
            .iter()
            .map(|(key, value)| (key.as_str(), *value))
    }
}

#[derive(Debug, PartialEq)]
pub struct Document<'a> {
    pub(super) props: Properties<'a>,
    pub(super) references: ReferenceDefinitions<'a>,
    pub blocks: Vec<Block<'a>>,
}

impl<'a> Document<'a> {
    pub fn title(&self) -> Option<&'a str> {
        self.props.get_one("title")
    }

    pub fn properties(&self) -> impl Iterator<Item = (&str, &'a str)> {
        self.props.iter()
    }

    pub fn reference_definitions(&self) -> &ReferenceDefinitions<'a> {
        &self.references
    }

    pub fn reference(&self, key: &str) -> Option<&ReferenceDefinition<'a>> {
        self.references.get(key)
    }

    pub fn link_target(&self, title: &str) -> Option<&'a str> {
        self.references.link_target(title)
    }
}

#[derive(Debug, PartialEq)]
pub struct Block<'a> {
    pub(super) props: Properties<'a>,
    pub kind: BlockKind<'a>,
}

impl<'a> Block<'a> {
    pub fn properties(&self) -> impl Iterator<Item = (&str, &'a str)> {
        self.props.iter()
    }

    pub fn property(&self, key: &str) -> Option<&'a str> {
        self.props.get_one(key)
    }
}

#[derive(Debug, PartialEq)]
pub enum BlockKind<'a> {
    Paragraph {
        body: Vec<Inline<'a>>,
    },
    Code {
        lines: Vec<&'a str>,
        lang: Option<&'a str>,
    },
    Heading {
        level: usize,
        body: Vec<Inline<'a>>,
        raw_body: &'a str,
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
    ReferenceDefinition {
        definitions: Vec<ReferenceDefinition<'a>>,
    },
}

#[derive(Debug, PartialEq)]
pub struct TableRow<'a> {
    pub kind: TableRowKind,
    pub cells: Vec<TableCell<'a>>,
}

impl TableRow<'_> {
    pub fn is_separator(&self) -> bool {
        self.kind == TableRowKind::Separator
    }
}

#[derive(Debug, PartialEq)]
pub struct TableCell<'a> {
    pub body: Vec<Inline<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TableColumnAlignment {
    Text,
    Number,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TableRowKind {
    Data,
    Separator,
}

#[derive(Debug, PartialEq)]
pub struct ListItem<'a> {
    pub body: Vec<Inline<'a>>,
    pub kind: ListKind,
    pub todo: Option<TodoState>,
    pub children: Vec<Block<'a>>, // List를 포함하기 위함
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ListKind {
    Unordered,
    Ordered,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum TodoState {
    Todo,
    Done,
}
