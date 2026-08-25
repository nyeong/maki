use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::parser::{Date, DateStampKind, DateStampTarget};

use super::super::note::NoteRef;
use super::period::DatePeriod;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DateIndex {
    by_date: BTreeMap<Date, Vec<DateBacklink>>,
    index_by_date: BTreeMap<Date, Vec<DateBacklink>>,
    by_period: BTreeMap<DatePeriod, Vec<DateBacklink>>,
    occurrences: BTreeMap<String, DateOccurrence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateOccurrence {
    pub(in crate::maki::dates) id: String,
    pub(in crate::maki::dates) source_path: PathBuf,
    pub(in crate::maki::dates) note_ref: NoteRef,
    pub(in crate::maki::dates) note_title: String,
    pub(in crate::maki::dates) origin: DateOrigin,
    pub(in crate::maki::dates) marker: DateMarker,
    pub(in crate::maki::dates) context: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateOrigin {
    Inline,
    Property { key: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateMarker {
    Single {
        kind: DateStampKind,
        target: DateStampTarget,
        raw: String,
    },
    Range {
        kind: DateStampKind,
        start: Date,
        end: Date,
        raw: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateBacklink {
    occurrence_id: String,
    relation: DateRelation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateRelation {
    Single,
    Month,
    Week,
    MonthDay,
    WeekDay,
    Range,
    RangeStart,
    RangeMiddle,
    RangeEnd,
}

#[allow(dead_code)]
impl DateIndex {
    pub(in crate::maki::dates) fn insert_occurrence(&mut self, occurrence: DateOccurrence) {
        let id = occurrence.id.clone();

        match &occurrence.marker {
            DateMarker::Single { target, .. } => {
                self.push_single_backlinks(*target, &id);
            }
            DateMarker::Range { start, end, .. } => {
                let mut date = *start;
                loop {
                    let relation = if start == end {
                        DateRelation::Range
                    } else if date == *start {
                        DateRelation::RangeStart
                    } else if date == *end {
                        DateRelation::RangeEnd
                    } else {
                        DateRelation::RangeMiddle
                    };
                    self.push_backlink(date, &id, relation);

                    if date == *end {
                        break;
                    }
                    let Some(next) = date.next_day() else {
                        break;
                    };
                    date = next;
                }
            }
        }

        self.occurrences.insert(id, occurrence);
    }

    fn push_single_backlinks(&mut self, target: DateStampTarget, occurrence_id: &str) {
        match target {
            DateStampTarget::Date(date) => {
                self.push_backlink(date, occurrence_id, DateRelation::Single);
            }
            DateStampTarget::Month(month) => {
                self.push_period_backlink(
                    DatePeriod::Month {
                        year: month.year(),
                        month: month.month(),
                    },
                    occurrence_id,
                    DateRelation::Month,
                );
                self.push_date_span_backlinks(
                    month.first_day(),
                    month.last_day(),
                    occurrence_id,
                    DateRelation::MonthDay,
                );
            }
            DateStampTarget::IsoWeek(week) => {
                self.push_period_backlink(
                    DatePeriod::Week(week),
                    occurrence_id,
                    DateRelation::Week,
                );
                let (start, end) = week.representable_date_range();
                self.push_date_span_backlinks(start, end, occurrence_id, DateRelation::WeekDay);
            }
        }
    }

    fn push_date_span_backlinks(
        &mut self,
        start: Date,
        end: Date,
        occurrence_id: &str,
        relation: DateRelation,
    ) {
        let mut date = start;
        loop {
            self.push_backlink(date, occurrence_id, relation);

            if date == end {
                break;
            }
            let Some(next) = date.next_day() else {
                break;
            };
            date = next;
        }
    }

    fn push_backlink(&mut self, date: Date, occurrence_id: &str, relation: DateRelation) {
        let backlink = DateBacklink {
            occurrence_id: occurrence_id.to_string(),
            relation,
        };

        self.by_date.entry(date).or_default().push(backlink.clone());
        if relation.is_indexed() {
            self.index_by_date.entry(date).or_default().push(backlink);
        }
    }

    fn push_period_backlink(
        &mut self,
        period: DatePeriod,
        occurrence_id: &str,
        relation: DateRelation,
    ) {
        let backlink = DateBacklink {
            occurrence_id: occurrence_id.to_string(),
            relation,
        };

        self.by_period.entry(period).or_default().push(backlink);
    }

    pub(in crate::maki::dates) fn sort_backlinks(&mut self) {
        for backlinks in self.by_date.values_mut() {
            backlinks.sort_by_key(|backlink| backlink.relation.priority());
        }
        for backlinks in self.index_by_date.values_mut() {
            backlinks.sort_by_key(|backlink| backlink.relation.priority());
        }
        for backlinks in self.by_period.values_mut() {
            backlinks.sort_by_key(|backlink| backlink.relation.priority());
        }
    }

    pub fn dates(&self) -> impl DoubleEndedIterator<Item = (&Date, &[DateBacklink])> {
        self.index_by_date
            .iter()
            .map(|(date, backlinks)| (date, backlinks.as_slice()))
    }

    pub fn backlinks_for(&self, date: &Date) -> Option<&[DateBacklink]> {
        self.by_date.get(date).map(Vec::as_slice)
    }

    pub fn periods(&self) -> impl DoubleEndedIterator<Item = (&DatePeriod, &[DateBacklink])> {
        self.by_period
            .iter()
            .map(|(period, backlinks)| (period, backlinks.as_slice()))
    }

    pub fn backlinks_for_period(&self, period: DatePeriod) -> Option<&[DateBacklink]> {
        self.by_period.get(&period).map(Vec::as_slice)
    }

    pub fn occurrence(&self, id: &str) -> Option<&DateOccurrence> {
        self.occurrences.get(id)
    }
}

#[allow(dead_code)]
impl DateOccurrence {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub fn note_ref(&self) -> &NoteRef {
        &self.note_ref
    }

    pub fn note_title(&self) -> &str {
        &self.note_title
    }

    pub fn origin(&self) -> &DateOrigin {
        &self.origin
    }

    pub fn marker(&self) -> &DateMarker {
        &self.marker
    }

    pub fn context(&self) -> &str {
        &self.context
    }
}

#[allow(dead_code)]
impl DateMarker {
    pub fn kind(&self) -> DateStampKind {
        match self {
            Self::Single { kind, .. } | Self::Range { kind, .. } => *kind,
        }
    }

    pub fn raw(&self) -> &str {
        match self {
            Self::Single { raw, .. } | Self::Range { raw, .. } => raw,
        }
    }
}

#[allow(dead_code)]
impl DateBacklink {
    pub fn occurrence_id(&self) -> &str {
        &self.occurrence_id
    }

    pub fn relation(&self) -> DateRelation {
        self.relation
    }
}

#[allow(dead_code)]
impl DateRelation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Month => "month",
            Self::Week => "week",
            Self::MonthDay => "month day",
            Self::WeekDay => "week day",
            Self::Range => "range",
            Self::RangeStart => "range start",
            Self::RangeMiddle => "range",
            Self::RangeEnd => "range end",
        }
    }

    fn is_indexed(self) -> bool {
        !matches!(self, Self::RangeMiddle | Self::Month | Self::Week)
    }

    fn priority(self) -> u8 {
        match self {
            Self::Single
            | Self::Month
            | Self::Week
            | Self::Range
            | Self::RangeStart
            | Self::RangeEnd => 0,
            Self::RangeMiddle => 1,
            Self::MonthDay | Self::WeekDay => 2,
        }
    }
}
