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
fn direct_href_safety_allows_web_and_local_links_but_rejects_active_content() {
    for target in [
        "https://example.com",
        "HTTP://example.com",
        "mailto:me@example.com",
        "tel:+82000000000",
        "/docs/page",
        "docs/page",
        "#heading",
        "//cdn.example.com/file",
    ] {
        assert!(is_safe_direct_href(target), "expected safe href: {target}");
    }
    for target in [
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "data:text/html,unsafe",
        "vbscript:unsafe",
        "file:///etc/passwd",
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
fn resolve_note_link_supports_root_child_heading_and_document_local_id_selectors() {
    let project = temp_project("nested-document-selectors");
    write_note_with_content(
        &project,
        "plan.maki",
        r#"--^ title: Plan

Current paragraph
--^ id: current-id

[[#Current section]] [[@current-id]]

= Current section"#,
    );
    write_note_with_content(
        &project,
        "plan/coding.maki",
        r#"--^ title: Coding

= Preparation

- Solve problems
--^ id: checklist"#,
    );
    let maki = Maki::load(&project.root).unwrap();
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
    let project = temp_project("explicit-document-coordinate");
    write_note(&project, "plan.maki");
    write_note(&project, "other/coding.maki");
    let maki = Maki::load(&project.root).unwrap();
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
    let project = temp_project("document-local-ids");
    write_note_with_content(
        &project,
        "alpha.maki",
        "Alpha\n--^ id: schedule\n\nDuplicate\n--^ id: duplicate\n\nAgain\n--^ id: duplicate",
    );
    write_note_with_content(&project, "beta.maki", "Beta\n--^ id: schedule");
    let maki = Maki::load(&project.root).unwrap();

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
    let project = temp_project("fragment-collision");
    write_note_with_content(
        &project,
        "index.maki",
        "= shared\n\nTarget block\n--^ id: shared\n\n[[#shared]] [[@shared]]",
    );
    let maki = Maki::load(&project.root).unwrap();
    let current = NoteRef::new("index");

    assert_eq!(
        maki.resolve_note_link(&current, "#shared"),
        NoteLinkResolution::Ambiguous
    );
    assert_eq!(
        maki.resolve_note_link(&current, "@shared"),
        NoteLinkResolution::Ambiguous
    );
    assert!(
        maki.diagnostics_without_external_links()
            .iter()
            .any(|diagnostic| {
                matches!(
                    diagnostic.kind(),
                    ProjectDiagnosticKind::DuplicateId { id } if id == "shared"
                )
            })
    );

    let html = maki.render_html(Path::new("index.maki")).unwrap();
    assert_eq!(html.matches("class=\"ambiguous-link\"").count(), 2);
    assert!(!maki.search_entries().iter().any(|entry| {
        matches!(entry.kind(), SearchEntryKind::Heading | SearchEntryKind::Id)
            && entry.path() == "/index#shared"
    }));
}

#[test]
fn rendered_project_pages_expose_block_id_fragments_and_direct_document_relations() {
    let project = temp_project("document-navigation");
    write_note_with_content(&project, "plan.maki", "--^ title: Plan\n\nBody");
    write_note_with_content(
        &project,
        "plan/coding.maki",
        "--^ title: Coding\n\nTarget paragraph\n--^ id: target",
    );
    write_note_with_content(
        &project,
        "plan/interviews.maki",
        "--^ title: Interviews\n\nBody",
    );
    write_note_with_content(
        &project,
        "plan/coding/week-one.maki",
        "--^ title: Week One\n\nBody",
    );
    write_note_with_content(
        &project,
        "plan/missing/deep.maki",
        "--^ title: Deep\n\nBody",
    );
    write_note_with_content(
        &project,
        "partial/parent.maki",
        "--^ title: Partial Parent\n\nBody",
    );
    write_note_with_content(
        &project,
        "partial/parent/deep.maki",
        "--^ title: Partial Deep\n\nBody",
    );
    let maki = Maki::load(&project.root).unwrap();

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
    let project = temp_project("subdocument-routes");
    write_note_with_content(&project, "plan.maki", "--^ title: Plan <&>\n\nBody");
    write_note_with_content(&project, "plan/a.maki", "--^ title: Zeta <child>\n\nBody");
    write_note_with_content(&project, "plan/z.maki", "--^ title: Alpha & child\n\nBody");
    write_note_with_content(
        &project,
        "plan/a/deep.maki",
        "--^ title: Deep child\n\nBody",
    );
    write_note_with_content(&project, "leaf.maki", "--^ title: Leaf\n\nBody");
    let maki = Maki::load(&project.root).unwrap();

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
        "See [the page][].\n\n[the page]: [[page]]",
    );
    write_note_with_content(&project, "page.maki", "--^ title: Page\n\nbody");

    let maki = Maki::load(&project.root).unwrap();
    let html = maki.render_html(Path::new("start.maki")).unwrap();

    assert!(html.contains("<a href=\"/page\">the page</a>"));
}

#[test]
fn direct_links_preserve_local_hrefs_without_note_resolution() {
    let project = temp_project("direct-local-href");
    write_note_with_content(
        &project,
        "start.maki",
        "[download](assets/archive) [section](#details)",
    );

    let maki = Maki::load(&project.root).unwrap();
    let html = maki.render_html(Path::new("start.maki")).unwrap();

    assert!(html.contains("<a href=\"assets/archive\">download</a>"));
    assert!(html.contains("<a href=\"#details\">section</a>"));
    assert!(maki.diagnostics_without_external_links().is_empty());
}

#[test]
fn reference_external_links_render_as_plain_hrefs() {
    let project = temp_project("reference-external-link");
    write_note_with_content(
        &project,
        "start.maki",
        "See [djot][].\n\n[djot]: <https://github.com/jgm/djot>",
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
