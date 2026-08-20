use crate::parser::DateStampKind;

pub(in crate::html) const DATE_RANGE_SEPARATOR_HTML: &str = "&ndash;";

pub(in crate::html) fn date_stamp_delimiters(kind: DateStampKind) -> (char, char) {
    match kind {
        DateStampKind::Date => ('[', ']'),
        DateStampKind::Event => ('<', '>'),
    }
}

pub(in crate::html) fn date_stamp_class(kind: DateStampKind) -> &'static str {
    match kind {
        DateStampKind::Date => "maki-date-stamp maki-date-stamp-reference",
        DateStampKind::Event => "maki-date-stamp maki-date-stamp-event",
    }
}

pub(in crate::html) fn date_marker_kind_label(kind: DateStampKind) -> &'static str {
    match kind {
        DateStampKind::Date => "date",
        DateStampKind::Event => "event",
    }
}
