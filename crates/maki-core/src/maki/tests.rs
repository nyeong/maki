use super::links::ExternalLinkCheck;
use super::note::{NoteMetadataEntry, collect_recent_entries};
use super::*;
use crate::parser::Date;
use std::path::{Path, PathBuf};
use std::{
    cell::RefCell,
    fs,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct TestProject {
    root: PathBuf,
}

impl Drop for TestProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn temp_project(name: &str) -> TestProject {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("maki-{name}-{}-{nanos}", std::process::id()));

    fs::create_dir_all(&root).unwrap();

    TestProject { root }
}

fn repo_path(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn write_note_with_content(project: &TestProject, path: &str, content: &str) {
    let path = project.root.join(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn write_note(project: &TestProject, path: &str) {
    write_note_with_content(project, path, "");
}

#[test]
fn project_config_can_set_source_directory() {
    let project = temp_project("project-source");
    fs::write(
        project.root.join(PROJECT_FILE_NAME),
        "[project]\ntitle = \"Source Fixture\"\nsource = \"docs\"\nhome = \"index\"\n",
    )
    .unwrap();

    let config = MakiConfig::load_project(&project.root).unwrap();

    assert_eq!(
        config.project_source_root(&project.root),
        project.root.join("docs")
    );
    assert_eq!(
        config.home_mode(),
        &HomeMode::Redirect("/index".to_string())
    );
}

#[test]
fn project_config_rejects_source_outside_project() {
    let project = temp_project("project-source-invalid");
    fs::write(
        project.root.join(PROJECT_FILE_NAME),
        "[project]\nsource = \"../docs\"\n",
    )
    .unwrap();

    assert!(matches!(
        MakiConfig::load_project(&project.root),
        Err(Error::InvalidProjectFile(_, message))
            if message == "project.source must be a relative path inside the project"
    ));
}

#[test]
fn note_path() {
    let note = Note::load(repo_path("."), "docs/use-cases.maki").unwrap();

    assert_eq!(note.source_path(), PathBuf::from("docs/use-cases.maki"));
    assert_eq!(note.canonical_path(), PathBuf::from("docs/use-cases"));
    assert_eq!(note.file_stem(), "use-cases");
    assert_eq!(note.note_ref().web_path(), "/docs/use-cases");
}

#[test]
fn note_ref() {
    let note = Note::load(repo_path("."), "docs/use-cases.maki").unwrap();
    let ref_ = note.note_ref();
    assert_eq!(ref_.canonical_path(), PathBuf::from("docs/use-cases"));
    assert_eq!(ref_.web_path(), "/docs/use-cases");
}

#[test]
fn resolve_note_link() {
    let maki = Maki::load(repo_path("docs")).unwrap();
    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("index"), "use-cases"),
        NoteLinkResolution::Found(NoteRef::new("use-cases"))
    );

    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("index"), "v0"),
        NoteLinkResolution::Found(NoteRef::new("milestones/v0"))
    );

    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("index"), "milestones/v0"),
        NoteLinkResolution::Found(NoteRef::new("milestones/v0"))
    );
}

#[test]
fn resolve_note_link_uses_case_insensitive_path_lookup() {
    let project = temp_project("case-insensitive-path");
    write_note(&project, "milestones/v0.maki");
    write_note(&project, "index.maki");

    let maki = Maki::load(&project.root).unwrap();

    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("index"), "Milestones/V0"),
        NoteLinkResolution::Found(NoteRef::new("milestones/v0"))
    );
}

#[test]
fn resolve_note_link_uses_case_insensitive_sibling_stem_lookup() {
    let project = temp_project("case-insensitive-sibling");
    write_note(&project, "notes/devenv.maki");
    write_note(&project, "notes/nix.maki");

    let maki = Maki::load(&project.root).unwrap();

    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("notes/devenv"), "Nix"),
        NoteLinkResolution::Found(NoteRef::new("notes/nix"))
    );
}

#[test]
fn resolve_note_link_prefers_sibling_stem_before_project_wide_stem() {
    let project = temp_project("sibling-before-project-stem");
    write_note(&project, "notes/page.maki");
    write_note(&project, "notes/nix.maki");
    write_note(&project, "other/Nix.maki");

    let maki = Maki::load(&project.root).unwrap();

    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("notes/page"), "NIX"),
        NoteLinkResolution::Found(NoteRef::new("notes/nix"))
    );
}

#[test]
fn resolve_note_link_reports_case_insensitive_stem_ambiguity() {
    let project = temp_project("case-insensitive-stem-ambiguity");
    write_note(&project, "start.maki");
    write_note(&project, "alpha/nix.maki");
    write_note(&project, "beta/NIX.maki");

    let maki = Maki::load(&project.root).unwrap();

    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("start"), "Nix"),
        NoteLinkResolution::Ambiguous
    );
}

#[test]
fn resolve_note_link_preserves_exact_path_priority() {
    let project = temp_project("exact-before-sibling");
    write_note(&project, "nix.maki");
    write_note(&project, "notes/page.maki");
    write_note(&project, "notes/nix.maki");

    let maki = Maki::load(&project.root).unwrap();

    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("notes/page"), "nix"),
        NoteLinkResolution::Found(NoteRef::new("nix"))
    );
}

#[test]
fn markdown_style_links_can_resolve_to_notes_with_custom_titles() {
    let project = temp_project("markdown-style-note-link");
    write_note_with_content(&project, "start.maki", "See [the page](page).");
    write_note_with_content(&project, "page.maki", "--^ title: Page\n\nbody");

    let maki = Maki::load(&project.root).unwrap();
    let html = maki.render_html(Path::new("start.maki")).unwrap();

    assert!(html.contains("<a href=\"/page\">the page</a>"));
}

#[test]
fn markdown_style_external_links_render_as_plain_hrefs() {
    let project = temp_project("markdown-style-external-link");
    write_note_with_content(
        &project,
        "start.maki",
        "See [djot](https://github.com/jgm/djot).",
    );

    let maki = Maki::load(&project.root).unwrap();
    let html = maki.render_html(Path::new("start.maki")).unwrap();

    assert!(
        html.contains("<a class=\"external-link\" href=\"https://github.com/jgm/djot\">djot</a>")
    );
}

#[test]
fn plain_external_urls_render_as_links() {
    let project = temp_project("plain-external-link");
    write_note_with_content(&project, "start.maki", "See https://example.com/docs.");

    let maki = Maki::load(&project.root).unwrap();
    let html = maki.render_html(Path::new("start.maki")).unwrap();

    assert!(html.contains(
        "<a class=\"external-link\" href=\"https://example.com/docs\">https://example.com/docs</a>."
    ));
}

#[test]
fn diagnostics_collect_parse_warnings_and_link_resolution_issues() {
    let project = temp_project("diagnostics");
    write_note_with_content(
        &project,
        "start.maki",
        r#"--^ invalid-property

See [[missing]], [Ghost](ghost), and [[same]].

> See [[quoted-missing]].

--- quote
See [[container-missing]].
---"#,
    );
    write_note(&project, "alpha/same.maki");
    write_note(&project, "beta/same.maki");

    let maki = Maki::load(&project.root).unwrap();
    let diagnostics = maki.diagnostics();

    assert!(diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.kind(),
            ProjectDiagnosticKind::ParseWarning { message }
                if message == "invalid property: --^ invalid-property"
        ) && diagnostic.line() == Some(1)
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.kind(),
            ProjectDiagnosticKind::BrokenLink { target }
                if target == "missing"
        )
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.kind(),
            ProjectDiagnosticKind::BrokenLink { target }
                if target == "ghost"
        )
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.kind(),
            ProjectDiagnosticKind::BrokenLink { target }
                if target == "quoted-missing"
        )
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.kind(),
            ProjectDiagnosticKind::BrokenLink { target }
                if target == "container-missing"
        )
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.kind(),
            ProjectDiagnosticKind::AmbiguousLink { target }
                if target == "same"
        )
    }));
}

#[test]
fn diagnostics_without_external_links_skips_external_link_checks() {
    let project = temp_project("local-diagnostics");
    write_note_with_content(
        &project,
        "start.maki",
        "See [Down](https://down.example/path) and [[missing]].",
    );

    let maki = Maki::load(&project.root).unwrap();
    let diagnostics = maki.diagnostics_without_external_links();

    assert!(diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.kind(),
            ProjectDiagnosticKind::BrokenLink { target } if target == "missing"
        )
    }));
    assert!(!diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.kind(),
        ProjectDiagnosticKind::BrokenExternalLink { .. }
    )));
}

#[test]
fn diagnostics_collect_broken_external_links() {
    let project = temp_project("external-link-diagnostics");
    write_note_with_content(
        &project,
        "start.maki",
        r#"See [Down](https://down.example/path), https://ok.example/docs, and `https://code.example`.

--- quote
See https://down.example/path.
---"#,
    );

    let maki = Maki::load(&project.root).unwrap();
    let checked = RefCell::new(vec![]);
    let diagnostics = maki.diagnostics_with_external_link_checker(&|target| {
        checked.borrow_mut().push(target.to_string());
        if target == "https://down.example/path" {
            ExternalLinkCheck::Broken {
                reason: "HTTP 404".to_string(),
            }
        } else {
            ExternalLinkCheck::Ok
        }
    });

    assert_eq!(
        checked.into_inner(),
        vec![
            "https://down.example/path".to_string(),
            "https://ok.example/docs".to_string(),
        ]
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.source_path() == Path::new("start.maki")
            && matches!(
                diagnostic.kind(),
                ProjectDiagnosticKind::BrokenExternalLink { target, reason }
                    if target == "https://down.example/path" && reason == "HTTP 404"
            )
    }));
    assert!(!diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.kind(),
            ProjectDiagnosticKind::BrokenExternalLink { target, .. }
                if target == "https://ok.example/docs"
                    || target == "https://code.example"
        )
    }));
}

#[test]
fn diagnostics_collect_links_inside_strong_inline() {
    let project = temp_project("strong-link-diagnostics");
    write_note_with_content(
        &project,
        "start.maki",
        r#"See *[[missing]] and [Missing](/missing-note) and [Down](https://down.example/path)*."#,
    );

    let maki = Maki::load(&project.root).unwrap();
    let checked = RefCell::new(vec![]);
    let diagnostics = maki.diagnostics_with_external_link_checker(&|target| {
        checked.borrow_mut().push(target.to_string());
        ExternalLinkCheck::Broken {
            reason: "HTTP 404".to_string(),
        }
    });

    assert_eq!(
        checked.into_inner(),
        vec!["https://down.example/path".to_string()]
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.kind(),
            ProjectDiagnosticKind::BrokenLink { target } if target == "missing"
        )
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.kind(),
            ProjectDiagnosticKind::BrokenLink { target } if target == "missing-note"
        )
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.kind(),
            ProjectDiagnosticKind::BrokenExternalLink { target, reason }
                if target == "https://down.example/path" && reason == "HTTP 404"
        )
    }));
}

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

#[test]
fn search_entries_use_title_property_or_file_stem() {
    let project = temp_project("search-entry-title");
    write_note_with_content(&project, "alpha.maki", "--^ title: Alpha Note\n\nbody");
    write_note_with_content(&project, "beta-note.maki", "body");

    let maki = Maki::load(&project.root).unwrap();

    assert!(maki.search_entries().iter().any(|entry| {
        entry.title() == "Alpha Note"
            && entry.path() == "/alpha"
            && entry.source_path() == "alpha.maki"
    }));
    assert!(maki.search_entries().iter().any(|entry| {
        entry.title() == "beta-note"
            && entry.path() == "/beta-note"
            && entry.source_path() == "beta-note.maki"
    }));
}

#[test]
fn recent_entries_sort_by_modified_descending_then_source_path() {
    let base = UNIX_EPOCH + Duration::from_secs(1_000);
    let entries = collect_recent_entries(vec![
        NoteMetadataEntry {
            title: "Older".to_string(),
            path: "/older".to_string(),
            source_path: "older.maki".to_string(),
            modified: Some(base),
        },
        NoteMetadataEntry {
            title: "Tie B".to_string(),
            path: "/tie-b".to_string(),
            source_path: "b.maki".to_string(),
            modified: Some(base + Duration::from_secs(10)),
        },
        NoteMetadataEntry {
            title: "Tie A".to_string(),
            path: "/tie-a".to_string(),
            source_path: "a.maki".to_string(),
            modified: Some(base + Duration::from_secs(10)),
        },
        NoteMetadataEntry {
            title: "Unknown".to_string(),
            path: "/unknown".to_string(),
            source_path: "unknown.maki".to_string(),
            modified: None,
        },
    ]);

    let titles = entries
        .iter()
        .map(|entry| entry.title())
        .collect::<Vec<_>>();
    assert_eq!(titles, vec!["Tie A", "Tie B", "Older", "Unknown"]);
}

#[test]
fn search_titles_matches_case_insensitive_title_substrings() {
    let project = temp_project("search-title-match");
    write_note_with_content(&project, "alpha.maki", "--^ title: Alpha Note\n\nbody");
    write_note_with_content(&project, "beta.maki", "--^ title: Beta Note\n\nbody");
    write_note_with_content(&project, "gamma.maki", "--^ title: Gamma\n\nbody");

    let maki = Maki::load(&project.root).unwrap();
    let titles = maki
        .search_titles("NOTE", 10)
        .iter()
        .map(|entry| entry.title().to_string())
        .collect::<Vec<_>>();

    assert_eq!(titles, vec!["Beta Note", "Alpha Note"]);
}
