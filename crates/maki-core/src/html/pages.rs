use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    maki::{
        self, DateIndex, DateOccurrence, DateOrigin, DatePeriod, DateRelation, ProjectDiagnostic,
        ProjectDiagnosticKind, ProjectDiagnosticSummary, RecentEntry, SearchEntry,
    },
    parser::Date,
};

use super::{
    context::{AssetMode, RenderContext},
    date_markup::date_marker_kind_label,
    render_maki_source_with_context,
    renderer::Renderer,
};

const META_TEMPLATE: &str = include_str!("../../../../templates/meta.maki");
const RECENTS_TEMPLATE: &str = include_str!("../../../../templates/recents.maki");
const DATES_INDEX_TEMPLATE: &str = include_str!("../../../../templates/dates-index.maki");
const DATE_PERIOD_TEMPLATE: &str = include_str!("../../../../templates/date-period.maki");
const DIAGNOSTICS_TEMPLATE: &str = include_str!("../../../../templates/diagnostics.maki");
const KST_OFFSET_SECONDS: u64 = 9 * 60 * 60;

pub fn render_search_page(
    query: &str,
    results: &[SearchEntry],
    total_entries: usize,
    asset_mode: AssetMode,
) -> String {
    let mut renderer = Renderer::new_with_asset_mode(asset_mode);
    renderer.begin_project_page("Search");
    renderer.push_raw("<main class=\"maki-search-page\">");
    renderer.push_raw("<p class=\"maki-search-summary\">");
    if query.trim().is_empty() {
        renderer.push_raw(&format!("Showing {total_entries} titles."));
    } else {
        renderer.push_raw(&format!("{} matches for ", results.len()));
        renderer.push_raw("<code>");
        renderer.escape_html_into(query);
        renderer.push_raw("</code>.");
    }
    renderer.push_raw("</p>");

    if results.is_empty() {
        renderer.push_raw("<p class=\"maki-search-empty\">No matching titles.</p>");
    } else {
        renderer.push_raw("<ul class=\"maki-search-page-results\">");
        for entry in results {
            renderer.push_raw("<li><a href=\"");
            renderer.escape_html_attr_into(entry.path());
            renderer.push_raw("\">");
            renderer.escape_html_into(entry.title());
            renderer.push_raw("</a><span>");
            renderer.escape_html_into(entry.source_path());
            renderer.push_raw("</span></li>");
        }
        renderer.push_raw("</ul>");
    }

    renderer.push_raw("</main></body></html>");
    renderer.into_html()
}

pub fn render_meta_index_page(asset_mode: AssetMode) -> String {
    render_project_maki_source(META_TEMPLATE, asset_mode)
}

pub fn render_recents_page(entries: &[RecentEntry], asset_mode: AssetMode) -> String {
    let body = recents_page_body_source(entries);
    let source = render_maki_template(RECENTS_TEMPLATE, &[("{{body}}", &body)]);

    render_project_maki_source(&source, asset_mode)
}

pub fn render_date_index_page(date_index: &DateIndex, asset_mode: AssetMode) -> String {
    let body = date_index_page_body_source(date_index);
    let source = render_maki_template(DATES_INDEX_TEMPLATE, &[("{{body}}", &body)]);

    render_project_maki_source(&source, asset_mode)
}

pub fn render_date_period_page(
    period: DatePeriod,
    date_index: &DateIndex,
    asset_mode: AssetMode,
) -> String {
    let source = date_period_page_source(period, date_index);

    render_project_maki_source(&source, asset_mode)
}

fn render_maki_template(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut source = template.to_string();

    for (placeholder, value) in replacements {
        source = source.replace(placeholder, value);
    }

    source
}

fn render_project_maki_source(source: &str, asset_mode: AssetMode) -> String {
    render_maki_source_with_context(
        source,
        RenderContext::default()
            .with_asset_mode(asset_mode)
            .with_project_navigation(),
    )
}

fn recents_page_body_source(entries: &[RecentEntry]) -> String {
    let mut source = String::new();

    if entries.is_empty() {
        source.push_str("No notes.\n");
        return source;
    }

    for entry in entries {
        source.push_str("- ");
        let modified = modified_time_kst_label(entry.modified());
        push_maki_single_line(&mut source, &modified);
        source.push(' ');
        push_maki_closed_link(&mut source, entry.title(), entry.path());
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
    for (year, count) in year_counts.iter().rev() {
        push_maki_link_item_with_count(
            &mut source,
            &format!("{year:04}"),
            &maki::date_year_page_path(*year),
            *count,
        );
    }

    source
}

fn date_period_page_source(period: DatePeriod, date_index: &DateIndex) -> String {
    let navigation = date_period_navigation_source(period);
    let body = date_period_page_body_source(period, date_index);
    let title = date_period_title(period);
    render_maki_template(
        DATE_PERIOD_TEMPLATE,
        &[
            ("{{title}}", &title),
            ("{{navigation}}", &navigation),
            ("{{body}}", &body),
        ],
    )
}

fn date_period_page_body_source(period: DatePeriod, date_index: &DateIndex) -> String {
    let mut source = String::new();
    match period {
        DatePeriod::Year(year) => push_date_year_source(&mut source, date_index, year),
        DatePeriod::Month { year, month } => {
            push_date_month_source(&mut source, date_index, year, month)
        }
        DatePeriod::Day(date) => push_date_day_source(&mut source, date_index, date),
    }

    source
}

fn date_period_navigation_source(period: DatePeriod) -> String {
    let mut source = String::new();

    if let Some(previous) = period.previous() {
        push_maki_closed_link(
            &mut source,
            &format!("← {}", date_period_navigation_label(previous)),
            &previous.path(),
        );
        source.push(' ');
    }
    push_maki_closed_link(
        &mut source,
        &format!("↑ {}", date_period_parent_label(period)),
        &period.parent_path(),
    );
    if let Some(next) = period.next() {
        source.push(' ');
        push_maki_closed_link(
            &mut source,
            &format!("{} →", date_period_navigation_label(next)),
            &next.path(),
        );
    }

    source
}

fn date_period_title(period: DatePeriod) -> String {
    date_period_navigation_label(period)
}

fn date_period_navigation_label(period: DatePeriod) -> String {
    match period {
        DatePeriod::Year(_) | DatePeriod::Month { .. } => period.title(),
        DatePeriod::Day(date) => date_label(date),
    }
}

fn date_period_parent_label(period: DatePeriod) -> String {
    match period {
        DatePeriod::Year(_) => "Dates".to_string(),
        DatePeriod::Month { year, .. } => format!("{year:04}"),
        DatePeriod::Day(date) => format!("{:04}-{:02}", date.year(), date.month()),
    }
}

fn date_label(date: Date) -> String {
    format!("{date} {}", date.weekday_abbrev())
}

fn push_date_year_source(source: &mut String, date_index: &DateIndex, year: u16) {
    let mut month_counts = BTreeMap::new();
    for (date, _backlinks) in date_index.dates() {
        if date.year() == year {
            *month_counts.entry(date.month()).or_insert(0) += 1;
        }
    }

    source.push_str("== Months\n\n");
    if month_counts.is_empty() {
        source.push_str("No date markers.\n");
        return;
    }

    for (month, count) in month_counts.iter().rev() {
        let period = DatePeriod::Month {
            year,
            month: *month,
        };
        push_maki_link_item_with_count(source, &period.title(), &period.path(), *count);
    }
}

fn push_date_month_source(source: &mut String, date_index: &DateIndex, year: u16, month: u8) {
    let dates = date_index
        .dates()
        .filter(|(date, _backlinks)| date.year() == year && date.month() == month)
        .map(|(date, _backlinks)| *date)
        .collect::<Vec<_>>();

    source.push_str("== Days\n\n");
    if dates.is_empty() {
        source.push_str("No date markers.\n");
        return;
    }

    for date in dates.iter().rev() {
        source.push_str("=== ");
        push_maki_closed_link(source, &date_label(*date), &maki::date_page_path(*date));
        source.push_str("\n\n");
        if !push_date_backlinks_for_date(source, date_index, *date) {
            source.push_str("No date markers.\n");
        }
        source.push('\n');
    }
}

fn push_date_day_source(source: &mut String, date_index: &DateIndex, date: Date) {
    source.push_str("== Backlinks\n\n");

    if !push_date_backlinks_for_date(source, date_index, date) {
        source.push_str("No date markers.\n");
    }
}

fn push_date_backlinks_for_date(source: &mut String, date_index: &DateIndex, date: Date) -> bool {
    let Some(backlinks) = date_index.backlinks_for(&date) else {
        return false;
    };

    let mut has_backlinks = false;
    for backlink in backlinks {
        let Some(occurrence) = date_index.occurrence(backlink.occurrence_id()) else {
            continue;
        };
        has_backlinks = true;
        push_date_backlink_source(source, occurrence, backlink.relation());
    }

    has_backlinks
}

fn push_date_backlink_source(
    source: &mut String,
    occurrence: &DateOccurrence,
    relation: DateRelation,
) {
    let target_href = format!("{}#{}", occurrence.note_ref().web_path(), occurrence.id());

    source.push_str("- ");
    push_maki_closed_link(source, occurrence.note_title(), &target_href);
    source.push(' ');
    push_date_labels(source, occurrence, relation);
    source.push('\n');

    if !occurrence.context().trim().is_empty() {
        push_indented_maki_code_block(source, occurrence.context(), "  ");
    }
}

fn push_date_labels(source: &mut String, occurrence: &DateOccurrence, relation: DateRelation) {
    push_maki_single_line(source, date_marker_kind_label(occurrence.marker().kind()));
    source.push_str(", ");
    push_maki_single_line(source, relation.label());
    source.push_str(", ");
    match occurrence.origin() {
        DateOrigin::Inline => source.push_str("inline"),
        DateOrigin::Property { key } => {
            source.push_str("property:");
            push_maki_single_line(source, key);
        }
    }
}

fn push_maki_link_item_with_count(source: &mut String, title: &str, href: &str, count: usize) {
    source.push_str("- ");
    push_maki_closed_link(source, title, href);
    source.push(' ');
    push_maki_inline_code(source, &count.to_string());
    source.push('\n');
}

fn push_maki_closed_link(source: &mut String, title: &str, href: &str) {
    push_maki_link(source, title, href);
    source.push(')');
}

fn push_maki_link(source: &mut String, title: &str, href: &str) {
    source.push('[');
    push_maki_single_line(source, title);
    source.push_str("](");
    push_maki_single_line(source, href);
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

fn push_indented_maki_code_block(source: &mut String, input: &str, indent: &str) {
    for line in input.lines() {
        source.push_str(indent);
        source.push(':');
        if !line.is_empty() {
            source.push(' ');
            source.push_str(line);
        }
        source.push('\n');
    }
}

pub fn render_not_found_page(path: &str, asset_mode: AssetMode) -> String {
    let mut renderer = Renderer::new_with_asset_mode(asset_mode);
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
) -> String {
    let source = diagnostics_page_source(diagnostics, total_notes);

    render_project_maki_source(&source, asset_mode)
}

fn diagnostics_page_source(diagnostics: &[ProjectDiagnostic], total_notes: usize) -> String {
    let summary = ProjectDiagnosticSummary::from_diagnostics(diagnostics);
    let summary = format!(
        "{} issue(s) across {total_notes} note(s): {} broken link(s), {} ambiguous link(s), {} broken external link(s), {} parser warning(s), {} read failure(s).",
        summary.total(),
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
            body.push_str("== [");
            push_maki_single_line(&mut body, &source_path.display().to_string());
            body.push_str("](");
            push_maki_single_line(&mut body, &source_href);
            body.push_str(")\n\n");

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
