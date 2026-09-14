use super::*;
use crate::parser;

#[test]
fn project_compile_reuses_root_parses_until_render() {
    const SOURCE_COUNT: usize = 3;

    let mut project = test_project("parse-once-runtime");
    project.add_source(
        "index.maki",
        r#"--^ title: Home
--^ date: [2026-09-08]

See [[/notes/child]] and <https://example.com/docs>.

> Quoted [2026-09-09]."#,
    );
    project.add_source(
        "notes/child.maki",
        "--^ title: Child\n\nBack to [[/index]].",
    );
    project.add_source("archive.maki", "--^ title: Archive\n\nStored.");

    parser::reset_parse_counters();
    let mut maki = project.compile();
    let compile_counts = parser::read_parse_counters();

    assert_eq!(compile_counts.root, SOURCE_COUNT);
    assert_eq!(compile_counts.nested, 1);

    let analysis = maki.analysis().unwrap();
    assert_eq!(analysis.documents().len(), SOURCE_COUNT);
    assert_eq!(analysis.external_links().len(), 1);
    assert!(!maki.search_entries().is_empty());
    assert!(!maki.search_titles("home", 10).is_empty());
    assert!(maki.date_index().dates().next().is_some());
    assert!(maki.diagnostics().is_empty());
    let checks = BTreeMap::from([(
        "https://example.com/docs".to_string(),
        ExternalLinkCheck::Ok,
    )]);
    assert!(
        maki.diagnostics_with_external_link_checks(&checks)
            .is_empty()
    );

    maki.apply_recent_modified_times(&std::collections::BTreeMap::from([(
        PathBuf::from("index.maki"),
        UNIX_EPOCH + Duration::from_secs(1),
    )]));
    assert!(!maki.recent_entries().is_empty());
    assert!(!maki.sitemap_entries().is_empty());
    assert_eq!(parser::read_parse_counters(), compile_counts);

    maki.render_html(Path::new("index.maki")).unwrap();
    let render_counts = parser::read_parse_counters();

    assert_eq!(render_counts.root, compile_counts.root + 1);
    assert!(render_counts.nested > compile_counts.nested);
}

#[test]
fn project_compile_uses_supplied_source_metadata() {
    let mut project = test_project("source-metadata");
    let modified = UNIX_EPOCH + Duration::from_secs(1_000);
    project.add_source_with_modified("index.maki", "--^ title: Home\n\nBody", modified);

    let maki = project.compile();

    assert_eq!(maki.root(), project.root);
    assert_eq!(maki.recent_entries()[0].modified(), Some(modified));
}

#[test]
fn project_compile_preserves_read_failures_as_snapshot_data() {
    let mut project = test_project("read-failure");
    project.add_source("index.maki", "--^ invalid-property");
    project.add_read_failure("broken.maki");

    let maki = project.compile();
    let report = maki.validation_report();

    assert!(matches!(
        maki.analysis(),
        Err(Error::ReadNoteFailed(path)) if path == project.root.join("broken.maki")
    ));
    assert_eq!(
        maki.source(Path::new("index.maki")),
        Some("--^ invalid-property")
    );
    assert!(!report.is_complete());
    assert!(report.has_findings());
    assert_eq!(
        report.unavailable_sources(),
        &[PathBuf::from("broken.maki")]
    );
    assert_eq!(report.summary().warnings(), 1);
    assert_eq!(report.diagnostics()[0].code(), "invalid-property");
    assert!(maki.diagnostics().iter().any(|diagnostic| {
        diagnostic.source_path() == Path::new("broken.maki")
            && matches!(diagnostic.kind(), ProjectDiagnosticKind::ReadFailed)
    }));
}

#[test]
fn public_policy_requires_an_exact_root_document_publish_declaration() {
    let mut project = test_project("public-opt-in");
    project.add_source(
        "published.maki",
        "--^ title: Published\n--^ publish: all\n\nVisible.",
    );
    project.add_source("missing.maki", "--^ title: Missing declaration\n");
    project.add_source(
        "wrong.maki",
        "--^ title: Wrong value\n--^ publish: everything\n",
    );
    project.add_source(
        "nested.maki",
        "--^ title: Nested declaration\n\n> --^ publish: all\n>\n> Nested.",
    );
    project.add_source(
        "overridden-private.maki",
        "--^ publish: all\n--^ publish: none\n",
    );
    project.add_source(
        "overridden-public.maki",
        "--^ publish: none\n--^ PuBlIsH: all\n",
    );
    project.add_source("uppercase-value.maki", "--^ publish: ALL\n");
    project.add_read_failure("unreadable.maki");

    let mut maki = project.compile();
    assert_eq!(maki.notes_len(), 8);
    assert_eq!(
        maki.resolve_route("/published.maki").unwrap(),
        MakiRoute::NoteSource(PathBuf::from("published.maki"))
    );

    maki.set_publish_policy(PublishPolicy::Public);

    let visible_paths = maki
        .notes()
        .map(|note| note.source_path().to_path_buf())
        .collect::<Vec<_>>();
    assert_eq!(
        visible_paths,
        vec![
            PathBuf::from("overridden-public.maki"),
            PathBuf::from("published.maki"),
        ]
    );
    assert_eq!(maki.notes_len(), 2);
    assert_eq!(maki.analysis().unwrap().documents().len(), 2);
    assert!(maki.validation_report().is_complete());
    assert!(maki.diagnostics().is_empty());

    assert_eq!(maki.source(Path::new("published.maki")), None);
    assert!(matches!(
        maki.get_raw_content(Path::new("published.maki")),
        Err(Error::NoteNotFound(_))
    ));
    assert!(matches!(
        maki.resolve_route("/published.maki"),
        Err(Error::NoteNotFound(_))
    ));
    for target in [
        "/missing",
        "/wrong",
        "/nested",
        "/overridden-private",
        "/uppercase-value",
        "/unreadable",
    ] {
        assert!(
            matches!(maki.resolve_route(target), Err(Error::NoteNotFound(_))),
            "expected {target} to fail closed"
        );
    }
}

#[test]
fn public_link_resolution_uses_only_public_candidates_and_redacts_failures() {
    let mut project = test_project("public-link-resolution");
    project.add_source(
        "home.maki",
        r#"--^ title: Home
--^ publish: all

[[twin]] [[private-secret]] [[missing-secret]] [private reference][private-ref]

[private-ref]: [[private-secret]]"#,
    );
    project.add_source(
        "public/twin.maki",
        "--^ title: Published Twin\n--^ publish: all\n",
    );
    project.add_source("private/twin.maki", "--^ title: Private Twin\n");
    project.add_source(
        "private-secret.maki",
        "--^ title: Private Secret\n\nsecret body",
    );

    let mut maki = project.compile();
    let home = NoteRef::new("home");
    assert_eq!(
        maki.resolve_note_link(&home, "twin"),
        NoteLinkResolution::Ambiguous
    );
    assert!(matches!(
        maki.resolve_note_link(&home, "private-secret"),
        NoteLinkResolution::Found(_)
    ));
    assert_eq!(
        maki.resolve_note_link(&home, "missing-secret"),
        NoteLinkResolution::Broken
    );

    maki.set_publish_policy(PublishPolicy::Public);

    assert_eq!(
        maki.resolve_note_link(&home, "twin"),
        NoteLinkResolution::Found(NoteRef::new("public/twin"))
    );
    assert_eq!(
        maki.resolve_note_link(&home, "private-secret"),
        NoteLinkResolution::Redacted
    );
    assert_eq!(
        maki.resolve_note_link(&home, "missing-secret"),
        NoteLinkResolution::Redacted
    );

    let html = maki.render_html(Path::new("home.maki")).unwrap();
    assert!(html.contains("href=\"/public/twin\""));
    assert_eq!(html.matches("[데이터 말소]").count(), 3);
    assert!(!html.contains("private-secret"));
    assert!(!html.contains("missing-secret"));
    assert!(!html.contains("Private Secret"));

    let analysis = maki.analysis().unwrap();
    let document = analysis.document(Path::new("home.maki")).unwrap();
    assert!(
        document
            .note_links
            .iter()
            .all(|link| !link.target.contains("private-secret")
                && !link.target.contains("missing-secret"))
    );
    assert!(
        document
            .reference_links
            .iter()
            .all(|link| !link.target.contains("private-secret"))
    );
    assert!(
        document
            .reference_graph
            .definitions
            .iter()
            .all(|definition| {
                !definition.value.contains("private-secret")
                    && definition
                        .semantic_target
                        .as_deref()
                        .is_none_or(|target| !target.contains("private-secret"))
            })
    );
}
