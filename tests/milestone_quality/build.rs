use super::*;

#[test]
fn maki_build_reports_parser_warnings_to_stderr() {
    let project = temp_project("build-warnings");
    let file = project.root.join("warning.maki");
    fs::write(
        &file,
        "--^ invalid-property\n--^ title: Warning Fixture\n\n= Heading\n\n1. fallback\n",
    )
    .unwrap();

    let output = Command::new(BIN).arg("build").arg(&file).output().unwrap();

    assert!(
        output.status.success(),
        "maki build failed with stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(stdout.contains("<title>Warning Fixture</title>"));
    assert!(!stdout.contains("warning:"));
    assert!(stderr.contains(&format!(
        "warning: {}:1: invalid property: --^ invalid-property",
        file.display()
    )));
    assert!(stdout.contains("<ol><li>fallback</li></ol>"));
    assert!(!stderr.contains("unsupported numbered block"));
}

#[test]
fn maki_build_discovers_project_root_from_maki_toml() {
    let (_project, notes) = temp_project_with_maki_toml("build-project-root");

    let output = Command::new(BIN)
        .current_dir(&notes)
        .arg("build")
        .arg("page.maki")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "maki build failed with stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("<a href=\"/start\">Start</a>"));
}

#[test]
fn maki_build_reports_project_diagnostic_summary_to_stderr() {
    let project = temp_project("build-project-diagnostics");
    fs::write(
        project.root.join("maki.toml"),
        "[project]\ntitle = \"Diagnostics Fixture\"\n",
    )
    .unwrap();
    fs::write(
        project.root.join("home.maki"),
        "--^ title: Home\n\nSee [[missing]] and [Ghost].\n\n[Ghost]: ghost\n",
    )
    .unwrap();

    let output = Command::new(BIN)
        .arg("build")
        .arg(project.root.join("home.maki"))
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "maki build failed with stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(stdout.contains("<title>Home</title>"));
    assert!(stderr.contains("diagnostics: 2 issue(s): 2 broken link(s)"));
    assert!(stderr.contains("warning: home.maki: broken link: missing"));
    assert!(stderr.contains("warning: home.maki: broken link: ghost"));
}
