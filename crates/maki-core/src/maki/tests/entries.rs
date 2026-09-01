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
    assert!(maki.search_entries().iter().any(|entry| {
        entry.kind() == SearchEntryKind::Id
            && entry.title() == "stable-heading"
            && entry.path() == "/alpha#stable-heading"
            && entry.source_path() == "alpha.maki@stable-heading"
    }));
}

#[test]
fn recent_entries_sort_by_modified_descending_then_source_path() {
    let base = UNIX_EPOCH + Duration::from_secs(1_000);
    let entries = collect_recent_entries(vec![
        NoteMetadataEntry {
            title: "Older".to_string(),
            title_is_file_stem: false,
            path: "/older".to_string(),
            source_path: "older.maki".to_string(),
            modified: Some(base),
        },
        NoteMetadataEntry {
            title: "Tie B".to_string(),
            title_is_file_stem: false,
            path: "/tie-b".to_string(),
            source_path: "b.maki".to_string(),
            modified: Some(base + Duration::from_secs(10)),
        },
        NoteMetadataEntry {
            title: "Tie A".to_string(),
            title_is_file_stem: false,
            path: "/tie-a".to_string(),
            source_path: "a.maki".to_string(),
            modified: Some(base + Duration::from_secs(10)),
        },
        NoteMetadataEntry {
            title: "Unknown".to_string(),
            title_is_file_stem: false,
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
fn recent_entry_disambiguation_preserves_modified_and_source_path_sorting() {
    let base = UNIX_EPOCH + Duration::from_secs(1_000);
    let entries = collect_recent_entries(vec![
        NoteMetadataEntry {
            title: "same".to_string(),
            title_is_file_stem: true,
            path: "/older".to_string(),
            source_path: "c/same.maki".to_string(),
            modified: Some(base),
        },
        NoteMetadataEntry {
            title: "same".to_string(),
            title_is_file_stem: true,
            path: "/tie-b".to_string(),
            source_path: "b/same.maki".to_string(),
            modified: Some(base + Duration::from_secs(10)),
        },
        NoteMetadataEntry {
            title: "same".to_string(),
            title_is_file_stem: true,
            path: "/tie-a".to_string(),
            source_path: "a/same.maki".to_string(),
            modified: Some(base + Duration::from_secs(10)),
        },
    ]);

    let actual = entries
        .iter()
        .map(|entry| (entry.path(), entry.title(), entry.modified()))
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            ("/tie-a", "a/same", Some(base + Duration::from_secs(10))),
            ("/tie-b", "b/same", Some(base + Duration::from_secs(10))),
            ("/older", "c/same", Some(base)),
        ]
    );
}

#[test]
fn recent_entries_disambiguate_duplicate_file_stems_with_minimal_path_suffixes() {
    let project = temp_project("recents-duplicate-file-stems");
    write_note(&project, "notes/코딩 테스트.maki");
    write_note(&project, "notes/제2차 미래 먹거리 계획/코딩 테스트.maki");
    write_note(&project, "notes/A/개발 & 계획.maki");
    write_note(&project, "archive/A/개발 & 계획.maki");
    write_note(&project, "notes/로드맵.maki");
    write_note(&project, "notes/A/로드맵.maki");
    write_note(&project, "notes/B/A/로드맵.maki");
    write_note(&project, "notes/안내서.v2.maki");
    write_note(&project, "archive/안내서.v2.maki");
    write_note(&project, "notes/고유 문서.maki");
    write_note_with_content(&project, "authored/one.maki", "--^ title: 같은 제목\n");
    write_note_with_content(&project, "authored/two.maki", "--^ title: 같은 제목\n");
    write_note_with_content(
        &project,
        "authored/같은 제목.maki",
        "--^ title: 같은 제목\n",
    );
    write_note(&project, "fallback/같은 제목.maki");

    let mut maki = Maki::load(&project.root).unwrap();
    let titles_by_path = maki
        .recent_entries()
        .iter()
        .map(|entry| (entry.path().to_string(), entry.title().to_string()))
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(titles_by_path["/notes/코딩 테스트"], "코딩 테스트");
    assert_eq!(
        titles_by_path["/notes/제2차 미래 먹거리 계획/코딩 테스트"],
        "제2차 미래 먹거리 계획/코딩 테스트"
    );
    assert_eq!(
        titles_by_path["/notes/A/개발 & 계획"],
        "notes/A/개발 & 계획"
    );
    assert_eq!(
        titles_by_path["/archive/A/개발 & 계획"],
        "archive/A/개발 & 계획"
    );
    assert_eq!(titles_by_path["/notes/로드맵"], "로드맵");
    assert_eq!(titles_by_path["/notes/A/로드맵"], "A/로드맵");
    assert_eq!(titles_by_path["/notes/B/A/로드맵"], "B/A/로드맵");
    assert_eq!(titles_by_path["/notes/안내서.v2"], "notes/안내서.v2");
    assert_eq!(titles_by_path["/archive/안내서.v2"], "archive/안내서.v2");
    assert_eq!(titles_by_path["/notes/고유 문서"], "고유 문서");
    assert_eq!(titles_by_path["/authored/one"], "같은 제목");
    assert_eq!(titles_by_path["/authored/two"], "같은 제목");
    assert_eq!(titles_by_path["/authored/같은 제목"], "같은 제목");
    assert_eq!(titles_by_path["/fallback/같은 제목"], "같은 제목");
    assert!(
        titles_by_path
            .values()
            .all(|title| !title.ends_with(".maki"))
    );

    maki.apply_recent_modified_times(&std::collections::BTreeMap::from([(
        PathBuf::from("notes/코딩 테스트.maki"),
        UNIX_EPOCH + Duration::from_secs(1_000),
    )]));
    let titles_after_modified_times = maki
        .recent_entries()
        .iter()
        .map(|entry| (entry.path().to_string(), entry.title().to_string()))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(titles_after_modified_times, titles_by_path);
}

#[test]
fn recent_entries_keep_snapshot_titles_after_modified_times_are_applied() {
    let project = temp_project("snapshot-recents-title");
    let modified = UNIX_EPOCH + Duration::from_secs(1_000);
    write_note_with_content(&project, "alpha.maki", "--^ title: Alpha Note\n\nbody");

    let mut maki = Maki::load(&project.root).unwrap();
    maki.apply_recent_modified_times(&std::collections::BTreeMap::from([(
        PathBuf::from("alpha.maki"),
        modified,
    )]));

    assert!(maki.recent_entries().iter().any(|entry| {
        entry.title() == "Alpha Note"
            && entry.path() == "/alpha"
            && entry.modified() == Some(modified)
    }));
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

#[test]
fn loaded_project_uses_an_immutable_source_snapshot() {
    let project = temp_project("immutable-source-snapshot");
    write_note_with_content(&project, "index.maki", "Before reload");

    let maki = Maki::load(&project.root).unwrap();
    write_note_with_content(&project, "index.maki", "After reload");

    assert_eq!(
        maki.get_raw_content(Path::new("index.maki")).unwrap(),
        "Before reload"
    );
    assert!(
        maki.render_html(Path::new("index.maki"))
            .unwrap()
            .contains("Before reload")
    );
    assert!(
        Maki::load(&project.root)
            .unwrap()
            .render_html(Path::new("index.maki"))
            .unwrap()
            .contains("After reload")
    );
}
