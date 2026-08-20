use crate::parser::{DateRange, DateStamp, DateStampKind};

use super::types::DateMarker;

pub(super) fn date_stamp_marker(stamp: DateStamp<'_>) -> DateMarker {
    DateMarker::Single {
        kind: stamp.kind(),
        date: stamp.date(),
        raw: date_stamp_raw(stamp),
    }
}

pub(super) fn date_range_marker(range: DateRange<'_>) -> DateMarker {
    DateMarker::Range {
        kind: range.kind(),
        start: range.start().date(),
        end: range.end().date(),
        raw: date_range_raw(range),
    }
}

pub(super) fn date_stamp_raw(stamp: DateStamp<'_>) -> String {
    let (open, close) = match stamp.kind() {
        DateStampKind::Date => ('[', ']'),
        DateStampKind::Event => ('<', '>'),
    };

    format!("{open}{}{close}", stamp.body())
}

pub(super) fn date_range_raw(range: DateRange<'_>) -> String {
    format!(
        "{}--{}",
        date_stamp_raw(range.start()),
        date_stamp_raw(range.end())
    )
}
