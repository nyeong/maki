use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    maki::{
        self, DateBacklink, DateIndex, DateMarker, DateOccurrence, DateOrigin, DatePeriod,
        DateRelation, ProjectDiagnostic, ProjectDiagnosticKind, ProjectDiagnosticSummary,
        RecentEntry, SearchEntry, SearchEntryKind, SitemapEntry,
    },
    parser::{Date, DateMonth, DateStampTarget, IsoWeek},
};

use super::{
    context::{AssetMode, DocumentNavigationItem, RenderContext},
    date_markup::date_marker_kind_label,
    render_maki_source_with_context,
    renderer::Renderer,
};

const META_TEMPLATE: &str = include_str!("../../../../templates/meta.maki");
const DATES_INDEX_TEMPLATE: &str = include_str!("../../../../templates/dates-index.maki");
const DIAGNOSTICS_TEMPLATE: &str = include_str!("../../../../templates/diagnostics.maki");
const SITEMAP_TEMPLATE: &str = include_str!("../../../../templates/sitemap.maki");
const KST_OFFSET_SECONDS: u64 = 9 * 60 * 60;

pub fn render_subdocuments_page(
    parent: &DocumentNavigationItem,
    children: &[DocumentNavigationItem],
    asset_mode: AssetMode,
    site_title: Option<&str>,
    site_header: bool,
) -> String {
    let title = format!("Subdocuments of {}", parent.title());
    let mut renderer = Renderer::new_with_context(
        RenderContext::default()
            .with_asset_mode(asset_mode)
            .with_site_title(site_title)
            .with_site_header(site_header),
    );

    renderer.begin_project_page(&title);
    renderer.push_raw("<nav class=\"maki-subdocuments-parent\" aria-label=\"Parent document\">");
    renderer.push_raw("<span class=\"maki-document-navigation-label\">Parent document</span>");
    renderer.render_anchor(parent.path(), parent.title());
    renderer.push_raw("</nav><main class=\"maki-subdocuments-page\">");

    if children.is_empty() {
        renderer.push_raw("<p class=\"maki-subdocuments-empty\">No subdocuments.</p>");
    } else {
        renderer.push_raw("<ul class=\"maki-subdocuments-list\">");
        for child in children {
            renderer.push_raw("<li>");
            renderer.render_anchor(child.path(), child.title());
            renderer.push_raw("</li>");
        }
        renderer.push_raw("</ul>");
    }

    renderer.push_raw("</main></body></html>");
    renderer.into_html()
}

pub fn render_search_page(
    query: &str,
    results: &[SearchEntry],
    total_entries: usize,
    asset_mode: AssetMode,
    site_title: Option<&str>,
    site_header: bool,
) -> String {
    let mut renderer = Renderer::new_with_context(
        RenderContext::default()
            .with_asset_mode(asset_mode)
            .with_site_title(site_title)
            .with_site_header(site_header),
    );
    renderer.begin_project_page("Search");
    renderer.push_raw("<main class=\"maki-search-page\">");
    renderer.push_raw("<p class=\"maki-search-summary\">");
    if query.trim().is_empty() {
        renderer.push_raw(&format!("Showing {total_entries} entries."));
    } else {
        renderer.push_raw(&format!("{} matches for ", results.len()));
        renderer.push_raw("<code>");
        renderer.escape_html_into(query);
        renderer.push_raw("</code>.");
    }
    renderer.push_raw("</p>");

    if results.is_empty() {
        renderer.push_raw("<p class=\"maki-search-empty\">No matching entries.</p>");
    } else {
        renderer.push_raw("<ul class=\"maki-search-page-results\">");
        for entry in results {
            renderer.push_raw("<li><a href=\"");
            renderer.escape_html_attr_into(entry.path());
            renderer.push_raw("\">");
            if entry.kind() == SearchEntryKind::Heading {
                renderer.push_raw("#");
            } else if entry.kind() == SearchEntryKind::Id {
                renderer.push_raw("@");
            }
            renderer.escape_html_into(entry.title());
            renderer.push_raw("</a><span>");
            renderer.escape_html_into(entry.kind().as_str());
            renderer.push_raw(": ");
            renderer.escape_html_into(entry.source_path());
            renderer.push_raw("</span></li>");
        }
        renderer.push_raw("</ul>");
    }

    renderer.push_raw("</main></body></html>");
    renderer.into_html()
}

pub fn render_meta_index_page(
    asset_mode: AssetMode,
    site_title: Option<&str>,
    site_header: bool,
) -> String {
    render_project_maki_source(META_TEMPLATE, asset_mode, site_title, site_header)
}

pub fn render_sitemap_page(
    entries: &[SitemapEntry],
    asset_mode: AssetMode,
    site_title: Option<&str>,
    site_header: bool,
) -> String {
    let body = sitemap_page_body_source(entries);
    let source = render_maki_template(SITEMAP_TEMPLATE, &[("{{body}}", &body)]);

    render_project_maki_source(&source, asset_mode, site_title, site_header)
}

pub fn render_recents_page(
    entries: &[RecentEntry],
    asset_mode: AssetMode,
    site_title: Option<&str>,
    site_header: bool,
) -> String {
    let mut renderer = Renderer::new_with_context(
        RenderContext::default()
            .with_asset_mode(asset_mode)
            .with_site_title(site_title)
            .with_site_header(site_header),
    );
    renderer.begin_project_page("Recents");

    if entries.is_empty() {
        renderer.push_raw("<p>No notes.</p>");
    } else {
        renderer.push_raw("<ul>");
        for entry in entries {
            let mut href = String::new();
            push_maki_direct_link_target(&mut href, entry.path());
            let href = href.trim();
            let authored_title;
            let title = if entry.uses_path_label() {
                entry.title()
            } else {
                authored_title = direct_link_safe_title(entry.title());
                &authored_title
            };
            renderer.push_raw("<li>");
            renderer.escape_html_into(&modified_time_kst_label(entry.modified()));
            renderer.push_raw(" ");
            if maki::is_safe_direct_href(href) {
                renderer.render_anchor(href, title);
            } else {
                renderer.escape_html_into(title);
            }
            renderer.push_raw("</li>");
        }
        renderer.push_raw("</ul>");
    }

    renderer.push_raw("</body></html>");
    renderer.into_html()
}

pub fn render_date_index_page(
    date_index: &DateIndex,
    asset_mode: AssetMode,
    site_title: Option<&str>,
    site_header: bool,
) -> String {
    let body = date_index_page_body_source(date_index);
    let source = render_maki_template(DATES_INDEX_TEMPLATE, &[("{{body}}", &body)]);

    render_project_maki_source(&source, asset_mode, site_title, site_header)
}

pub fn render_date_period_page(
    period: DatePeriod,
    date_index: &DateIndex,
    asset_mode: AssetMode,
    site_title: Option<&str>,
    site_header: bool,
) -> String {
    let title = date_period_title(period);
    let mut renderer = Renderer::new_with_context(
        RenderContext::default()
            .with_asset_mode(asset_mode)
            .with_site_title(site_title)
            .with_site_header(site_header),
    );

    renderer.begin_project_page(&title);
    push_date_period_navigation_html(&mut renderer, period);
    renderer.push_raw("<main class=\"maki-date-page\">");
    match period {
        DatePeriod::Year(year) => push_date_year_html(&mut renderer, date_index, year),
        DatePeriod::Month { year, month } => {
            push_date_month_html(&mut renderer, date_index, year, month)
        }
        DatePeriod::Week(week) => push_date_week_html(&mut renderer, date_index, week),
        DatePeriod::Day(date) => push_date_day_html(&mut renderer, date_index, date),
    }
    renderer.push_raw("</main></body></html>");
    renderer.into_html()
}

fn render_maki_template(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut source = template.to_string();

    for (placeholder, value) in replacements {
        source = source.replace(placeholder, value);
    }

    source
}

fn push_maki_link(source: &mut String, title: &str, href: &str) {
    source.push('[');
    push_maki_single_line(source, &direct_link_safe_title(title));
    source.push_str("](");
    push_maki_direct_link_target(source, href);
    source.push(')');
}

fn direct_link_safe_title(title: &str) -> String {
    let title = title.replace(['[', ']'], "");
    let title = title.trim();
    if title.is_empty() {
        "Link".to_string()
    } else if title.starts_with('^') {
        format!("Link {title}")
    } else {
        title.to_string()
    }
}

fn push_maki_direct_link_target(source: &mut String, target: &str) {
    for ch in target.chars() {
        match ch {
            '(' => source.push_str("%28"),
            ')' => source.push_str("%29"),
            '\\' => source.push_str("%5C"),
            '\r' | '\n' => source.push(' '),
            _ => source.push(ch),
        }
    }
}

fn render_project_maki_source(
    source: &str,
    asset_mode: AssetMode,
    site_title: Option<&str>,
    site_header: bool,
) -> String {
    render_maki_source_with_context(
        source,
        RenderContext::default()
            .with_asset_mode(asset_mode)
            .with_project_navigation()
            .with_site_title(site_title)
            .with_site_header(site_header),
    )
}

fn sitemap_page_body_source(entries: &[SitemapEntry]) -> String {
    let mut source = String::new();

    if entries.is_empty() {
        source.push_str("No notes.\n");
        return source;
    }

    for entry in entries {
        source.push_str("- ");
        push_maki_link(&mut source, entry.title(), entry.path());
        source.push(' ');
        source.push('`');
        push_maki_single_line(&mut source, entry.source_path());
        source.push('`');
        source.push('\n');
    }

    source
}

fn modified_time_kst_label(modified: Option<SystemTime>) -> String {
    let Some(modified) = modified else {
        return "unknown".to_string();
    };
    let Ok(duration) = modified.duration_since(UNIX_EPOCH) else {
        return "before 1970".to_string();
    };

    format_unix_seconds_kst(duration.as_secs())
}

pub(in crate::html) fn format_unix_seconds_kst(seconds: u64) -> String {
    const SECONDS_PER_DAY: u64 = 86_400;

    let local_seconds = seconds.saturating_add(KST_OFFSET_SECONDS);
    let days = (local_seconds / SECONDS_PER_DAY) as i64;
    let seconds_of_day = local_seconds % SECONDS_PER_DAY;
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let (year, month, day) = civil_from_unix_days(days);

    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} KST")
}

fn civil_from_unix_days(days: i64) -> (i32, u32, u32) {
    // Howard Hinnant's civil-from-days algorithm for proleptic Gregorian UTC dates.
    let shifted_days = days + 719_468;
    let era = if shifted_days >= 0 {
        shifted_days
    } else {
        shifted_days - 146_096
    } / 146_097;
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };

    (year as i32, month as u32, day as u32)
}

fn date_index_page_body_source(date_index: &DateIndex) -> String {
    let mut source = String::new();
    let mut year_counts = BTreeMap::new();
    for (date, _backlinks) in date_index.dates() {
        *year_counts.entry(date.year()).or_insert(0) += 1;
    }

    if year_counts.is_empty() {
        source.push_str("No date markers.\n");
        return source;
    }

    source.push_str("== Years\n\n");
    for (year, count) in &year_counts {
        push_maki_link_item_with_count(
            &mut source,
            &format!("{year:04}"),
            &maki::date_year_page_path(*year),
            *count,
        );
    }

    source
}

fn push_date_period_navigation_html(renderer: &mut Renderer<'_>, period: DatePeriod) {
    renderer.push_raw("<nav class=\"maki-date-navigation\" aria-label=\"Date navigation\">");
    let mut needs_space = false;
    if let Some(previous) = period.previous() {
        push_html_link(
            renderer,
            &previous.path(),
            &format!("← {}", date_period_link_label(previous)),
        );
        needs_space = true;
    }
    if needs_space {
        renderer.push_raw(" ");
    }
    push_html_link(
        renderer,
        &period.parent_path(),
        &format!("↑ {}", date_period_parent_label(period)),
    );
    if let DatePeriod::Day(date) = period
        && let Some(week) = iso_week_for_date(date)
    {
        renderer.push_raw(" ");
        push_html_link(
            renderer,
            &DatePeriod::Week(week).path(),
            &format!("↗ {}", DatePeriod::Week(week).title()),
        );
    }
    if let Some(next) = period.next() {
        renderer.push_raw(" ");
        push_html_link(
            renderer,
            &next.path(),
            &format!("{} →", date_period_link_label(next)),
        );
    }
    renderer.push_raw("</nav>");
}

fn date_period_title(period: DatePeriod) -> String {
    match period {
        DatePeriod::Year(_) => period.title(),
        DatePeriod::Month { .. } => format!("Month {}", period.title()),
        DatePeriod::Week(_) => format!("Week {}", period.title()),
        DatePeriod::Day(date) => format!("Date {}", date_label(date)),
    }
}

fn date_period_link_label(period: DatePeriod) -> String {
    match period {
        DatePeriod::Year(_) | DatePeriod::Month { .. } | DatePeriod::Week(_) => period.title(),
        DatePeriod::Day(date) => date_label(date),
    }
}

fn date_period_parent_label(period: DatePeriod) -> String {
    match period {
        DatePeriod::Year(_) => "Dates".to_string(),
        DatePeriod::Month { year, .. } => format!("{year:04}"),
        DatePeriod::Week(week) => format!("{:04}", week.year()),
        DatePeriod::Day(date) => format!("{:04}-{:02}", date.year(), date.month()),
    }
}

fn date_label(date: Date) -> String {
    format!("{date} {}", date.weekday_abbrev())
}

fn iso_week_for_date(date: Date) -> Option<IsoWeek> {
    let first_year = date.year().saturating_sub(1).max(1);
    let last_year = date.year().saturating_add(1).min(9999);

    for year in first_year..=last_year {
        for week_number in 1..=53 {
            let Some(week) = IsoWeek::new(year, week_number) else {
                break;
            };
            let (start, end) = week.representable_date_range();
            if start <= date && date <= end {
                return Some(week);
            }
        }
    }

    None
}

#[derive(Debug, Clone)]
struct DateLinkItem {
    label: String,
    href: String,
    count: usize,
}

fn push_date_year_html(renderer: &mut Renderer<'_>, date_index: &DateIndex, year: u16) {
    push_date_link_list_section(renderer, "Months", date_year_month_items(date_index, year));
    push_date_link_list_section(renderer, "Weeks", date_year_week_items(date_index, year));
}

fn push_date_month_html(renderer: &mut Renderer<'_>, date_index: &DateIndex, year: u16, month: u8) {
    let period = DatePeriod::Month { year, month };

    push_date_backlinks_section(
        renderer,
        "Backlinks",
        date_index,
        period_backlinks(date_index, period),
    );
    push_date_link_list_section(
        renderer,
        "Days",
        date_month_day_items(date_index, year, month),
    );
    push_date_link_list_section(
        renderer,
        "Weeks",
        date_month_week_items(date_index, year, month),
    );
}

fn push_date_week_html(renderer: &mut Renderer<'_>, date_index: &DateIndex, week: IsoWeek) {
    push_date_backlinks_section(
        renderer,
        "Backlinks",
        date_index,
        period_backlinks(date_index, DatePeriod::Week(week)),
    );
    push_date_link_list_section(renderer, "Days", date_week_day_items(date_index, week));
}

fn push_date_day_html(renderer: &mut Renderer<'_>, date_index: &DateIndex, date: Date) {
    push_date_backlinks_section(
        renderer,
        "Backlinks",
        date_index,
        direct_date_backlinks(date_index, date),
    );
    let containing_periods = date_containing_period_items(date_index, date);
    if !containing_periods.is_empty() {
        push_date_link_list_section(renderer, "Containing Periods", containing_periods);
    }
    let containing_ranges = date_range_backlinks(date_index, date);
    if !containing_ranges.is_empty() {
        push_date_backlinks_section(renderer, "Containing Ranges", date_index, containing_ranges);
    }
}

fn date_year_month_items(date_index: &DateIndex, year: u16) -> Vec<DateLinkItem> {
    let mut counts = BTreeMap::<u8, usize>::new();

    for (period, backlinks) in date_index.periods() {
        if let DatePeriod::Month {
            year: period_year,
            month,
        } = period
            && *period_year == year
        {
            *counts.entry(*month).or_insert(0) += backlinks.len();
        }
    }

    for (date, _backlinks) in date_index.dates() {
        if date.year() == year {
            let count = direct_date_backlink_count(date_index, *date);
            if count > 0 {
                *counts.entry(date.month()).or_insert(0) += count;
            }
        }
    }

    counts
        .into_iter()
        .map(|(month, count)| {
            let period = DatePeriod::Month { year, month };
            DateLinkItem {
                label: period.title(),
                href: period.path(),
                count,
            }
        })
        .collect()
}

fn date_year_week_items(date_index: &DateIndex, year: u16) -> Vec<DateLinkItem> {
    let mut counts = BTreeMap::<u8, usize>::new();

    for (period, backlinks) in date_index.periods() {
        if let DatePeriod::Week(week) = period
            && week.year() == year
        {
            *counts.entry(week.week()).or_insert(0) += backlinks.len();
        }
    }

    counts
        .into_iter()
        .map(|(week, count)| {
            let period =
                DatePeriod::Week(IsoWeek::new(year, week).expect("indexed ISO week is valid"));
            DateLinkItem {
                label: period.title(),
                href: period.path(),
                count,
            }
        })
        .collect()
}

fn date_month_day_items(date_index: &DateIndex, year: u16, month: u8) -> Vec<DateLinkItem> {
    date_index
        .dates()
        .filter_map(|(date, _backlinks)| {
            if date.year() != year || date.month() != month {
                return None;
            }

            let count = direct_date_backlink_count(date_index, *date);
            (count > 0).then(|| DateLinkItem {
                label: date_label(*date),
                href: maki::date_page_path(*date),
                count,
            })
        })
        .collect()
}

fn date_month_week_items(date_index: &DateIndex, year: u16, month: u8) -> Vec<DateLinkItem> {
    let Some(month) = DateMonth::new(year, month) else {
        return vec![];
    };
    let month_start = month.first_day();
    let month_end = month.last_day();

    date_index
        .periods()
        .filter_map(|(period, backlinks)| {
            let DatePeriod::Week(week) = period else {
                return None;
            };
            let (week_start, week_end) = week.representable_date_range();
            if week_end < month_start || week_start > month_end {
                return None;
            }

            Some(DateLinkItem {
                label: period.title(),
                href: period.path(),
                count: backlinks.len(),
            })
        })
        .collect()
}

fn date_week_day_items(date_index: &DateIndex, week: IsoWeek) -> Vec<DateLinkItem> {
    let (start, end) = week.representable_date_range();

    date_index
        .dates()
        .filter_map(|(date, _backlinks)| {
            if *date < start || *date > end {
                return None;
            }

            let count = direct_date_backlink_count(date_index, *date);
            (count > 0).then(|| DateLinkItem {
                label: date_label(*date),
                href: maki::date_page_path(*date),
                count,
            })
        })
        .collect()
}

fn date_containing_period_items(date_index: &DateIndex, date: Date) -> Vec<DateLinkItem> {
    let mut counts = BTreeMap::<DatePeriod, usize>::new();

    if let Some(backlinks) = date_index.backlinks_for(&date) {
        for backlink in backlinks {
            if let Some(period) = containing_period_for_backlink(date_index, backlink) {
                *counts.entry(period).or_insert(0) += 1;
            }
        }
    }

    counts
        .into_iter()
        .map(|(period, count)| DateLinkItem {
            label: period.title(),
            href: period.path(),
            count,
        })
        .collect()
}

fn containing_period_for_backlink(
    date_index: &DateIndex,
    backlink: &DateBacklink,
) -> Option<DatePeriod> {
    let occurrence = date_index.occurrence(backlink.occurrence_id())?;

    match (backlink.relation(), occurrence.marker()) {
        (
            DateRelation::MonthDay,
            DateMarker::Single {
                target: DateStampTarget::Month(month),
                ..
            },
        ) => Some(DatePeriod::Month {
            year: month.year(),
            month: month.month(),
        }),
        (
            DateRelation::WeekDay,
            DateMarker::Single {
                target: DateStampTarget::IsoWeek(week),
                ..
            },
        ) => Some(DatePeriod::Week(*week)),
        _ => None,
    }
}

fn period_backlinks(date_index: &DateIndex, period: DatePeriod) -> Vec<&DateBacklink> {
    date_index
        .backlinks_for_period(period)
        .map(|backlinks| backlinks.iter().collect())
        .unwrap_or_default()
}

fn direct_date_backlinks(date_index: &DateIndex, date: Date) -> Vec<&DateBacklink> {
    date_index
        .backlinks_for(&date)
        .map(|backlinks| {
            backlinks
                .iter()
                .filter(|backlink| is_direct_date_relation(backlink.relation()))
                .collect()
        })
        .unwrap_or_default()
}

fn date_range_backlinks(date_index: &DateIndex, date: Date) -> Vec<&DateBacklink> {
    date_index
        .backlinks_for(&date)
        .map(|backlinks| {
            backlinks
                .iter()
                .filter(|backlink| is_range_date_relation(backlink.relation()))
                .collect()
        })
        .unwrap_or_default()
}

fn direct_date_backlink_count(date_index: &DateIndex, date: Date) -> usize {
    direct_date_backlinks(date_index, date).len()
}

fn is_direct_date_relation(relation: DateRelation) -> bool {
    matches!(relation, DateRelation::Single)
}

fn is_range_date_relation(relation: DateRelation) -> bool {
    matches!(
        relation,
        DateRelation::Range
            | DateRelation::RangeStart
            | DateRelation::RangeMiddle
            | DateRelation::RangeEnd
    )
}

fn push_date_link_list_section(renderer: &mut Renderer<'_>, title: &str, items: Vec<DateLinkItem>) {
    push_html_heading(renderer, 3, title);
    if items.is_empty() {
        push_date_empty(renderer);
        return;
    }

    renderer.push_raw("<ul class=\"maki-date-list\">");
    for item in items {
        renderer.push_raw("<li>");
        push_html_link(renderer, &item.href, &item.label);
        renderer.push_raw("<span>");
        renderer.escape_html_into(&item.count.to_string());
        renderer.push_raw("</span></li>");
    }
    renderer.push_raw("</ul>");
}

fn push_date_backlinks_section(
    renderer: &mut Renderer<'_>,
    title: &str,
    date_index: &DateIndex,
    backlinks: Vec<&DateBacklink>,
) {
    push_html_heading(renderer, 3, title);

    let entries = backlinks
        .into_iter()
        .filter_map(|backlink| {
            date_index
                .occurrence(backlink.occurrence_id())
                .map(|occurrence| (occurrence, backlink.relation()))
        })
        .collect::<Vec<_>>();

    if entries.is_empty() {
        push_date_empty(renderer);
        return;
    }

    renderer.push_raw("<ul class=\"maki-date-backlinks\">");
    for (occurrence, relation) in entries {
        push_date_backlink_html(renderer, occurrence, relation);
    }
    renderer.push_raw("</ul>");
}

fn push_date_backlink_html(
    renderer: &mut Renderer<'_>,
    occurrence: &DateOccurrence,
    relation: DateRelation,
) {
    let target_href = format!("{}#{}", occurrence.note_ref().web_path(), occurrence.id());

    renderer.push_raw("<li class=\"maki-date-backlink");
    if let Some(class_name) = date_relation_class(relation) {
        renderer.push_raw(" ");
        renderer.push_raw(class_name);
    }
    renderer.push_raw("\"><div class=\"maki-date-backlink-main\">");
    push_html_link(renderer, &target_href, occurrence.note_title());
    renderer.push_raw("<span class=\"maki-date-backlink-source\">");
    push_date_labels_html(renderer, occurrence, relation);
    renderer.push_raw("</span></div>");

    if !occurrence.context().trim().is_empty() {
        renderer.push_raw("<pre class=\"maki-date-backlink-context\"><code>");
        renderer.escape_html_into(occurrence.context());
        renderer.push_raw("</code></pre>");
    }

    renderer.push_raw("</li>");
}

fn date_relation_class(relation: DateRelation) -> Option<&'static str> {
    match relation {
        DateRelation::Range => Some("maki-date-backlink-range"),
        DateRelation::RangeStart => Some("maki-date-backlink-range-start"),
        DateRelation::RangeMiddle => Some("maki-date-backlink-range-middle"),
        DateRelation::RangeEnd => Some("maki-date-backlink-range-end"),
        DateRelation::Single
        | DateRelation::Month
        | DateRelation::Week
        | DateRelation::MonthDay
        | DateRelation::WeekDay => None,
    }
}

fn push_date_labels_html(
    renderer: &mut Renderer<'_>,
    occurrence: &DateOccurrence,
    relation: DateRelation,
) {
    renderer.escape_html_into(date_marker_kind_label(occurrence.marker().kind()));
    renderer.push_raw(", ");
    renderer.escape_html_into(relation.label());
    renderer.push_raw(", ");
    match occurrence.origin() {
        DateOrigin::Inline => renderer.push_raw("inline"),
        DateOrigin::Property { key } => {
            renderer.push_raw("property:");
            renderer.escape_html_into(key);
        }
    }
}

fn push_html_heading(renderer: &mut Renderer<'_>, level: usize, title: &str) {
    renderer.push_raw("<h");
    renderer.push_raw(&level.to_string());
    renderer.push_raw(" id=\"");
    renderer.escape_html_attr_into(title);
    renderer.push_raw("\">");
    renderer.escape_html_into(title);
    renderer.push_raw("</h");
    renderer.push_raw(&level.to_string());
    renderer.push_raw(">");
}

fn push_html_link(renderer: &mut Renderer<'_>, href: &str, label: &str) {
    renderer.push_raw("<a href=\"");
    renderer.escape_html_attr_into(href);
    renderer.push_raw("\">");
    renderer.escape_html_into(label);
    renderer.push_raw("</a>");
}

fn push_date_empty(renderer: &mut Renderer<'_>) {
    renderer.push_raw("<p class=\"maki-date-empty\">No date markers.</p>");
}

fn push_maki_link_item_with_count(source: &mut String, title: &str, href: &str, count: usize) {
    source.push_str("- ");
    push_maki_link(source, title, href);
    source.push(' ');
    push_maki_inline_code(source, &count.to_string());
    source.push('\n');
}

fn push_maki_inline_code(source: &mut String, input: &str) {
    source.push('`');
    for ch in input.chars() {
        match ch {
            '\r' | '\n' => source.push(' '),
            '`' => source.push('\''),
            _ => source.push(ch),
        }
    }
    source.push('`');
}

pub fn render_not_found_page(
    path: &str,
    asset_mode: AssetMode,
    site_title: Option<&str>,
) -> String {
    render_not_found_page_with_site_header(path, asset_mode, site_title, false)
}

pub fn render_not_found_page_with_site_header(
    path: &str,
    asset_mode: AssetMode,
    site_title: Option<&str>,
    site_header: bool,
) -> String {
    let mut renderer = Renderer::new_with_context(
        RenderContext::default()
            .with_asset_mode(asset_mode)
            .with_site_title(site_title)
            .with_site_header(site_header),
    );
    renderer.begin_project_page("Not Found");
    renderer.push_raw("<main class=\"maki-not-found-page\">");
    renderer.push_raw("<p class=\"maki-not-found-summary\">No Maki note is available at <code>");
    renderer.escape_html_into(path);
    renderer.push_raw("</code>.</p>");
    renderer.push_raw("<nav class=\"maki-not-found-actions\" aria-label=\"Not found actions\"><a href=\"/\">Home</a><a href=\"/@/\">Meta</a><a href=\"/.maki/search\">Search</a></nav>");
    renderer.push_raw("</main></body></html>");
    renderer.into_html()
}

pub fn render_diagnostics_page(
    diagnostics: &[ProjectDiagnostic],
    total_notes: usize,
    asset_mode: AssetMode,
    site_title: Option<&str>,
    site_header: bool,
) -> String {
    let source = diagnostics_page_source(diagnostics, total_notes);

    render_project_maki_source(&source, asset_mode, site_title, site_header)
}

fn diagnostics_page_source(diagnostics: &[ProjectDiagnostic], total_notes: usize) -> String {
    let summary = ProjectDiagnosticSummary::from_diagnostics(diagnostics);
    let summary = format!(
        "{} issue(s) across {total_notes} note(s): {} duplicate id(s), {} unresolved reference(s), {} broken link(s), {} ambiguous link(s), {} broken external link(s), {} parser warning(s), {} read failure(s).",
        summary.total(),
        summary.duplicate_ids(),
        summary.unresolved_references(),
        summary.broken_links(),
        summary.ambiguous_links(),
        summary.broken_external_links(),
        summary.parse_warnings(),
        summary.read_failures()
    );

    let body = if diagnostics.is_empty() {
        "No diagnostics.".to_string()
    } else {
        let mut body = String::new();
        let mut by_source: BTreeMap<PathBuf, Vec<&ProjectDiagnostic>> = BTreeMap::new();
        for diagnostic in diagnostics {
            by_source
                .entry(diagnostic.source_path().to_path_buf())
                .or_default()
                .push(diagnostic);
        }

        for (source_path, diagnostics) in by_source {
            let source_href = format!("/{}", source_path.with_extension("").display());
            body.push_str("== ");
            push_maki_link(&mut body, &source_path.display().to_string(), &source_href);
            body.push_str("\n\n");

            for diagnostic in diagnostics {
                body.push_str("- ");
                push_diagnostic_item(&mut body, diagnostic);
                body.push('\n');
            }
            body.push('\n');
        }

        body.trim_end().to_string()
    };

    render_maki_template(
        DIAGNOSTICS_TEMPLATE,
        &[("{{summary}}", &summary), ("{{body}}", &body)],
    )
}

fn push_diagnostic_item(source: &mut String, diagnostic: &ProjectDiagnostic) {
    source.push_str(diagnostic.kind().label());
    source.push_str(": ");
    if let Some(line) = diagnostic.line() {
        source.push_str("line ");
        source.push_str(&line.to_string());
        source.push_str(": ");
    }

    match diagnostic.kind() {
        ProjectDiagnosticKind::ParseWarning { message } => {
            push_maki_single_line(source, message);
        }
        ProjectDiagnosticKind::DuplicateId { id } => {
            push_maki_single_line(source, id);
        }
        ProjectDiagnosticKind::UnresolvedReference { key } => {
            push_maki_single_line(source, key);
        }
        ProjectDiagnosticKind::BrokenLink { target } => {
            push_maki_single_line(source, target);
        }
        ProjectDiagnosticKind::AmbiguousLink { target } => {
            push_maki_single_line(source, target);
        }
        ProjectDiagnosticKind::BrokenExternalLink { target, reason } => {
            push_maki_single_line(source, target);
            source.push_str(" (");
            push_maki_single_line(source, reason);
            source.push(')');
        }
        ProjectDiagnosticKind::ReadFailed => {
            source.push_str("failed to read note");
        }
    }
}

fn push_maki_single_line(source: &mut String, input: &str) {
    for ch in input.chars() {
        match ch {
            '\r' | '\n' => source.push(' '),
            _ => source.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_maki_links_encode_direct_link_delimiters() {
        let mut source = String::new();
        push_maki_link(&mut source, "[Draft]", "/notes/(draft)\\copy)");

        assert_eq!(source, "[Draft](/notes/%28draft%29%5Ccopy%29)");
        let html = render_project_maki_source(&source, AssetMode::Inline, None, false);
        assert!(html.contains("<a href=\"/notes/%28draft%29%5Ccopy%29\">Draft</a>"));
    }

    #[test]
    fn generated_maki_links_avoid_reserved_or_empty_titles() {
        let mut source = String::new();
        push_maki_link(&mut source, " [^Draft] ", "/draft");
        source.push(' ');
        push_maki_link(&mut source, "[]", "/fallback");

        assert_eq!(source, "[Link ^Draft](/draft) [Link](/fallback)");
        let html = render_project_maki_source(&source, AssetMode::Inline, None, false);
        assert!(html.contains("<a href=\"/draft\">Link ^Draft</a>"));
        assert!(html.contains("<a href=\"/fallback\">Link</a>"));
    }
}
