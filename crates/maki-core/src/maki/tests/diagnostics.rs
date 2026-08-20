use super::*;

#[test]
fn diagnostics_collect_parse_warnings_and_link_resolution_issues() {
    let project = temp_project("diagnostics");
    write_note_with_content(
        &project,
        "start.maki",
        r#"--^ invalid-property

See [[missing]], [Ghost](ghost), and [[same]].

> See [[quoted-missing]].

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
        "See [Down](https://down.example/path) and [[missing]].",
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
        r#"See [Down](https://down.example/path), https://ok.example/docs, and `https://code.example`.

--- quote
See https://down.example/path.
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
        r#"See *[[missing]] and [Missing](/missing-note) and [Down](https://down.example/path)*."#,
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
            ProjectDiagnosticKind::BrokenLink { target } if target == "missing-note"
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
