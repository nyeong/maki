use super::*;

#[test]
fn note_path() {
    let note = Note::new("project", "docs/use-cases.maki", None);

    assert_eq!(note.source_path(), PathBuf::from("docs/use-cases.maki"));
    assert_eq!(note.canonical_path(), PathBuf::from("docs/use-cases"));
    assert_eq!(note.file_stem(), "use-cases");
    assert_eq!(note.note_ref().web_path(), "/docs/use-cases");
}

#[test]
fn note_ref() {
    let note = Note::new("project", "docs/use-cases.maki", None);
    let ref_ = note.note_ref();
    assert_eq!(ref_.canonical_path(), PathBuf::from("docs/use-cases"));
    assert_eq!(ref_.web_path(), "/docs/use-cases");
}

#[test]
fn direct_href_safety_allows_only_local_links() {
    for target in ["/docs/page", "docs/page", "../asset", "#heading"] {
        assert!(is_safe_direct_href(target), "expected safe href: {target}");
    }
    for target in [
        "https://example.com",
        "HTTP://example.com",
        "mailto:me@example.com",
        "tel:+82000000000",
        "//cdn.example.com/file",
        "\\\\cdn.example.com/file",
        "/\\cdn.example.com/file",
        "\\/cdn.example.com/file",
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "data:text/html,unsafe",
        "vbscript:unsafe",
        "file:///etc/passwd",
        "\tleading-control",
        "trailing-control\t",
        "java\tscript:alert(1)",
    ] {
        assert!(
            !is_safe_direct_href(target),
            "expected unsafe href: {target}"
        );
    }
}

#[test]
fn resolve_note_link() {
    let project = test_project("resolve-note-link");
    add_empty_source(&project, "index.maki");
    add_empty_source(&project, "use-cases.maki");
    add_empty_source(&project, "maki-toml.maki");
    let maki = project.compile();
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
    let project = test_project("heading-link");
    add_source(
        &project,
        "start.maki",
        "= 소개\n--^ id: intro\n\n[[#intro]] [[other#詳細]]",
    );
    add_source(&project, "other.maki", "= 詳細");
    let maki = project.compile();

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
fn resolve_note_link_supports_root_child_heading_and_document_local_id_selectors() {
    let project = test_project("nested-document-selectors");
    add_source(
        &project,
        "plan.maki",
        r#"--^ title: Plan

Current paragraph
--^ id: current-id

[[#Current section]] [[@current-id]]

= Current section"#,
    );
    add_source(
        &project,
        "plan/coding.maki",
        r#"--^ title: Coding

= Preparation

- Solve problems
--^ id: checklist"#,
    );
    let maki = project.compile();
    let current = NoteRef::new("plan");

    assert_eq!(
        maki.resolve_note_link(&current, "/plan/coding#Preparation"),
        NoteLinkResolution::FoundHeading {
            note: NoteRef::new("plan/coding"),
            anchor: "Preparation".to_string(),
        }
    );
    assert_eq!(
        maki.resolve_note_link(&current, "+coding#Preparation"),
        NoteLinkResolution::FoundHeading {
            note: NoteRef::new("plan/coding"),
            anchor: "Preparation".to_string(),
        }
    );
    assert_eq!(
        maki.resolve_note_link(&current, "/plan/coding@checklist"),
        NoteLinkResolution::FoundId {
            note: NoteRef::new("plan/coding"),
            id: "checklist".to_string(),
        }
    );
    assert_eq!(
        maki.resolve_note_link(&current, "+coding@checklist"),
        NoteLinkResolution::FoundId {
            note: NoteRef::new("plan/coding"),
            id: "checklist".to_string(),
        }
    );
    assert_eq!(
        maki.resolve_note_link(&current, "#Current section"),
        NoteLinkResolution::FoundHeading {
            note: current.clone(),
            anchor: "Current section".to_string(),
        }
    );
    assert_eq!(
        maki.resolve_note_link(&current, "@current-id"),
        NoteLinkResolution::FoundId {
            note: current,
            id: "current-id".to_string(),
        }
    );
}

#[test]
fn explicit_root_and_child_selectors_do_not_fall_back_to_project_wide_stems() {
    let project = test_project("explicit-document-coordinate");
    add_empty_source(&project, "plan.maki");
    add_empty_source(&project, "other/coding.maki");
    let maki = project.compile();
    let current = NoteRef::new("plan");

    assert_eq!(
        maki.resolve_note_link(&current, "coding"),
        NoteLinkResolution::Found(NoteRef::new("other/coding"))
    );
    assert_eq!(
        maki.resolve_note_link(&current, "/coding"),
        NoteLinkResolution::Broken
    );
    assert_eq!(
        maki.resolve_note_link(&current, "+coding"),
        NoteLinkResolution::Broken
    );
    assert_eq!(
        maki.resolve_note_link(&current, "+other/../coding"),
        NoteLinkResolution::Broken
    );
}

#[test]
fn document_local_ids_can_repeat_across_documents_but_are_exact_within_one_document() {
    let project = test_project("document-local-ids");
    add_source(
        &project,
        "alpha.maki",
        "Alpha\n--^ id: schedule\n\nDuplicate\n--^ id: duplicate\n\nAgain\n--^ id: duplicate",
    );
    add_source(&project, "beta.maki", "Beta\n--^ id: schedule");
    let maki = project.compile();

    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("alpha"), "@schedule"),
        NoteLinkResolution::FoundId {
            note: NoteRef::new("alpha"),
            id: "schedule".to_string(),
        }
    );
    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("alpha"), "/beta@schedule"),
        NoteLinkResolution::FoundId {
            note: NoteRef::new("beta"),
            id: "schedule".to_string(),
        }
    );
    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("alpha"), "@Schedule"),
        NoteLinkResolution::Broken
    );
    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("alpha"), "@duplicate"),
        NoteLinkResolution::Ambiguous
    );
}

#[test]
fn heading_and_explicit_id_selectors_are_ambiguous_when_their_html_fragments_collide() {
    let project = test_project("fragment-collision");
    add_source(
        &project,
        "index.maki",
        "= shared\n\nTarget block\n--^ id: shared\n\n[[#shared]] [[@shared]]",
    );
    let maki = project.compile();
    let current = NoteRef::new("index");

    assert_eq!(
        maki.resolve_note_link(&current, "#shared"),
        NoteLinkResolution::Ambiguous
    );
    assert_eq!(
        maki.resolve_note_link(&current, "@shared"),
        NoteLinkResolution::Ambiguous
    );
    assert!(maki.diagnostics().iter().any(|diagnostic| {
        matches!(
            diagnostic.kind(),
            ProjectDiagnosticKind::DuplicateId { id } if id == "shared"
        )
    }));

    let html = maki.render_html(Path::new("index.maki")).unwrap();
    assert_eq!(html.matches("class=\"ambiguous-link\"").count(), 2);
    assert!(!maki.search_entries().iter().any(|entry| {
        matches!(entry.kind(), SearchEntryKind::Heading | SearchEntryKind::Id)
            && entry.path() == "/index#shared"
    }));
}

#[test]
fn rendered_project_pages_expose_block_id_fragments_and_direct_document_relations() {
    let project = test_project("document-navigation");
    add_source(&project, "plan.maki", "--^ title: Plan\n\nBody");
    add_source(
        &project,
        "plan/coding.maki",
        "--^ title: Coding\n\nTarget paragraph\n--^ id: target",
    );
    add_source(
        &project,
        "plan/interviews.maki",
        "--^ title: Interviews\n\nBody",
    );
    add_source(
        &project,
        "plan/coding/week-one.maki",
        "--^ title: Week One\n\nBody",
    );
    add_source(
        &project,
        "plan/missing/deep.maki",
        "--^ title: Deep\n\nBody",
    );
    add_source(
        &project,
        "partial/parent.maki",
        "--^ title: Partial Parent\n\nBody",
    );
    add_source(
        &project,
        "partial/parent/deep.maki",
        "--^ title: Partial Deep\n\nBody",
    );
    let maki = project.compile();

    let parent_html = maki.render_html(Path::new("plan.maki")).unwrap();
    assert!(!parent_html.contains("aria-label=\"Parent documents\""));
    assert!(
        parent_html.contains(
            "<a class=\"maki-document-navigation-label\" href=\"/plan/\">Subdocuments</a>"
        )
    );
    assert!(!parent_html.contains(">Coding</a>"));
    assert!(!parent_html.contains(">Interviews</a>"));
    assert!(!parent_html.contains("href=\"/plan/coding/week-one\""));

    let child_html = maki.render_html(Path::new("plan/coding.maki")).unwrap();
    assert!(child_html.contains(
        "<nav class=\"maki-document-breadcrumb\" aria-label=\"Parent documents\"><span class=\"maki-document-navigation-label\">Parent documents</span><ol><li><a href=\"/plan\">Plan</a></li></ol></nav>"
    ));
    assert!(child_html.contains(
        "<a class=\"maki-document-navigation-label\" href=\"/plan/coding/\">Subdocuments</a>"
    ));
    assert!(!child_html.contains(">Week One</a>"));
    assert!(child_html.contains(
        "<span class=\"maki-block-anchor\" id=\"target\" aria-hidden=\"true\"></span><p>Target paragraph</p>"
    ));

    let grandchild_html = maki
        .render_html(Path::new("plan/coding/week-one.maki"))
        .unwrap();
    assert!(grandchild_html.contains(
        "<nav class=\"maki-document-breadcrumb\" aria-label=\"Parent documents\"><span class=\"maki-document-navigation-label\">Parent documents</span><ol><li><a href=\"/plan\">Plan</a><span class=\"maki-document-breadcrumb-separator\" aria-hidden=\"true\">›</span></li><li><a href=\"/plan/coding\">Coding</a></li></ol></nav>"
    ));
    assert!(!grandchild_html.contains("aria-label=\"Subdocuments\""));

    let missing_parent_html = maki
        .render_html(Path::new("plan/missing/deep.maki"))
        .unwrap();
    assert!(!missing_parent_html.contains("aria-label=\"Breadcrumb\""));

    let partial_ancestry_html = maki
        .render_html(Path::new("partial/parent/deep.maki"))
        .unwrap();
    assert!(partial_ancestry_html.contains(
        "<nav class=\"maki-document-breadcrumb\" aria-label=\"Parent documents\"><span class=\"maki-document-navigation-label\">Parent documents</span><ol><li><a href=\"/partial/parent\">Partial Parent</a></li></ol></nav>"
    ));
    assert!(!partial_ancestry_html.contains("href=\"/partial\""));
}

#[test]
fn subdocument_routes_and_pages_are_distinct_from_note_and_source_routes() {
    let project = test_project("subdocument-routes");
    add_source(&project, "plan.maki", "--^ title: Plan <&>\n\nBody");
    add_source(&project, "plan/a.maki", "--^ title: Zeta <child>\n\nBody");
    add_source(&project, "plan/z.maki", "--^ title: Alpha & child\n\nBody");
    add_source(
        &project,
        "plan/a/deep.maki",
        "--^ title: Deep child\n\nBody",
    );
    add_source(&project, "leaf.maki", "--^ title: Leaf\n\nBody");
    let maki = project.compile();

    assert_eq!(maki.resolve_route("/").unwrap(), MakiRoute::Home);
    assert_eq!(
        maki.resolve_route("/plan").unwrap(),
        MakiRoute::NotePage(PathBuf::from("plan.maki"))
    );
    assert_eq!(
        maki.resolve_route("/plan/").unwrap(),
        MakiRoute::SubdocumentsPage(PathBuf::from("plan.maki"))
    );
    assert_eq!(
        maki.resolve_route("/plan.maki").unwrap(),
        MakiRoute::NoteSource(PathBuf::from("plan.maki"))
    );
    assert!(maki.resolve_route("/plan.maki/").is_err());
    assert!(maki.resolve_route("/plan//").is_err());
    assert!(maki.resolve_route("/missing/").is_err());

    let html = maki
        .render_subdocuments_html(Path::new("plan.maki"))
        .unwrap();
    assert!(html.contains("<title>Subdocuments of Plan &lt;&amp;&gt;</title>"));
    assert!(html.contains(
        "<nav class=\"maki-subdocuments-parent\" aria-label=\"Parent document\"><span class=\"maki-document-navigation-label\">Parent document</span><a href=\"/plan\">Plan &lt;&amp;&gt;</a></nav>"
    ));
    let first = html
        .find("<a href=\"/plan/a\">Zeta &lt;child&gt;</a>")
        .unwrap();
    let second = html
        .find("<a href=\"/plan/z\">Alpha &amp; child</a>")
        .unwrap();
    assert!(first < second);
    assert!(!html.contains("Deep child"));

    let empty_html = maki
        .render_subdocuments_html(Path::new("leaf.maki"))
        .unwrap();
    assert!(empty_html.contains("No subdocuments."));
    assert!(empty_html.contains("href=\"/leaf\">Leaf</a>"));
}

#[test]
fn resolve_note_link_uses_case_insensitive_path_lookup() {
    let project = test_project("case-insensitive-path");
    add_empty_source(&project, "milestones/v0.maki");
    add_empty_source(&project, "index.maki");

    let maki = project.compile();

    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("index"), "Milestones/V0"),
        NoteLinkResolution::Found(NoteRef::new("milestones/v0"))
    );
}

#[test]
fn resolve_note_link_uses_case_insensitive_sibling_stem_lookup() {
    let project = test_project("case-insensitive-sibling");
    add_empty_source(&project, "notes/devenv.maki");
    add_empty_source(&project, "notes/nix.maki");

    let maki = project.compile();

    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("notes/devenv"), "Nix"),
        NoteLinkResolution::Found(NoteRef::new("notes/nix"))
    );
}

#[test]
fn resolve_note_link_prefers_sibling_stem_before_project_wide_stem() {
    let project = test_project("sibling-before-project-stem");
    add_empty_source(&project, "notes/page.maki");
    add_empty_source(&project, "notes/nix.maki");
    add_empty_source(&project, "other/Nix.maki");

    let maki = project.compile();

    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("notes/page"), "NIX"),
        NoteLinkResolution::Found(NoteRef::new("notes/nix"))
    );
}

#[test]
fn resolve_note_link_reports_case_insensitive_stem_ambiguity() {
    let project = test_project("case-insensitive-stem-ambiguity");
    add_empty_source(&project, "start.maki");
    add_empty_source(&project, "alpha/nix.maki");
    add_empty_source(&project, "beta/NIX.maki");

    let maki = project.compile();

    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("start"), "Nix"),
        NoteLinkResolution::Ambiguous
    );
}

#[test]
fn resolve_note_link_preserves_exact_path_priority() {
    let project = test_project("exact-before-sibling");
    add_empty_source(&project, "nix.maki");
    add_empty_source(&project, "notes/page.maki");
    add_empty_source(&project, "notes/nix.maki");

    let maki = project.compile();

    assert_eq!(
        maki.resolve_note_link(&NoteRef::new("notes/page"), "nix"),
        NoteLinkResolution::Found(NoteRef::new("nix"))
    );
}

#[test]
fn reference_links_can_resolve_to_notes_with_custom_titles() {
    let project = test_project("reference-note-link");
    add_source(
        &project,
        "start.maki",
        "See [the page][].\n\n[the page]: [[page]]",
    );
    add_source(&project, "page.maki", "--^ title: Page\n\nbody");

    let maki = project.compile();
    let html = maki.render_html(Path::new("start.maki")).unwrap();

    assert!(html.contains("<a href=\"/page\">the page</a>"));
}

#[test]
fn direct_links_preserve_local_hrefs_without_note_resolution() {
    let project = test_project("direct-local-href");
    add_source(
        &project,
        "start.maki",
        "[download](assets/archive) [section](#details)",
    );

    let maki = project.compile();
    let html = maki.render_html(Path::new("start.maki")).unwrap();

    assert!(html.contains("<a href=\"assets/archive\">download</a>"));
    assert!(html.contains("<a href=\"#details\">section</a>"));
    assert!(maki.diagnostics().is_empty());
}

#[test]
fn reference_external_links_render_as_plain_hrefs() {
    let project = test_project("reference-external-link");
    add_source(
        &project,
        "start.maki",
        "See [djot][].\n\n[djot]: <https://github.com/jgm/djot>",
    );

    let maki = project.compile();
    let html = maki.render_html(Path::new("start.maki")).unwrap();

    assert!(
        html.contains("<a class=\"external-link\" href=\"https://github.com/jgm/djot\">djot</a>")
    );
}

#[test]
fn angle_wrapped_external_urls_render_as_links_but_bare_urls_do_not() {
    let project = test_project("hyper-link");
    add_source(
        &project,
        "start.maki",
        "See <https://example.com/docs>, not https://example.com/bare.",
    );

    let maki = project.compile();
    let html = maki.render_html(Path::new("start.maki")).unwrap();

    assert!(html.contains(
        "<a class=\"external-link\" href=\"https://example.com/docs\">example.com/docs</a>, not https://example.com/bare."
    ));
    assert!(!html.contains("href=\"https://example.com/bare\""));
}
