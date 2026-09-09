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
    project.add_empty_source("index.maki");
    project.add_read_failure("broken.maki");

    let maki = project.compile();

    assert!(matches!(
        maki.analysis(),
        Err(Error::ReadNoteFailed(path)) if path == project.root.join("broken.maki")
    ));
    assert!(maki.diagnostics().iter().any(|diagnostic| {
        diagnostic.source_path() == Path::new("broken.maki")
            && matches!(diagnostic.kind(), ProjectDiagnosticKind::ReadFailed)
    }));
}
