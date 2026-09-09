use super::*;

#[test]
fn diagnostics_collect_parse_warnings_and_link_resolution_issues() {
    let project = test_project("diagnostics");
    add_source(
        &project,
        "start.maki",
        r#"--^ invalid-property

See [[missing]], [Ghost][], and [[same]].

[Ghost]: [[ghost]]

> See [[quoted-missing]].
> [quoted reference][]
> [quoted reference]: [[quoted-reference-missing]]

--- quote
See [[container-missing]].
---"#,
    );
    add_empty_source(&project, "alpha/same.maki");
    add_empty_source(&project, "beta/same.maki");

    let maki = project.compile();
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
                if target == "quoted-reference-missing"
        )
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
fn diagnostics_preserve_note_order_when_source_paths_sort_differently() {
    let project = test_project("diagnostic-order");
    add_source(&project, "foo.maki", "[[missing-parent]]");
    add_source(&project, "foo/bar.maki", "[[missing-child]]");

    let maki = project.compile();
    let paths = maki
        .diagnostics()
        .into_iter()
        .filter_map(|diagnostic| {
            matches!(diagnostic.kind(), ProjectDiagnosticKind::BrokenLink { .. })
                .then(|| diagnostic.source_path().to_path_buf())
        })
        .collect::<Vec<_>>();

    assert_eq!(
        paths,
        vec![PathBuf::from("foo.maki"), PathBuf::from("foo/bar.maki")]
    );
}

#[test]
fn pure_diagnostics_do_not_include_external_link_results() {
    let project = test_project("local-diagnostics");
    add_source(
        &project,
        "start.maki",
        "See [Down][] and [[missing]].\n\n[Down]: <https://down.example/path>",
    );

    let maki = project.compile();
    let diagnostics = maki.diagnostics();

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
fn diagnostics_compose_supplied_broken_external_link_results() {
    let project = test_project("external-link-diagnostics");
    add_source(
        &project,
        "start.maki",
        r#"See [Down][], <https://ok.example/docs>, and `https://code.example`.

[Down]: <https://down.example/path>

--- quote
See <https://down.example/path>.
---"#,
    );

    let maki = project.compile();
    let external_links = maki
        .external_links()
        .iter()
        .map(|link| link.target.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        external_links,
        vec!["https://down.example/path", "https://ok.example/docs"]
    );

    let checks = BTreeMap::from([
        (
            "https://down.example/path".to_string(),
            ExternalLinkCheck::Broken {
                reason: "HTTP 404".to_string(),
            },
        ),
        ("https://ok.example/docs".to_string(), ExternalLinkCheck::Ok),
    ]);
    let diagnostics = maki.diagnostics_with_external_link_checks(&checks);

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
    let project = test_project("strong-link-diagnostics");
    add_source(
        &project,
        "start.maki",
        r#"See *[[missing]] and [Missing][] and <https://down.example/path>*.

[Missing]: [[/missing-note]]"#,
    );

    let maki = project.compile();
    let checks = BTreeMap::from([(
        "https://down.example/path".to_string(),
        ExternalLinkCheck::Broken {
            reason: "HTTP 404".to_string(),
        },
    )]);
    let diagnostics = maki.diagnostics_with_external_link_checks(&checks);
    assert!(diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.kind(),
            ProjectDiagnosticKind::BrokenLink { target } if target == "missing"
        )
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.kind(),
            ProjectDiagnosticKind::BrokenLink { target } if target == "/missing-note"
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
fn diagnostics_do_not_report_missing_footnote_definitions() {
    let project = test_project("missing-footnote-diagnostics");
    add_source(&project, "start.maki", "Missing [^note] stays text.");

    let maki = project.compile();

    assert!(maki.diagnostics().is_empty());
}

#[test]
fn diagnostics_report_unresolved_explicit_references_but_not_bare_markers() {
    let project = test_project("unresolved-reference-diagnostics");
    add_source(
        &project,
        "start.maki",
        "[missing] [^missing] [missing][] [title][ missing ] [^missing][] [^][ missing ]",
    );

    let maki = project.compile();
    let unresolved = maki
        .diagnostics()
        .into_iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic.kind(),
                ProjectDiagnosticKind::UnresolvedReference { key } if key == "missing"
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(unresolved.len(), 4);
    assert!(
        unresolved
            .iter()
            .all(|diagnostic| diagnostic.line() == Some(1))
    );
    let summary = ProjectDiagnosticSummary::from_diagnostics(&unresolved);
    assert_eq!(summary.unresolved_references(), 4);
}

#[test]
fn reference_values_contribute_one_external_link_for_their_shared_shape() {
    let project = test_project("reference-value-shape-diagnostics");
    add_source(
        &project,
        "start.maki",
        r#"[raw][] [prose][]

[raw]: [[missing]]
[prose]: <https://example.com/a> has details"#,
    );

    let maki = project.compile();
    let broken_targets = maki
        .diagnostics()
        .into_iter()
        .filter_map(|diagnostic| match diagnostic.kind() {
            ProjectDiagnosticKind::BrokenLink { target } => Some(target.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(broken_targets, vec!["missing".to_string()]);

    assert_eq!(
        maki.external_links()
            .iter()
            .map(|link| link.target.as_str())
            .collect::<Vec<_>>(),
        vec!["https://example.com/a"]
    );
    let checks = BTreeMap::from([("https://example.com/a".to_string(), ExternalLinkCheck::Ok)]);
    assert!(
        maki.diagnostics_with_external_link_checks(&checks)
            .iter()
            .all(|diagnostic| !matches!(
                diagnostic.kind(),
                ProjectDiagnosticKind::BrokenExternalLink { .. }
            ))
    );
}

#[test]
fn diagnostics_report_every_duplicate_id_declaration_with_its_line() {
    let project = test_project("duplicate-id-diagnostics");
    add_source(
        &project,
        "start.maki",
        "First\n--^ id: shared\n\nSecond\n--^ id: shared",
    );

    let maki = project.compile();
    let diagnostics = maki.diagnostics();
    let duplicate_lines = diagnostics
        .iter()
        .filter_map(|diagnostic| match diagnostic.kind() {
            ProjectDiagnosticKind::DuplicateId { id } if id == "shared" => diagnostic.line(),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(duplicate_lines, vec![2, 5]);
    let summary = ProjectDiagnosticSummary::from_diagnostics(&diagnostics);
    assert_eq!(summary.duplicate_ids(), 2);
}

#[test]
fn diagnostics_ignore_links_inside_raw_quotes() {
    let project = test_project("raw-quote-diagnostics");
    add_source(
        &project,
        "start.maki",
        r#"--v mode: pre
> [[not-a-link]]

--v mode: text
---quote
[[also-not-a-link]]
---"#,
    );

    let maki = project.compile();

    assert!(maki.diagnostics().is_empty());
}
