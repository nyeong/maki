use std::{collections::BTreeMap, fmt};

use super::draft::PropertyItemDraft;

#[derive(Debug, PartialEq)]
pub enum Inline<'a> {
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
pub struct Date {
    year: u16,
    month: u8,
    day: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateStampKind {
    Date,
    Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateStamp<'a> {
    pub(super) kind: DateStampKind,
    pub(super) date: Date,
    pub(super) body: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRange<'a> {
    pub(super) start: DateStamp<'a>,
    pub(super) end: DateStamp<'a>,
}

impl Date {
    pub(super) fn new(year: u16, month: u8, day: u8) -> Option<Self> {
        if year == 0 || month == 0 || month > 12 {
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

    pub fn weekday_abbrev(&self) -> &'static str {
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
    pub fn kind(&self) -> DateStampKind {
        self.kind
    }

    pub fn date(&self) -> Date {
        self.date
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
    pub blocks: Vec<Block<'a>>,
}

impl<'a> Document<'a> {
    pub fn title(&self) -> Option<&'a str> {
        self.props.get_one("title")
    }

    pub fn properties(&self) -> impl Iterator<Item = (&str, &'a str)> {
        self.props.iter()
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
    pub children: Vec<Block<'a>>, // List를 포함하기 위함
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ListKind {
    Unordered,
    Ordered,
}
