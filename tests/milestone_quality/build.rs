use super::*;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::thread;

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
        "--^ title: Home\n\nSee [[missing]] and [Ghost][].\n",
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
    assert!(stderr.contains(
        "diagnostics: 2 issue(s): 0 duplicate id(s), 1 unresolved reference(s), 1 broken link(s)"
    ));
    assert!(stderr.contains("warning: home.maki:3: broken link: missing"));
    assert!(stderr.contains("warning: home.maki:3: unresolved reference: Ghost"));
}

#[test]
fn maki_build_reports_duplicate_ids_in_project_diagnostic_summary() {
    let project = temp_project("build-duplicate-id-diagnostics");
    fs::write(project.root.join("maki.toml"), "[project]\n").unwrap();
    fs::write(
        project.root.join("home.maki"),
        "First\n--^ id: repeated\n\nSecond\n--^ id: repeated\n",
    )
    .unwrap();

    let output = Command::new(BIN)
        .arg("build")
        .arg(project.root.join("home.maki"))
        .output()
        .unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains(
        "diagnostics: 2 issue(s): 2 duplicate id(s), 0 unresolved reference(s), 0 broken link(s)"
    ));
    assert!(stderr.contains("warning: home.maki:2: duplicate id: repeated"));
    assert!(stderr.contains("warning: home.maki:5: duplicate id: repeated"));
}

#[test]
fn maki_build_is_offline_by_default() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let project = temp_project("build-offline-default");
    fs::write(project.root.join("maki.toml"), "[project]\n").unwrap();
    let page = project.root.join("home.maki");
    fs::write(&page, format!("See <http://{address}/offline>.\n")).unwrap();

    let output = Command::new(BIN).arg("build").arg(&page).output().unwrap();

    assert!(output.status.success());
    assert!(matches!(
        listener.accept(),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock
    ));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("broken external link"));
}

#[test]
fn maki_build_checks_each_external_target_once_when_explicitly_requested() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let methods = Arc::new(Mutex::new(Vec::new()));
    let server_done = Arc::clone(&done);
    let server_methods = Arc::clone(&methods);
    let server = thread::spawn(move || {
        while !server_done.load(AtomicOrdering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut request = [0_u8; 2048];
                    let length = stream.read(&mut request).unwrap();
                    let method = String::from_utf8_lossy(&request[..length])
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_string();
                    let status = if method == "HEAD" {
                        "405 Method Not Allowed"
                    } else {
                        "404 Not Found"
                    };
                    server_methods.lock().unwrap().push(method);
                    write!(
                        stream,
                        "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("external-link test server failed: {error}"),
            }
        }
    });

    let project = temp_project("build-explicit-external-links");
    fs::write(project.root.join("maki.toml"), "[project]\n").unwrap();
    let page = project.root.join("home.maki");
    fs::write(
        &page,
        format!("First <http://{address}/broken>. Again <http://{address}/broken>.\n"),
    )
    .unwrap();
    fs::write(
        project.root.join("other.maki"),
        format!("Same target in another note: <http://{address}/broken>.\n"),
    )
    .unwrap();

    let output = Command::new(BIN)
        .arg("build")
        .arg(&page)
        .arg("--check-external-links")
        .env("NO_PROXY", "127.0.0.1")
        .env("no_proxy", "127.0.0.1")
        .output()
        .unwrap();

    let standalone = temp_project("build-explicit-external-links-standalone");
    let standalone_page = standalone.root.join("standalone.maki");
    fs::write(
        &standalone_page,
        format!("Standalone <http://{address}/broken>.\n"),
    )
    .unwrap();
    let standalone_output = Command::new(BIN)
        .arg("build")
        .arg(&standalone_page)
        .arg("--check-external-links")
        .env("NO_PROXY", "127.0.0.1")
        .env("no_proxy", "127.0.0.1")
        .output()
        .unwrap();
    done.store(true, AtomicOrdering::Release);
    server.join().unwrap();

    assert!(
        output.status.success(),
        "maki build failed with stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        standalone_output.status.success(),
        "standalone maki build failed with stderr:\n{}",
        String::from_utf8_lossy(&standalone_output.stderr)
    );
    assert_eq!(
        methods.lock().unwrap().as_slice(),
        [
            "HEAD".to_string(),
            "GET".to_string(),
            "HEAD".to_string(),
            "GET".to_string(),
        ]
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.contains("<!doctype html>"));
    assert!(stderr.contains(&format!(
        "broken external link: http://{address}/broken (HTTP 404)"
    )));
    let standalone_stderr = String::from_utf8(standalone_output.stderr).unwrap();
    assert!(standalone_stderr.contains(&format!(
        "broken external link: http://{address}/broken (HTTP 404)"
    )));
}
