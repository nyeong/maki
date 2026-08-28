use super::*;

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
        maki.resolve_note_link(&NoteRef::new("index"), "maki-toml"),
        NoteLinkResolution::Found(NoteRef::new("maki-toml"))
    );

    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("index"), "/maki-toml"),
        NoteLinkResolution::Found(NoteRef::new("maki-toml"))
    );
}

#[test]
fn resolve_note_link_supports_heading_anchors_and_stable_ids() {
    let project = temp_project("heading-link");
    write_note_with_content(
        &project,
        "start.maki",
        "= 소개\n--^ id: intro\n\n[[#intro]] [[other#詳細]]",
    );
    write_note_with_content(&project, "other.maki", "= 詳細");
    let maki = Maki::load(&project.root).unwrap();

    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("start"), "#intro"),
        NoteLinkResolution::FoundHeading {
            note: NoteRef::new("start"),
            anchor: "intro".to_string(),
        }
    );
    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("start"), "other#詳細"),
        NoteLinkResolution::FoundHeading {
            note: NoteRef::new("other"),
            anchor: "詳細".to_string(),
        }
    );

    let html = maki.render_html(Path::new("start.maki")).unwrap();
    assert!(html.contains("<h2 id=\"intro\">소개</h2>"));
    assert!(html.contains("href=\"/start#intro\""));
    assert!(html.contains("href=\"/other#詳細\""));
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
fn reference_links_can_resolve_to_notes_with_custom_titles() {
    let project = temp_project("reference-note-link");
    write_note_with_content(
        &project,
        "start.maki",
        "See [the page].\n\n[the page]: page",
    );
    write_note_with_content(&project, "page.maki", "--^ title: Page\n\nbody");

    let maki = Maki::load(&project.root).unwrap();
    let html = maki.render_html(Path::new("start.maki")).unwrap();

    assert!(html.contains("<a href=\"/page\">the page</a>"));
}

#[test]
fn reference_external_links_render_as_plain_hrefs() {
    let project = temp_project("reference-external-link");
    write_note_with_content(
        &project,
        "start.maki",
        "See [djot].\n\n[djot]: https://github.com/jgm/djot",
    );

    let maki = Maki::load(&project.root).unwrap();
    let html = maki.render_html(Path::new("start.maki")).unwrap();

    assert!(
        html.contains("<a class=\"external-link\" href=\"https://github.com/jgm/djot\">djot</a>")
    );
}

#[test]
fn angle_wrapped_external_urls_render_as_links_but_bare_urls_do_not() {
    let project = temp_project("hyper-link");
    write_note_with_content(
        &project,
        "start.maki",
        "See <https://example.com/docs>, not https://example.com/bare.",
    );

    let maki = Maki::load(&project.root).unwrap();
    let html = maki.render_html(Path::new("start.maki")).unwrap();

    assert!(html.contains(
        "<a class=\"external-link\" href=\"https://example.com/docs\">example.com/docs</a>, not https://example.com/bare."
    ));
    assert!(!html.contains("href=\"https://example.com/bare\""));
}
