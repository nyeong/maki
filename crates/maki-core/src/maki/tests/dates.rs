use super::*;
use crate::parser::IsoWeek;

#[test]
fn date_index_collects_inline_property_and_range_dates() {
    let project = temp_project("date-index");
    write_note_with_content(
        &project,
        "start.maki",
        r#"--^ title: Start
--^ date: [2026-08-15]

Meet <2026-08-16 토>.

Track [2026-08-17]--[2026-08-19]."#,
    );

    let maki = Maki::load(&project.root).unwrap();
    let property_date = Date::parse("2026-08-15").unwrap();
    let range_start_date = Date::parse("2026-08-17").unwrap();
    let middle_date = Date::parse("2026-08-18").unwrap();
    let range_end_date = Date::parse("2026-08-19").unwrap();

    let index_dates = maki
        .date_index()
        .dates()
        .map(|(date, _backlinks)| *date)
        .collect::<Vec<_>>();
    assert!(index_dates.contains(&property_date));
    assert!(index_dates.contains(&range_start_date));
    assert!(!index_dates.contains(&middle_date));
    assert!(index_dates.contains(&range_end_date));

    let property_backlinks = maki.date_index().backlinks_for(&property_date).unwrap();
    assert_eq!(property_backlinks.len(), 1);
    let property_occurrence = maki
        .date_index()
        .occurrence(property_backlinks[0].occurrence_id())
        .unwrap();
    assert!(matches!(
        property_occurrence.origin(),
        DateOrigin::Property { key } if key == "date"
    ));
    assert_eq!(property_occurrence.marker().raw(), "[2026-08-15]");

    let middle_backlinks = maki.date_index().backlinks_for(&middle_date).unwrap();
    assert_eq!(middle_backlinks.len(), 1);
    assert_eq!(middle_backlinks[0].relation(), DateRelation::RangeMiddle);
    let middle_occurrence = maki
        .date_index()
        .occurrence(middle_backlinks[0].occurrence_id())
        .unwrap();
    assert_eq!(
        middle_occurrence.marker().raw(),
        "[2026-08-17]--[2026-08-19]"
    );

    let html = maki.render_html(Path::new("start.maki")).unwrap();
    assert!(html.contains("href=\"/@/dates/2026-08-16#date-inline-start-maki-1\""));
    assert!(html.contains("href=\"/@/dates/2026-08-17#date-inline-start-maki-2\""));
    assert!(html.contains("href=\"/@/dates/2026-08-19#date-inline-start-maki-2\""));
}

#[test]
fn date_occurrence_kind_labels_inline_and_property_dates() {
    let project = temp_project("date-occurrence-kind");
    write_note_with_content(
        &project,
        "start.maki",
        r#"--^ scheduled: <2026-08-26>
--^ deadline: [2026-08-27]
--^ date: [2026-08-28]

Reference [2026-08-29].
Event <2026-08-30>."#,
    );

    let maki = Maki::load(&project.root).unwrap();
    let kind_for = |raw: &str| {
        let date = Date::parse(raw).unwrap();
        let backlinks = maki.date_index().backlinks_for(&date).unwrap();
        maki.date_index()
            .occurrence(backlinks[0].occurrence_id())
            .unwrap()
            .kind()
    };

    assert_eq!(kind_for("2026-08-26"), DateOccurrenceKind::Scheduled);
    assert_eq!(kind_for("2026-08-27"), DateOccurrenceKind::Deadline);
    assert_eq!(kind_for("2026-08-28"), DateOccurrenceKind::Metadata);
    assert_eq!(kind_for("2026-08-29"), DateOccurrenceKind::Reference);
    assert_eq!(kind_for("2026-08-30"), DateOccurrenceKind::Event);
    assert_eq!(DateOccurrenceKind::Scheduled.as_str(), "scheduled");
}

#[test]
fn date_index_collects_month_week_and_iso_weekday_markers() {
    let project = temp_project("period-date-index");
    write_note_with_content(
        &project,
        "start.maki",
        r#"--^ title: Start

Month [2026-08].
Week [2026-W23].
Specific [2026-W23-1].
Target [2026-06-01]."#,
    );

    let maki = Maki::load(&project.root).unwrap();
    let month_period = DatePeriod::Month {
        year: 2026,
        month: 8,
    };
    let week = IsoWeek::new(2026, 23).unwrap();
    let week_period = DatePeriod::Week(week);
    let august_first = Date::parse("2026-08-01").unwrap();
    let week_monday = Date::parse("2026-06-01").unwrap();
    let week_tuesday = Date::parse("2026-06-02").unwrap();

    let month_backlinks = maki
        .date_index()
        .backlinks_for_period(month_period)
        .unwrap();
    assert_eq!(month_backlinks.len(), 1);
    assert_eq!(month_backlinks[0].relation(), DateRelation::Month);

    let week_backlinks = maki.date_index().backlinks_for_period(week_period).unwrap();
    assert_eq!(week_backlinks.len(), 1);
    assert_eq!(week_backlinks[0].relation(), DateRelation::Week);

    let august_backlinks = maki.date_index().backlinks_for(&august_first).unwrap();
    assert_eq!(august_backlinks.len(), 1);
    assert_eq!(august_backlinks[0].relation(), DateRelation::MonthDay);

    let monday_backlinks = maki.date_index().backlinks_for(&week_monday).unwrap();
    assert_eq!(monday_backlinks.len(), 3);
    assert_eq!(monday_backlinks[0].relation(), DateRelation::Single);
    assert_eq!(monday_backlinks[1].relation(), DateRelation::Single);
    assert_eq!(monday_backlinks[2].relation(), DateRelation::WeekDay);

    let tuesday_backlinks = maki.date_index().backlinks_for(&week_tuesday).unwrap();
    assert_eq!(tuesday_backlinks.len(), 1);
    assert_eq!(tuesday_backlinks[0].relation(), DateRelation::WeekDay);

    let indexed_dates = maki
        .date_index()
        .dates()
        .map(|(date, _backlinks)| *date)
        .collect::<Vec<_>>();
    assert!(indexed_dates.contains(&august_first));
    assert!(indexed_dates.contains(&week_tuesday));

    let html = maki.render_html(Path::new("start.maki")).unwrap();
    assert!(html.contains("href=\"/@/dates/2026-08#date-inline-start-maki-1\""));
    assert!(html.contains("href=\"/@/dates/2026-W23#date-inline-start-maki-2\""));
    assert!(html.contains("href=\"/@/dates/2026-06-01#date-inline-start-maki-3\""));
}

#[test]
fn date_index_collects_dates_inside_strong_inline() {
    let project = temp_project("strong-date-index");
    write_note_with_content(
        &project,
        "start.maki",
        r#"--^ title: Start
--^ due: *[2026-08-20]*

Plan *<2026-08-21> and [2026-08-22]--[2026-08-23]*."#,
    );

    let maki = Maki::load(&project.root).unwrap();
    let property_date = Date::parse("2026-08-20").unwrap();
    let inline_date = Date::parse("2026-08-21").unwrap();
    let range_start_date = Date::parse("2026-08-22").unwrap();
    let range_end_date = Date::parse("2026-08-23").unwrap();

    let property_backlinks = maki.date_index().backlinks_for(&property_date).unwrap();
    assert_eq!(property_backlinks.len(), 1);
    let property_occurrence = maki
        .date_index()
        .occurrence(property_backlinks[0].occurrence_id())
        .unwrap();
    assert!(matches!(
        property_occurrence.origin(),
        DateOrigin::Property { key } if key == "due"
    ));
    assert_eq!(property_occurrence.marker().raw(), "[2026-08-20]");

    let inline_backlinks = maki.date_index().backlinks_for(&inline_date).unwrap();
    assert_eq!(inline_backlinks.len(), 1);
    let inline_occurrence = maki
        .date_index()
        .occurrence(inline_backlinks[0].occurrence_id())
        .unwrap();
    assert_eq!(
        inline_occurrence.context(),
        "Plan *<2026-08-21> and [2026-08-22]--[2026-08-23]*."
    );

    let range_start_backlinks = maki.date_index().backlinks_for(&range_start_date).unwrap();
    let range_end_backlinks = maki.date_index().backlinks_for(&range_end_date).unwrap();
    assert_eq!(range_start_backlinks.len(), 1);
    assert_eq!(range_end_backlinks.len(), 1);

    let html = maki.render_html(Path::new("start.maki")).unwrap();
    assert!(html.contains("<strong>"));
    assert!(html.contains("id=\"date-inline-start-maki-1\""));
    assert!(html.contains("href=\"/@/dates/2026-08-21#date-inline-start-maki-1\""));
    assert!(html.contains("href=\"/@/dates/2026-08-22#date-inline-start-maki-2\""));
    assert!(html.contains("href=\"/@/dates/2026-08-23#date-inline-start-maki-2\""));
}

#[test]
fn date_context_preserves_braced_changes_and_highlight_syntax() {
    let project = temp_project("braced-change-date-context");
    write_note_with_content(
        &project,
        "start.maki",
        "Plan +{new} -{old} ::now:: [2026-08-24].",
    );

    let maki = Maki::load(&project.root).unwrap();
    let date = Date::parse("2026-08-24").unwrap();
    let backlinks = maki.date_index().backlinks_for(&date).unwrap();
    let occurrence = maki
        .date_index()
        .occurrence(backlinks[0].occurrence_id())
        .unwrap();

    assert_eq!(
        occurrence.context(),
        "Plan +{new} -{old} ::now:: [2026-08-24]."
    );
}

#[test]
fn date_index_collects_dates_from_footnote_definitions() {
    let project = temp_project("footnote-date-index");
    write_note_with_content(
        &project,
        "start.maki",
        r#"--^ title: Start

Body[^release].

[^release]: Released on [2026-08-24]."#,
    );

    let maki = Maki::load(&project.root).unwrap();
    let date = Date::parse("2026-08-24").unwrap();
    let backlinks = maki.date_index().backlinks_for(&date).unwrap();
    let occurrence = maki
        .date_index()
        .occurrence(backlinks[0].occurrence_id())
        .unwrap();

    assert_eq!(
        occurrence.context(),
        "[^release]: Released on [2026-08-24]."
    );
    let html = maki.render_html(Path::new("start.maki")).unwrap();
    assert!(html.contains("id=\"date-inline-start-maki-1\""));
    assert!(html.contains("<section class=\"footnotes\">"));
}

#[test]
fn date_index_ignores_dates_inside_raw_quotes() {
    let project = temp_project("raw-quote-date-index");
    write_note_with_content(
        &project,
        "start.maki",
        r#"--v mode: pre
> [2026-08-24]

--v mode: text
---quote
[2026-08-25]
---"#,
    );

    let maki = Maki::load(&project.root).unwrap();

    assert!(
        maki.date_index()
            .backlinks_for(&Date::parse("2026-08-24").unwrap())
            .is_none()
    );
    assert!(
        maki.date_index()
            .backlinks_for(&Date::parse("2026-08-25").unwrap())
            .is_none()
    );
}

#[test]
fn date_index_orders_range_middle_backlinks_after_direct_dates() {
    let project = temp_project("date-index-priority");
    write_note_with_content(
        &project,
        "start.maki",
        r#"--^ title: Start

Track [2026-08-17]--[2026-08-19].

Target [2026-08-18]."#,
    );

    let maki = Maki::load(&project.root).unwrap();
    let middle_date = Date::parse("2026-08-18").unwrap();

    let middle_backlinks = maki.date_index().backlinks_for(&middle_date).unwrap();
    assert_eq!(middle_backlinks.len(), 2);
    assert_eq!(middle_backlinks[0].relation(), DateRelation::Single);
    assert_eq!(middle_backlinks[1].relation(), DateRelation::RangeMiddle);

    let index_backlinks = maki
        .date_index()
        .dates()
        .find_map(|(date, backlinks)| (*date == middle_date).then_some(backlinks))
        .unwrap();
    assert_eq!(index_backlinks.len(), 1);
    assert_eq!(index_backlinks[0].relation(), DateRelation::Single);
}

#[test]
fn date_index_context_includes_parent_heading_and_top_list_item() {
    let project = temp_project("date-index-context");
    write_note_with_content(
        &project,
        "start.maki",
        r#"--^ title: Start

= Roadmap

- Decide timing
  - still thinking
  - [2026-08-15] done

== Sprint [2026-08-16]"#,
    );

    let maki = Maki::load(&project.root).unwrap();
    let nested_date = Date::parse("2026-08-15").unwrap();
    let heading_date = Date::parse("2026-08-16").unwrap();

    let nested_backlinks = maki.date_index().backlinks_for(&nested_date).unwrap();
    let nested_occurrence = maki
        .date_index()
        .occurrence(nested_backlinks[0].occurrence_id())
        .unwrap();
    assert_eq!(
        nested_occurrence.context(),
        "= Roadmap\n- Decide timing\n  - [2026-08-15] done"
    );

    let heading_backlinks = maki.date_index().backlinks_for(&heading_date).unwrap();
    let heading_occurrence = maki
        .date_index()
        .occurrence(heading_backlinks[0].occurrence_id())
        .unwrap();
    assert_eq!(
        heading_occurrence.context(),
        "= Roadmap\n== Sprint [2026-08-16]"
    );
}

#[test]
fn date_index_context_preserves_todo_state() {
    let project = temp_project("todo-date-context");
    write_note_with_content(
        &project,
        "start.maki",
        r#"- [ ] Release on [2026-08-25]
- [x] Reviewed on [2026-08-24]"#,
    );

    let maki = Maki::load(&project.root).unwrap();
    let todo = Date::parse("2026-08-25").unwrap();
    let done = Date::parse("2026-08-24").unwrap();
    let todo_backlinks = maki.date_index().backlinks_for(&todo).unwrap();
    let done_backlinks = maki.date_index().backlinks_for(&done).unwrap();

    assert_eq!(
        maki.date_index()
            .occurrence(todo_backlinks[0].occurrence_id())
            .unwrap()
            .context(),
        "- [ ] Release on [2026-08-25]"
    );
    assert_eq!(
        maki.date_index()
            .occurrence(done_backlinks[0].occurrence_id())
            .unwrap()
            .context(),
        "- [x] Reviewed on [2026-08-24]"
    );
}

#[test]
fn date_index_context_for_table_dates_includes_heading_row_and_table_header() {
    let project = temp_project("date-index-table-context");
    write_note_with_content(
        &project,
        "start.maki",
        r#"--^ title: Start

= Releases

| Date | Summary | Owner |
|---+---+---|
| [2026-08-15] | Ship alpha | Nyeong |
| [2026-08-16] | Follow up | Codex |"#,
    );

    let maki = Maki::load(&project.root).unwrap();
    let date = Date::parse("2026-08-15").unwrap();

    let backlinks = maki.date_index().backlinks_for(&date).unwrap();
    let occurrence = maki
        .date_index()
        .occurrence(backlinks[0].occurrence_id())
        .unwrap();

    assert_eq!(
        occurrence.context(),
        "= Releases\n| Date | Summary | Owner |\n| [2026-08-15] | Ship alpha | Nyeong |"
    );
    assert!(!occurrence.context().contains("Follow up"));
}
