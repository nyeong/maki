use super::*;
use crate::parser;

#[test]
fn project_runtime_reuses_root_parses_until_render() {
    const SOURCE_COUNT: usize = 3;

    let project = temp_project("parse-once-runtime");
    write_note_with_content(
        &project,
        "index.maki",
        r#"--^ title: Home
--^ date: [2026-09-08]

See [[/notes/child]] and <https://example.com/docs>.

> Quoted [2026-09-09]."#,
    );
    write_note_with_content(
        &project,
        "notes/child.maki",
        "--^ title: Child\n\nBack to [[/index]].",
    );
    write_note_with_content(&project, "archive.maki", "--^ title: Archive\n\nStored.");

    parser::reset_parse_counters();
    let mut maki = Maki::load(&project.root).unwrap();
    let load_counts = parser::read_parse_counters();

    assert_eq!(load_counts.root, SOURCE_COUNT);
    assert_eq!(load_counts.nested, 1);

    let analysis = maki.analysis().unwrap();
    assert_eq!(analysis.documents().len(), SOURCE_COUNT);
    assert_eq!(analysis.external_links().len(), 1);
    assert!(!maki.search_entries().is_empty());
    assert!(!maki.search_titles("home", 10).is_empty());
    assert!(maki.date_index().dates().next().is_some());
    assert!(maki.diagnostics_without_external_links().is_empty());
    assert!(
        maki.diagnostics_with_external_link_checker(&|_| ExternalLinkCheck::Ok)
            .is_empty()
    );

    maki.apply_recent_modified_times(&std::collections::BTreeMap::from([(
        PathBuf::from("index.maki"),
        UNIX_EPOCH + Duration::from_secs(1),
    )]));
    assert!(!maki.recent_entries().is_empty());
    assert!(!maki.sitemap_entries().is_empty());
    assert_eq!(parser::read_parse_counters(), load_counts);

    maki.render_html(Path::new("index.maki")).unwrap();
    let render_counts = parser::read_parse_counters();

    assert_eq!(render_counts.root, load_counts.root + 1);
    assert!(render_counts.nested > load_counts.nested);
}
