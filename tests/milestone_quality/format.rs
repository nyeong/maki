use super::*;

fn run_with_stdin(arguments: &[&str], input: &str) -> std::process::Output {
    let mut child = Command::new(BIN)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn maki_fmt_streams_stdin_to_stdout_without_changing_line_endings() {
    let source = "  --^   title  :  Draft  \r\n  == Heading";
    let expected = "--^ title: Draft\r\n== Heading";

    let output = run_with_stdin(&["fmt", "-"], source);

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    assert!(output.stderr.is_empty());

    let check = run_with_stdin(&["fmt", "--check", "-"], source);
    assert_eq!(check.status.code(), Some(1));
    assert!(check.stdout.is_empty());
    assert_eq!(
        String::from_utf8(check.stderr).unwrap(),
        "would reformat: <stdin>\n"
    );

    let clean_check = run_with_stdin(&["fmt", "-", "--check"], expected);
    assert!(clean_check.status.success());
    assert!(clean_check.stdout.is_empty());
    assert!(clean_check.stderr.is_empty());
}

#[test]
fn maki_fmt_replaces_one_file_atomically_and_preserves_permissions() {
    let project = temp_project("format-file");
    let file = project.root.join("page.maki");
    fs::write(&file, "  --^   title  :  Draft  \n  == Heading\n").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
    }

    let output = Command::new(BIN).arg("fmt").arg(&file).output().unwrap();

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!("formatted: {}\n", file.display())
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "--^ title: Draft\n== Heading\n"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    let second = Command::new(BIN).arg("fmt").arg(&file).output().unwrap();
    assert!(second.status.success());
    assert!(second.stdout.is_empty());
    assert!(second.stderr.is_empty());
}

#[test]
fn maki_fmt_check_defaults_to_the_configured_project_source() {
    let project = temp_project("format-project");
    fs::create_dir_all(project.root.join("docs/nested")).unwrap();
    fs::write(
        project.root.join("maki.toml"),
        "[project]\nsource = \"docs\"\n",
    )
    .unwrap();
    let first = project.root.join("docs/a.maki");
    let nested = project.root.join("docs/nested/z.maki");
    let outside = project.root.join("outside.maki");
    let unformatted = "  --^  title : Draft  \n= Heading\n";
    fs::write(&first, unformatted).unwrap();
    fs::write(&nested, unformatted).unwrap();
    fs::write(&outside, unformatted).unwrap();

    let check = Command::new(BIN)
        .current_dir(&project.root)
        .args(["fmt", "--check"])
        .output()
        .unwrap();

    assert_eq!(check.status.code(), Some(1));
    assert!(check.stdout.is_empty());
    let stderr = String::from_utf8(check.stderr).unwrap();
    assert!(stderr.contains(&format!(
        "would reformat: {}",
        first.canonicalize().unwrap().display()
    )));
    assert!(stderr.contains(&format!(
        "would reformat: {}",
        nested.canonicalize().unwrap().display()
    )));
    assert!(!stderr.contains("outside.maki"));
    assert_eq!(fs::read_to_string(&first).unwrap(), unformatted);

    let format = Command::new(BIN)
        .current_dir(project.root.join("docs"))
        .args(["fmt", "nested"])
        .output()
        .unwrap();

    assert!(format.status.success());
    assert!(format.stdout.is_empty());
    assert_eq!(
        fs::read_to_string(&first).unwrap(),
        "--^ title: Draft\n= Heading\n"
    );
    assert_eq!(
        fs::read_to_string(&nested).unwrap(),
        "--^ title: Draft\n= Heading\n"
    );
    assert_eq!(fs::read_to_string(&outside).unwrap(), unformatted);
}

#[test]
fn maki_fmt_preflights_the_whole_directory_before_writing() {
    let project = temp_project("format-preflight");
    let valid = project.root.join("a.maki");
    let malformed = project.root.join("z.maki");
    let unformatted = "  --^  title : Draft  \n= Heading\n";
    fs::write(&valid, unformatted).unwrap();
    fs::write(&malformed, "---code\nraw\n").unwrap();

    let output = Command::new(BIN)
        .arg("fmt")
        .arg(&project.root)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains(&malformed.display().to_string()));
    assert!(stderr.contains("unclosed container"));
    assert_eq!(fs::read_to_string(&valid).unwrap(), unformatted);
    assert!(fs::read_dir(&project.root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".maki-fmt-")
    }));
}

#[test]
fn maki_fmt_rejects_non_maki_files() {
    let project = temp_project("format-extension");
    let file = project.root.join("page.txt");
    fs::write(&file, "= Heading\n").unwrap();

    let output = Command::new(BIN).arg("fmt").arg(&file).output().unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("expected a .maki file")
    );
    assert_eq!(fs::read_to_string(file).unwrap(), "= Heading\n");
}

#[cfg(unix)]
#[test]
fn maki_fmt_rejects_non_regular_maki_paths() {
    let project = temp_project("format-non-regular");
    let fifo = project.root.join("page.maki");
    // A FIFO exercises the same rejection without Unix socket path-length limits.
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );

    let output = Command::new(BIN).arg("fmt").arg(&fifo).output().unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("expected a regular file")
    );
}

#[cfg(unix)]
#[test]
fn maki_fmt_rejects_symbolic_link_files_without_touching_the_target() {
    use std::os::unix::fs::symlink;

    let project = temp_project("format-symlink");
    let target = project.root.join("target.maki");
    let link = project.root.join("link.maki");
    let source = "  --^  title : Draft  \n= Heading\n";
    fs::write(&target, source).unwrap();
    symlink(&target, &link).unwrap();

    let output = Command::new(BIN).arg("fmt").arg(&link).output().unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("symbolic links")
    );
    assert_eq!(fs::read_to_string(target).unwrap(), source);
}

#[cfg(unix)]
#[test]
fn maki_fmt_rejects_a_configured_source_outside_the_project() {
    use std::os::unix::fs::symlink;

    let project = temp_project("format-source-boundary");
    let outside = temp_project("format-source-outside");
    let page = outside.root.join("page.maki");
    let source = "  --^  title : Draft  \n= Heading\n";
    fs::write(
        project.root.join("maki.toml"),
        "[project]\nsource = \"docs\"\n",
    )
    .unwrap();
    fs::write(&page, source).unwrap();
    symlink(&outside.root, project.root.join("docs")).unwrap();

    let output = Command::new(BIN)
        .arg("fmt")
        .arg(&project.root)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("resolves outside project")
    );
    assert_eq!(fs::read_to_string(page).unwrap(), source);
}
