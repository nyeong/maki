use super::*;

#[test]
fn diagnostics_collect_parse_warnings_and_link_resolution_issues() {
    let project = temp_project("diagnostics");
    write_note_with_content(
        &project,
        "start.maki",
        r#"--^ invalid-property

See [[missing]], [Ghost], and [[same]].

[Ghost]: ghost

> See [[quoted-missing]].
> [quoted reference]
> [quoted reference]: quoted-reference-missing

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
fn diagnostics_without_external_links_skips_external_link_checks() {
    let project = temp_project("local-diagnostics");
    write_note_with_content(
        &project,
        "start.maki",
        "See [Down] and [[missing]].\n\n[Down]: https://down.example/path",
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
        r#"See [Down], <https://ok.example/docs>, and `https://code.example`.

[Down]: https://down.example/path

--- quote
See <https://down.example/path>.
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
        r#"See *[[missing]] and [Missing] and <https://down.example/path>*.

[Missing]: /missing-note"#,
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
    let project = temp_project("missing-footnote-diagnostics");
    write_note_with_content(&project, "start.maki", "Missing [^note] stays text.");

    let maki = Maki::load(&project.root).unwrap();

    assert!(maki.diagnostics_without_external_links().is_empty());
}

#[test]
fn reference_values_are_checked_once_according_to_their_shared_shape() {
    let project = temp_project("reference-value-shape-diagnostics");
    write_note_with_content(
        &project,
        "start.maki",
        r#"[raw] [prose]

[raw]: [[missing]]
[prose]: https://example.com/a b"#,
    );

    let maki = Maki::load(&project.root).unwrap();
    let broken_targets = maki
        .diagnostics_without_external_links()
        .into_iter()
        .filter_map(|diagnostic| match diagnostic.kind() {
            ProjectDiagnosticKind::BrokenLink { target } => Some(target.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(broken_targets, vec!["[[missing]]".to_string()]);

    let checked = RefCell::new(vec![]);
    maki.diagnostics_with_external_link_checker(&|target| {
        checked.borrow_mut().push(target.to_string());
        ExternalLinkCheck::Ok
    });
    assert!(checked.into_inner().is_empty());
}

#[test]
fn diagnostics_report_every_duplicate_id_declaration_with_its_line() {
    let project = temp_project("duplicate-id-diagnostics");
    write_note_with_content(
        &project,
        "start.maki",
        "First\n--^ id: shared\n\nSecond\n--^ id: shared",
    );

    let maki = Maki::load(&project.root).unwrap();
    let diagnostics = maki.diagnostics_without_external_links();
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
    let project = temp_project("raw-quote-diagnostics");
    write_note_with_content(
        &project,
        "start.maki",
        r#"--v mode: pre
> [[not-a-link]]

--v mode: text
---quote
[[also-not-a-link]]
---"#,
    );

    let maki = Maki::load(&project.root).unwrap();

    assert!(maki.diagnostics_without_external_links().is_empty());
}
