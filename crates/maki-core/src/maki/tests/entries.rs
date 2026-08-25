use super::*;

#[test]
fn search_entries_use_title_property_or_file_stem() {
    let project = temp_project("search-entry-title");
    write_note_with_content(&project, "alpha.maki", "--^ title: Alpha Note\n\nbody");
    write_note_with_content(&project, "beta-note.maki", "body");

    let maki = Maki::load(&project.root).unwrap();

    assert!(maki.search_entries().iter().any(|entry| {
        entry.kind() == SearchEntryKind::Note
            && entry.title() == "Alpha Note"
            && entry.path() == "/alpha"
            && entry.source_path() == "alpha.maki"
    }));
    assert!(maki.search_entries().iter().any(|entry| {
        entry.kind() == SearchEntryKind::Note
            && entry.title() == "beta-note"
            && entry.path() == "/beta-note"
            && entry.source_path() == "beta-note.maki"
    }));
}

#[test]
fn search_entries_include_source_files_and_headings() {
    let project = temp_project("search-entry-kinds");
    write_note_with_content(
        &project,
        "alpha.maki",
        r#"--^ title: Alpha Note

= Overview

== Stable Heading
--^ id: stable-heading

body"#,
    );

    let maki = Maki::load(&project.root).unwrap();

    assert!(maki.search_entries().iter().any(|entry| {
        entry.kind() == SearchEntryKind::File
            && entry.title() == "alpha.maki"
            && entry.path() == "/alpha"
            && entry.source_path() == "alpha.maki"
    }));
    assert!(maki.search_entries().iter().any(|entry| {
        entry.kind() == SearchEntryKind::Heading
            && entry.title() == "Overview"
            && entry.path() == "/alpha#Overview"
            && entry.source_path() == "alpha.maki#Overview"
    }));
    assert!(maki.search_entries().iter().any(|entry| {
        entry.kind() == SearchEntryKind::Heading
            && entry.title() == "Stable Heading"
            && entry.path() == "/alpha#stable-heading"
            && entry.source_path() == "alpha.maki#Stable Heading"
    }));

    let results = maki.search_titles("stable", 10);
    assert!(results.iter().any(|entry| {
        entry.kind() == SearchEntryKind::Heading && entry.path() == "/alpha#stable-heading"
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
