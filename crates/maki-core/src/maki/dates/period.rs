use crate::parser::{Date, DateStampTarget, IsoWeek};

pub fn date_page_path(date: Date) -> String {
    format!("/@/dates/{date}")
}

pub fn date_year_page_path(year: u16) -> String {
    format!("/@/dates/{year:04}")
}

pub fn date_month_page_path(year: u16, month: u8) -> String {
    format!("/@/dates/{year:04}-{month:02}")
}

pub fn date_week_page_path(week: IsoWeek) -> String {
    format!("/@/dates/{week}")
}

pub fn date_target_page_path(target: DateStampTarget) -> String {
    match target {
        DateStampTarget::Date(date) => date_page_path(date),
        DateStampTarget::Month(month) => date_month_page_path(month.year(), month.month()),
        DateStampTarget::IsoWeek(week) => date_week_page_path(week),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DatePeriod {
    Year(u16),
    Month { year: u16, month: u8 },
    Week(IsoWeek),
    Day(Date),
}

impl DatePeriod {
    const MIN_YEAR: u16 = 1;
    const MAX_YEAR: u16 = 9999;

    pub fn year(year: u16) -> Option<Self> {
        Self::valid_year(year).then_some(Self::Year(year))
    }

    pub fn month(year: u16, month: u8) -> Option<Self> {
        (Self::valid_year(year) && (1..=12).contains(&month)).then_some(Self::Month { year, month })
    }

    pub fn day(date: Date) -> Option<Self> {
        Self::valid_year(date.year()).then_some(Self::Day(date))
    }

    pub fn week(week: IsoWeek) -> Option<Self> {
        Self::valid_year(week.year()).then_some(Self::Week(week))
    }

    pub fn parse_path_segment(raw: &str) -> Option<Self> {
        if raw.len() == 4 {
            return Self::parse_year(raw).and_then(Self::year);
        }

        let (target, len) = DateStampTarget::parse_prefix(raw)?;
        if len != raw.len() {
            return None;
        }

        match target {
            DateStampTarget::Date(date) => Self::day(date),
            DateStampTarget::Month(month) => Self::month(month.year(), month.month()),
            DateStampTarget::IsoWeek(week) => Self::week(week),
        }
    }

    pub fn title(self) -> String {
        match self {
            Self::Year(year) => format!("{year:04}"),
            Self::Month { year, month } => format!("{year:04}-{month:02}"),
            Self::Week(week) => week.to_string(),
            Self::Day(date) => date.to_string(),
        }
    }

    pub fn path(self) -> String {
        match self {
            Self::Year(year) => date_year_page_path(year),
            Self::Month { year, month } => date_month_page_path(year, month),
            Self::Week(week) => date_week_page_path(week),
            Self::Day(date) => date_page_path(date),
        }
    }

    pub fn parent_path(self) -> String {
        match self {
            Self::Year(_) => "/@/dates".to_string(),
            Self::Month { year, .. } => date_year_page_path(year),
            Self::Week(week) => date_year_page_path(week.year()),
            Self::Day(date) => date_month_page_path(date.year(), date.month()),
        }
    }

    pub fn previous(self) -> Option<Self> {
        match self {
            Self::Year(year) => year
                .checked_sub(1)
                .filter(|year| Self::valid_year(*year))
                .map(Self::Year),
            Self::Month { year, month } if month > 1 => Self::month(year, month - 1),
            Self::Month { year, .. } => year.checked_sub(1).and_then(|year| Self::month(year, 12)),
            Self::Week(week) => week.previous().and_then(Self::week),
            Self::Day(date) => date.previous_day().and_then(Self::day),
        }
    }

    pub fn next(self) -> Option<Self> {
        match self {
            Self::Year(year) => year
                .checked_add(1)
                .filter(|year| Self::valid_year(*year))
                .map(Self::Year),
            Self::Month { year, month } if month < 12 => Self::month(year, month + 1),
            Self::Month { year, .. } => year.checked_add(1).and_then(|year| Self::month(year, 1)),
            Self::Week(week) => week.next().and_then(Self::week),
            Self::Day(date) => date.next_day().and_then(Self::day),
        }
    }

    fn valid_year(year: u16) -> bool {
        (Self::MIN_YEAR..=Self::MAX_YEAR).contains(&year)
    }

    fn parse_year(raw: &str) -> Option<u16> {
        if raw.len() != 4 || !raw.as_bytes().iter().all(u8::is_ascii_digit) {
            return None;
        }

        raw.parse::<u16>().ok().filter(|year| *year > 0)
    }
}
