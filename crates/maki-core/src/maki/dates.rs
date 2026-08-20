use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::parser::{self, BlockKind, Date, DateRange, DateStamp, DateStampKind, Inline};

use super::note::{Note, NoteRef};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DateIndex {
    by_date: BTreeMap<Date, Vec<DateBacklink>>,
    index_by_date: BTreeMap<Date, Vec<DateBacklink>>,
    occurrences: BTreeMap<String, DateOccurrence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateOccurrence {
    id: String,
    source_path: PathBuf,
    note_ref: NoteRef,
    note_title: String,
    origin: DateOrigin,
    marker: DateMarker,
    context: String,
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
        date: Date,
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
    Range,
    RangeStart,
    RangeMiddle,
    RangeEnd,
}

#[allow(dead_code)]
impl DateIndex {
    fn insert_occurrence(&mut self, occurrence: DateOccurrence) {
        let id = occurrence.id.clone();

        match &occurrence.marker {
            DateMarker::Single { date, .. } => {
                self.push_backlink(*date, &id, DateRelation::Single);
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

    fn sort_backlinks(&mut self) {
        for backlinks in self.by_date.values_mut() {
            backlinks.sort_by_key(|backlink| backlink.relation.priority());
        }
        for backlinks in self.index_by_date.values_mut() {
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
            Self::Range => "range",
            Self::RangeStart => "range start",
            Self::RangeMiddle => "range",
            Self::RangeEnd => "range end",
        }
    }

    fn is_indexed(self) -> bool {
        !matches!(self, Self::RangeMiddle)
    }

    fn priority(self) -> u8 {
        match self {
            Self::RangeMiddle => 1,
            Self::Single | Self::Range | Self::RangeStart | Self::RangeEnd => 0,
        }
    }
}
pub fn date_page_path(date: Date) -> String {
    format!("/@/dates/{date}")
}

pub fn date_year_page_path(year: u16) -> String {
    format!("/@/dates/{year:04}")
}

pub fn date_month_page_path(year: u16, month: u8) -> String {
    format!("/@/dates/{year:04}-{month:02}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DatePeriod {
    Year(u16),
    Month { year: u16, month: u8 },
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

    pub fn parse_path_segment(raw: &str) -> Option<Self> {
        match raw.len() {
            4 => Self::parse_year(raw).and_then(Self::year),
            7 if raw.as_bytes().get(4) == Some(&b'-') => {
                let year = Self::parse_year(&raw[..4])?;
                let month = Self::parse_two_digits(&raw[5..7])?;
                Self::month(year, month)
            }
            10 if raw.as_bytes().get(4) == Some(&b'-') && raw.as_bytes().get(7) == Some(&b'-') => {
                Date::parse(raw).and_then(Self::day)
            }
            _ => None,
        }
    }

    pub fn title(self) -> String {
        match self {
            Self::Year(year) => format!("{year:04}"),
            Self::Month { year, month } => format!("{year:04}-{month:02}"),
            Self::Day(date) => date.to_string(),
        }
    }

    pub fn path(self) -> String {
        match self {
            Self::Year(year) => date_year_page_path(year),
            Self::Month { year, month } => date_month_page_path(year, month),
            Self::Day(date) => date_page_path(date),
        }
    }

    pub fn parent_path(self) -> String {
        match self {
            Self::Year(_) => "/@/dates".to_string(),
            Self::Month { year, .. } => date_year_page_path(year),
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

    fn parse_two_digits(raw: &str) -> Option<u8> {
        if raw.len() != 2 || !raw.as_bytes().iter().all(u8::is_ascii_digit) {
            return None;
        }

        raw.parse::<u8>().ok()
    }
}

pub fn inline_date_occurrence_id(source_path: &Path, ordinal: usize) -> String {
    date_occurrence_id("inline", source_path, ordinal)
}

pub fn property_date_occurrence_id(source_path: &Path, ordinal: usize) -> String {
    date_occurrence_id("property", source_path, ordinal)
}

pub fn date_occurrence_href(date: Date, occurrence_id: &str) -> String {
    format!("{}#{occurrence_id}", date_page_path(date))
}

fn date_occurrence_id(kind: &str, source_path: &Path, ordinal: usize) -> String {
    format!(
        "date-{kind}-{}-{ordinal}",
        stable_ascii_path_slug(source_path)
    )
}

fn stable_ascii_path_slug(path: &Path) -> String {
    let mut slug = String::new();
    for byte in path.to_string_lossy().as_bytes() {
        match byte {
            b'a'..=b'z' | b'0'..=b'9' => slug.push(*byte as char),
            b'A'..=b'Z' => slug.push(byte.to_ascii_lowercase() as char),
            b'/' | b'.' | b'-' | b'_' => slug.push('-'),
            _ => slug.push_str(&format!("x{byte:02x}")),
        }
    }

    slug.trim_matches('-').to_string()
}
struct DateIndexCollector<'a> {
    index: &'a mut DateIndex,
    source_path: &'a Path,
    note_ref: NoteRef,
    note_title: String,
    inline_ordinal: usize,
    property_ordinal: usize,
}

impl<'a> DateIndexCollector<'a> {
    fn new(
        index: &'a mut DateIndex,
        source_path: &'a Path,
        note_ref: NoteRef,
        note_title: String,
    ) -> Self {
        Self {
            index,
            source_path,
            note_ref,
            note_title,
            inline_ordinal: 0,
            property_ordinal: 0,
        }
    }

    fn push_occurrence(
        &mut self,
        id: String,
        origin: DateOrigin,
        marker: DateMarker,
        context: &str,
    ) {
        self.index.insert_occurrence(DateOccurrence {
            id,
            source_path: self.source_path.to_path_buf(),
            note_ref: self.note_ref.clone(),
            note_title: self.note_title.clone(),
            origin,
            marker,
            context: context.to_string(),
        });
    }

    fn push_inline_stamp(&mut self, stamp: DateStamp<'_>, context: &str) {
        self.inline_ordinal += 1;
        self.push_occurrence(
            inline_date_occurrence_id(self.source_path, self.inline_ordinal),
            DateOrigin::Inline,
            date_stamp_marker(stamp),
            context,
        );
    }

    fn push_inline_range(&mut self, range: DateRange<'_>, context: &str) {
        self.inline_ordinal += 1;
        self.push_occurrence(
            inline_date_occurrence_id(self.source_path, self.inline_ordinal),
            DateOrigin::Inline,
            date_range_marker(range),
            context,
        );
    }

    fn push_property_stamp(&mut self, key: &str, stamp: DateStamp<'_>, context: &str) {
        self.property_ordinal += 1;
        self.push_occurrence(
            property_date_occurrence_id(self.source_path, self.property_ordinal),
            DateOrigin::Property {
                key: key.to_string(),
            },
            date_stamp_marker(stamp),
            context,
        );
    }

    fn push_property_range(&mut self, key: &str, range: DateRange<'_>, context: &str) {
        self.property_ordinal += 1;
        self.push_occurrence(
            property_date_occurrence_id(self.source_path, self.property_ordinal),
            DateOrigin::Property {
                key: key.to_string(),
            },
            date_range_marker(range),
            context,
        );
    }
}

fn date_stamp_marker(stamp: DateStamp<'_>) -> DateMarker {
    DateMarker::Single {
        kind: stamp.kind(),
        date: stamp.date(),
        raw: date_stamp_raw(stamp),
    }
}

fn date_range_marker(range: DateRange<'_>) -> DateMarker {
    DateMarker::Range {
        kind: range.kind(),
        start: range.start().date(),
        end: range.end().date(),
        raw: date_range_raw(range),
    }
}

fn date_stamp_raw(stamp: DateStamp<'_>) -> String {
    let (open, close) = match stamp.kind() {
        DateStampKind::Date => ('[', ']'),
        DateStampKind::Event => ('<', '>'),
    };

    format!("{open}{}{close}", stamp.body())
}

fn date_range_raw(range: DateRange<'_>) -> String {
    format!(
        "{}--{}",
        date_stamp_raw(range.start()),
        date_stamp_raw(range.end())
    )
}

const DATE_CONTEXT_MAX_CHARS: usize = 500;

#[derive(Debug, Clone)]
struct DateHeadingContext {
    level: usize,
    context: String,
}

#[derive(Debug, Clone, Default)]
struct DateTraversalContext {
    headings: Vec<DateHeadingContext>,
    top_list_item: Option<String>,
}

impl DateTraversalContext {
    fn current_heading_context(&self) -> Option<&str> {
        self.headings.last().map(|heading| heading.context.as_str())
    }

    fn parent_heading_context(&self, level: usize) -> Option<&str> {
        self.headings
            .iter()
            .rev()
            .find(|heading| heading.level < level)
            .map(|heading| heading.context.as_str())
    }

    fn enter_heading(&mut self, level: usize, body: &str) {
        self.headings.retain(|heading| heading.level < level);
        self.headings.push(DateHeadingContext {
            level,
            context: heading_date_context(level, body),
        });
    }

    fn with_top_list_item(&self, top_list_item: String) -> Self {
        let mut context = self.clone();
        if context.top_list_item.is_none() {
            context.top_list_item = Some(top_list_item);
        }
        context
    }

    fn contextualize(&self, local_context: &str) -> String {
        date_context_with_scope(
            self.current_heading_context(),
            self.top_list_item.as_deref(),
            local_context,
        )
    }

    fn contextualize_heading(&self, level: usize, local_context: &str) -> String {
        date_context_with_scope(
            self.parent_heading_context(level),
            self.top_list_item.as_deref(),
            local_context,
        )
    }
}

fn truncate_date_context(mut input: String) -> String {
    if let Some((byte_index, _)) = input.char_indices().nth(DATE_CONTEXT_MAX_CHARS) {
        input.truncate(byte_index);
        input.push_str("...");
    }

    input
}

fn push_date_context_part(context: &mut String, part: &str, indent: usize) {
    let part = part.trim_end();
    if part.trim().is_empty() {
        return;
    }

    if !context.is_empty() {
        context.push('\n');
    }

    let prefix = " ".repeat(indent);
    for (index, line) in part.lines().enumerate() {
        if index > 0 {
            context.push('\n');
        }
        if indent > 0 && !line.is_empty() {
            context.push_str(&prefix);
        }
        context.push_str(line);
    }
}

fn date_context_with_scope(
    heading_context: Option<&str>,
    top_list_item: Option<&str>,
    local_context: &str,
) -> String {
    let mut context = String::new();
    if let Some(heading_context) = heading_context {
        push_date_context_part(&mut context, heading_context, 0);
    }
    if let Some(top_list_item) = top_list_item {
        push_date_context_part(&mut context, top_list_item, 0);
    }

    let local_context = local_context.trim_end();
    let duplicates_top_list_item =
        top_list_item.is_some_and(|top_list_item| top_list_item.trim_end() == local_context);
    if !duplicates_top_list_item {
        let indent = if top_list_item.is_some() { 2 } else { 0 };
        push_date_context_part(&mut context, local_context, indent);
    }

    truncate_date_context(context)
}

fn push_inline_date_context(context: &mut String, inlines: &[Inline<'_>]) {
    for inline in inlines {
        match inline {
            Inline::NoteLink { target } => {
                context.push_str("[[");
                context.push_str(target);
                context.push_str("]]");
            }
            Inline::Link { title, target } => {
                context.push('[');
                context.push_str(title);
                context.push_str("](");
                context.push_str(target);
                context.push(')');
            }
            Inline::DateStamp(stamp) => context.push_str(&date_stamp_raw(*stamp)),
            Inline::DateRange(range) => context.push_str(&date_range_raw(*range)),
            Inline::Text(text) => context.push_str(text),
            Inline::SoftBreak => context.push(' '),
            Inline::Code(text) => {
                context.push('`');
                context.push_str(text);
                context.push('`');
            }
            Inline::Strong(body) => {
                context.push('*');
                push_inline_date_context(context, body);
                context.push('*');
            }
        }
    }
}

fn inline_date_context(inlines: &[Inline<'_>]) -> String {
    let mut context = String::new();
    push_inline_date_context(&mut context, inlines);

    truncate_date_context(context)
}

fn heading_date_context(level: usize, body: &str) -> String {
    format!("{} {body}", "=".repeat(level))
}

fn list_item_marker_prefix(kind: parser::ListKind) -> &'static str {
    match kind {
        parser::ListKind::Unordered => "- ",
        parser::ListKind::Ordered => "1. ",
    }
}

fn list_item_line_date_context(item: &parser::ListItem<'_>) -> String {
    let mut context = String::new();
    context.push_str(list_item_marker_prefix(item.kind));
    context.push_str(&inline_date_context(&item.body));

    truncate_date_context(context)
}

fn list_item_date_context(item: &parser::ListItem<'_>) -> String {
    let mut context = String::new();
    context.push_str(list_item_marker_prefix(item.kind));
    context.push_str(&inline_date_context(&item.body));

    for child in &item.children {
        let child_context = block_date_context(child);
        if child_context.trim().is_empty() {
            continue;
        }
        for line in child_context.lines() {
            context.push('\n');
            context.push_str("  ");
            context.push_str(line);
        }
    }

    truncate_date_context(context)
}

fn table_row_date_context(row: &parser::TableRow<'_>) -> String {
    if row.is_separator() {
        return String::from("| --- |");
    }

    let mut context = String::from("| ");
    context.push_str(
        &row.cells
            .iter()
            .map(|cell| inline_date_context(&cell.body))
            .collect::<Vec<_>>()
            .join(" | "),
    );
    context.push_str(" |");

    truncate_date_context(context)
}

fn table_date_context(header: &parser::TableRow<'_>, rows: &[parser::TableRow<'_>]) -> String {
    let mut context = table_row_date_context(header);

    for row in rows {
        context.push('\n');
        context.push_str(&table_row_date_context(row));
    }

    truncate_date_context(context)
}

fn table_body_row_date_context(header_context: &str, row: &parser::TableRow<'_>) -> String {
    let mut context = header_context.to_string();
    context.push('\n');
    context.push_str(&table_row_date_context(row));

    truncate_date_context(context)
}

fn block_date_context(block: &parser::Block<'_>) -> String {
    let context = match &block.kind {
        BlockKind::Paragraph { body } => inline_date_context(body),
        BlockKind::Code { lines, .. } => lines.join("\n"),
        BlockKind::Heading { level, body } => heading_date_context(*level, body),
        BlockKind::List { items } => items
            .iter()
            .map(list_item_date_context)
            .collect::<Vec<_>>()
            .join("\n"),
        BlockKind::Quote { lines } => lines.join("\n"),
        BlockKind::Table { header, rows, .. } => table_date_context(header, rows),
        BlockKind::Container { kind, args, lines } => {
            let mut context = String::from("--- ");
            context.push_str(kind);
            if !args.is_empty() {
                context.push(' ');
                context.push_str(&args.join(" "));
            }
            if !lines.is_empty() {
                context.push('\n');
                context.push_str(&lines.join("\n"));
            }
            context
        }
    };

    truncate_date_context(context)
}

fn property_date_context(key: &str, value: &str, owner_context: &str) -> String {
    let mut context = format!("{key}: {value}");
    if !owner_context.trim().is_empty() {
        context.push('\n');
        context.push_str(owner_context);
    }

    truncate_date_context(context)
}

fn document_date_context(document: &parser::Document<'_>, fallback_title: &str) -> String {
    truncate_date_context(document.title().unwrap_or(fallback_title).to_string())
}

fn collect_inline_dates(
    collector: &mut DateIndexCollector<'_>,
    inlines: &[Inline<'_>],
    context: &str,
) {
    for inline in inlines {
        match inline {
            Inline::DateStamp(stamp) => collector.push_inline_stamp(*stamp, context),
            Inline::DateRange(range) => collector.push_inline_range(*range, context),
            Inline::Strong(body) => collect_inline_dates(collector, body, context),
            Inline::NoteLink { .. }
            | Inline::Link { .. }
            | Inline::Text(_)
            | Inline::SoftBreak
            | Inline::Code(_) => {}
        }
    }
}

fn collect_property_inline_dates(
    collector: &mut DateIndexCollector<'_>,
    key: &str,
    inlines: &[Inline<'_>],
    context: &str,
) {
    for inline in inlines {
        match inline {
            Inline::DateStamp(stamp) => collector.push_property_stamp(key, *stamp, context),
            Inline::DateRange(range) => collector.push_property_range(key, *range, context),
            Inline::Strong(body) => collect_property_inline_dates(collector, key, body, context),
            Inline::NoteLink { .. }
            | Inline::Link { .. }
            | Inline::Text(_)
            | Inline::SoftBreak
            | Inline::Code(_) => {}
        }
    }
}

fn collect_property_dates<'a>(
    collector: &mut DateIndexCollector<'_>,
    properties: impl Iterator<Item = (&'a str, &'a str)>,
    owner_context: &str,
) {
    for (key, value) in properties {
        let context = property_date_context(key, value, owner_context);
        let inlines = parser::parse_inline(value);
        collect_property_inline_dates(collector, key, &inlines, &context);
    }
}

fn collect_list_item_dates(
    collector: &mut DateIndexCollector<'_>,
    item: &parser::ListItem<'_>,
    context: &DateTraversalContext,
) {
    let item_line_context = list_item_line_date_context(item);
    let mut item_context = context.with_top_list_item(item_line_context.clone());
    let occurrence_context = item_context.contextualize(&item_line_context);

    collect_inline_dates(collector, &item.body, &occurrence_context);
    for child in &item.children {
        collect_block_dates(collector, child, &mut item_context);
    }
}

fn collect_table_row_dates(
    collector: &mut DateIndexCollector<'_>,
    row: &parser::TableRow<'_>,
    context: &str,
) {
    if row.is_separator() {
        return;
    }

    for cell in &row.cells {
        collect_inline_dates(collector, &cell.body, context);
    }
}

fn collect_block_dates(
    collector: &mut DateIndexCollector<'_>,
    block: &parser::Block<'_>,
    context: &mut DateTraversalContext,
) {
    let local_context = block_date_context(block);
    let block_context = match &block.kind {
        BlockKind::Heading { level, .. } => context.contextualize_heading(*level, &local_context),
        _ => context.contextualize(&local_context),
    };
    collect_property_dates(collector, block.properties(), &block_context);

    match &block.kind {
        BlockKind::Paragraph { body } => collect_inline_dates(collector, body, &block_context),
        BlockKind::Heading { level, body } => {
            let inlines = parser::parse_inline(body);
            collect_inline_dates(collector, &inlines, &block_context);
            context.enter_heading(*level, body);
        }
        BlockKind::List { items } => {
            for item in items {
                collect_list_item_dates(collector, item, context);
            }
        }
        BlockKind::Quote { lines } => collect_maki_lines_dates(collector, lines, context),
        BlockKind::Table { header, rows, .. } => {
            let table_header_context = table_row_date_context(header);
            let header_context = context.contextualize(&table_header_context);
            collect_table_row_dates(collector, header, &header_context);
            for row in rows {
                let row_context =
                    context.contextualize(&table_body_row_date_context(&table_header_context, row));
                collect_table_row_dates(collector, row, &row_context);
            }
        }
        BlockKind::Container { kind, lines, .. } if *kind == "quote" => {
            collect_maki_lines_dates(collector, lines, context)
        }
        BlockKind::Code { .. } | BlockKind::Container { .. } => {}
    }
}

fn collect_maki_lines_dates(
    collector: &mut DateIndexCollector<'_>,
    lines: &[&str],
    context: &DateTraversalContext,
) {
    let source = lines.join("\n");
    let parsed = parser::parse(&source);
    let mut nested_context = context.clone();
    collect_document_dates_with_context(collector, &parsed.document, &mut nested_context);
}

fn collect_document_dates_with_context(
    collector: &mut DateIndexCollector<'_>,
    document: &parser::Document<'_>,
    context: &mut DateTraversalContext,
) {
    let document_context =
        context.contextualize(&document_date_context(document, &collector.note_title));

    collect_property_dates(collector, document.properties(), &document_context);
    for block in &document.blocks {
        collect_block_dates(collector, block, context);
    }
}

fn collect_document_dates(collector: &mut DateIndexCollector<'_>, document: &parser::Document<'_>) {
    let mut context = DateTraversalContext::default();
    collect_document_dates_with_context(collector, document, &mut context);
}

pub(super) fn collect_date_index(notes: &BTreeMap<NoteRef, Note>) -> DateIndex {
    let mut date_index = DateIndex::default();

    for note in notes.values() {
        let Ok(source) = std::fs::read_to_string(&note.absolute_path) else {
            continue;
        };
        let parsed = parser::parse(&source);
        let note_ref = note.note_ref();
        let note_title = parsed
            .document
            .title()
            .unwrap_or(note.file_stem())
            .to_string();
        let mut collector =
            DateIndexCollector::new(&mut date_index, note.source_path(), note_ref, note_title);
        collect_document_dates(&mut collector, &parsed.document);
    }

    date_index.sort_backlinks();
    date_index
}
