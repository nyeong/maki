use super::*;

fn run_check(current_dir: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .current_dir(current_dir)
        .args(arguments)
        .output()
        .unwrap()
}

fn json_report(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "check stdout was not JSON: {error}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn maki_check_defaults_to_the_discovered_project_and_has_golden_text_output() {
    let project = temp_project_from_fixture("check-project", "tests/fixtures/check/project");

    let first = run_check(&project.root, &["check"]);
    let second = run_check(&project.root, &["check", ".", "--format", "text"]);

    assert_eq!(first.status.code(), Some(1));
    assert_eq!(second.status.code(), Some(1));
    assert!(first.stderr.is_empty());
    assert!(second.stderr.is_empty());
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(
        String::from_utf8(first.stdout).unwrap(),
        include_str!("../fixtures/check/project.expected.txt")
    );
}

#[test]
fn maki_check_project_file_uses_full_context_but_reports_only_that_file() {
    let project = temp_project("check-project-file");
    fs::create_dir_all(project.root.join("docs/alpha")).unwrap();
    fs::create_dir_all(project.root.join("docs/beta")).unwrap();
    fs::write(
        project.root.join("maki.toml"),
        "[project]\nsource = \"docs\"\n",
    )
    .unwrap();
    fs::write(
        project.root.join("docs/selected.maki"),
        "[[target]] [[same]]\n",
    )
    .unwrap();
    fs::write(project.root.join("docs/target.maki"), "= Target\n").unwrap();
    fs::write(project.root.join("docs/alpha/same.maki"), "Alpha\n").unwrap();
    fs::write(project.root.join("docs/beta/same.maki"), "Beta\n").unwrap();
    fs::write(
        project.root.join("docs/other.maki"),
        "[[missing-only-in-other]]\n",
    )
    .unwrap();

    let output = run_check(
        &project.root,
        &["check", "docs/selected.maki", "--format", "json"],
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let report = json_report(&output);
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["complete"], true);
    assert_eq!(report["summary"]["total"], 1);
    assert_eq!(report["diagnostics"][0]["path"], "selected.maki");
    assert_eq!(report["diagnostics"][0]["code"], "ambiguous-note-link");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("missing-only-in-other"),
        "findings owned by non-target files must be filtered"
    );
}

#[test]
fn maki_check_standalone_file_reports_file_local_nested_ranges_in_compact_json() {
    let project = temp_project("check-standalone-file");
    let source = "😀\r\n> [missing][]\r\n> --^ invalid-property\r\n";
    fs::write(project.root.join("nested.maki"), source).unwrap();

    let output = run_check(&project.root, &["check", "nested.maki", "--format", "json"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let expected = serde_json::json!({
        "schema_version": 1,
        "complete": true,
        "diagnostics": [
            {
                "path": "nested.maki",
                "severity": "warning",
                "code": "unresolved-reference",
                "message": "unresolved reference: missing",
                "subject": { "kind": "reference", "value": "missing" },
                "range": {
                    "start": { "byte": 9, "line": 2, "column": 4 },
                    "end": { "byte": 16, "line": 2, "column": 11 },
                },
                "related": [],
            },
            {
                "path": "nested.maki",
                "severity": "warning",
                "code": "invalid-property",
                "message": "invalid property: --^ invalid-property",
                "subject": null,
                "range": {
                    "start": { "byte": 23, "line": 3, "column": 3 },
                    "end": { "byte": 43, "line": 3, "column": 23 },
                },
                "related": [],
            },
        ],
        "summary": {
            "total": 2,
            "errors": 0,
            "warnings": 2,
            "information": 0,
            "hints": 0,
        },
    });
    assert_eq!(json_report(&output), expected);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{}\n", expected)
    );
}

#[test]
fn maki_check_standalone_directory_analyzes_all_notes_together() {
    let project = temp_project("check-standalone-directory");
    fs::write(project.root.join("a.maki"), "See [[b]].\n").unwrap();
    fs::write(project.root.join("b.maki"), "See [[missing]].\n").unwrap();

    let output = run_check(&project.root, &["check", "."]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "b.maki:1:7: warning[broken-note-link]: broken note link: missing\n"
    );
}

#[cfg(unix)]
#[test]
fn maki_check_preserves_literal_backslashes_in_unix_paths() {
    let project = temp_project("check-backslash-path");
    let path = r"back\slash.maki";
    fs::write(project.root.join(path), "--^ invalid-property\n").unwrap();

    let output = run_check(&project.root, &["check", path, "--format", "json"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json_report(&output)["diagnostics"][0]["path"], path);
}

#[cfg(unix)]
#[test]
fn maki_check_rejects_non_utf8_paths_without_lossy_output() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let project = temp_project("check-non-utf8-path");
    let path = PathBuf::from(OsString::from_vec(b"invalid-\xff.maki".to_vec()));
    fs::write(project.root.join(&path), "--^ invalid-property\n").unwrap();

    let output = Command::new(BIN)
        .current_dir(&project.root)
        .arg("check")
        .arg(&path)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "command-line arguments must be valid UTF-8\n"
    );
}

#[cfg(unix)]
#[test]
fn maki_check_keeps_findings_when_a_non_utf8_source_is_unavailable() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let project = temp_project("check-non-utf8-unavailable");
    fs::write(project.root.join("maki.toml"), "[project]\n").unwrap();
    fs::write(project.root.join("readable.maki"), "--^ invalid-property\n").unwrap();
    let unavailable = OsString::from_vec(b"unavailable-\xff.maki".to_vec());
    fs::write(project.root.join(unavailable), [0xff, 0xfe]).unwrap();

    let output = run_check(&project.root, &["check", ".", "--format", "json"]);

    assert_eq!(output.status.code(), Some(2));
    let report = json_report(&output);
    assert_eq!(report["complete"], false);
    assert_eq!(report["summary"]["total"], 1);
    assert_eq!(report["diagnostics"][0]["path"], "readable.maki");
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to read Maki source"));
}

#[cfg(unix)]
#[test]
fn maki_check_rejects_non_utf8_diagnostic_owner_paths() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let project = temp_project("check-non-utf8-diagnostic");
    fs::write(project.root.join("maki.toml"), "[project]\n").unwrap();
    let path = OsString::from_vec(b"diagnostic-\xff.maki".to_vec());
    fs::write(project.root.join(path), "--^ invalid-property\n").unwrap();

    let output = run_check(&project.root, &["check", ".", "--format", "json"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot report non-UTF-8 path"));
}

#[cfg(unix)]
#[test]
fn maki_check_preserves_project_file_symlink_entry_names() {
    use std::os::unix::fs::symlink;

    let project = temp_project("check-file-symlinks");
    let external = temp_project("check-file-symlinks-external");
    let source_root = project.root.join("docs");
    fs::create_dir_all(&source_root).unwrap();
    fs::write(
        project.root.join("maki.toml"),
        "[project]\nsource = \"docs\"\n",
    )
    .unwrap();

    fs::write(source_root.join("internal.maki"), "--^ invalid-property\n").unwrap();
    symlink("internal.maki", source_root.join("internal-alias.maki")).unwrap();

    let external_target = external.root.join("external.maki");
    fs::write(&external_target, "--^ invalid-property\n").unwrap();
    symlink(&external_target, source_root.join("external-alias.maki")).unwrap();

    for alias in ["internal-alias.maki", "external-alias.maki"] {
        let target = format!("docs/{alias}");
        let output = run_check(&project.root, &["check", &target, "--format", "json"]);

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stderr.is_empty());
        assert_eq!(json_report(&output)["diagnostics"][0]["path"], alias);
    }
}

#[test]
fn maki_check_analyzes_explicit_project_excluded_files_as_standalone() {
    let project = temp_project("check-excluded-file");
    let source_root = project.root.join("docs");
    fs::create_dir_all(&source_root).unwrap();
    fs::write(
        project.root.join("maki.toml"),
        "[project]\nsource = \"docs\"\n",
    )
    .unwrap();
    fs::write(source_root.join(".hidden.maki"), "--^ invalid-property\n").unwrap();

    let output = run_check(
        &project.root,
        &["check", "docs/.hidden.maki", "--format", "json"],
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        json_report(&output)["diagnostics"][0]["code"],
        "invalid-property"
    );
}

#[test]
fn maki_check_distinguishes_clean_findings_usage_and_operational_failures() {
    let project = temp_project("check-exit-codes");
    fs::write(project.root.join("clean.maki"), "= Clean\n").unwrap();

    let clean = run_check(&project.root, &["check", "clean.maki"]);
    assert_eq!(clean.status.code(), Some(0));
    assert!(clean.stdout.is_empty());
    assert!(clean.stderr.is_empty());

    let usage = run_check(&project.root, &["check", "--format", "yaml"]);
    assert_eq!(usage.status.code(), Some(2));
    assert!(usage.stdout.is_empty());
    assert_eq!(
        String::from_utf8(usage.stderr).unwrap(),
        "Invalid check format: yaml\n"
    );

    let missing = run_check(&project.root, &["check", "missing"]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(missing.stdout.is_empty());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("failed to inspect missing"));

    let invalid = project.root.join("not-maki.txt");
    fs::write(&invalid, "text\n").unwrap();
    let invalid = run_check(&project.root, &["check", "not-maki.txt"]);
    assert_eq!(invalid.status.code(), Some(2));
    assert!(invalid.stdout.is_empty());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("expected a .maki file"));
}

#[test]
fn maki_check_keeps_readable_findings_when_a_project_read_is_incomplete() {
    let project = temp_project("check-incomplete");
    fs::write(project.root.join("maki.toml"), "[project]\n").unwrap();
    let readable = project.root.join("a.maki");
    let source = "--^ invalid-property\n";
    fs::write(&readable, source).unwrap();
    fs::write(project.root.join("bad.maki"), [0xff, 0xfe]).unwrap();

    let output = run_check(&project.root, &["check", "--format", "json"]);

    assert_eq!(output.status.code(), Some(2));
    let report = json_report(&output);
    assert_eq!(report["complete"], false);
    assert_eq!(report["summary"]["total"], 1);
    assert_eq!(report["diagnostics"][0]["code"], "invalid-property");
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "failed to read Maki source: bad.maki\n"
    );
    assert_eq!(fs::read_to_string(readable).unwrap(), source);
    assert_eq!(
        fs::read(project.root.join("bad.maki")).unwrap(),
        [0xff, 0xfe]
    );
}
