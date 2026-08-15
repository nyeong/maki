use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const BIN: &str = env!("CARGO_BIN_EXE_maki");
const REPOSITORY_GIT_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_PREFIX",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
];

struct TestServer {
    child: Child,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct TestProject {
    root: PathBuf,
}

impl Drop for TestProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture_path(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn free_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.local_addr().unwrap().port()
}

fn unique_temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("maki-{name}-{}-{nanos}", std::process::id()))
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();

    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let source = entry.path();
        let target = to.join(entry.file_name());

        if source.is_dir() {
            copy_dir(&source, &target);
        } else {
            fs::copy(&source, &target).unwrap();
        }
    }
}

fn temp_project_from_fixture(name: &str, fixture: &str) -> TestProject {
    let root = unique_temp_dir(name);
    copy_dir(&fixture_path(fixture), &root);
    TestProject { root }
}

fn temp_project(name: &str) -> TestProject {
    let root = unique_temp_dir(name);
    fs::create_dir_all(&root).unwrap();
    TestProject { root }
}

fn temp_project_with_maki_toml(name: &str) -> (TestProject, PathBuf) {
    let project = temp_project(name);
    let notes = project.root.join("notes");
    fs::create_dir_all(&notes).unwrap();
    fs::write(
        project.root.join("maki.toml"),
        "[project]\ntitle = \"Project Fixture\"\nhome = \"start\"\n",
    )
    .unwrap();
    fs::write(
        project.root.join("start.maki"),
        "--^ title: Start\n\nProject home.\n",
    )
    .unwrap();
    fs::write(
        notes.join("page.maki"),
        "--^ title: Page\n\nSee [[start]].\n",
    )
    .unwrap();

    (project, notes)
}

fn start_server(root: &Path, port: u16, index_redirect: &str) -> TestServer {
    let child = Command::new(BIN)
        .arg("serve")
        .arg(root)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--index-redirect")
        .arg(index_redirect)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let server = TestServer { child };
    wait_until_serving(port);
    server
}

fn start_server_with_project_config(root: &Path, port: u16) -> TestServer {
    let child = Command::new(BIN)
        .arg("serve")
        .arg(root)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let server = TestServer { child };
    wait_until_serving(port);
    server
}

fn start_git_server(repo: &Path, state_dir: &Path, port: u16) -> TestServer {
    let mut child = Command::new(BIN)
        .arg("serve")
        .arg("--git")
        .arg(repo)
        .arg("--branch")
        .arg("main")
        .arg("--state-dir")
        .arg(state_dir)
        .arg("--fetch-interval")
        .arg("1s")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    wait_until_child_serving(&mut child, port);
    TestServer { child }
}

fn wait_until_serving(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(5);

    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    panic!("server did not start on port {port}");
}

fn wait_until_child_serving(child: &mut Child, port: u16) {
    let deadline = Instant::now() + Duration::from_secs(5);

    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "server exited before listening on port {port} with {status}\n{}",
                read_child_output(child)
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    let _ = child.kill();
    let _ = child.wait();
    panic!(
        "server did not start on port {port}\n{}",
        read_child_output(child)
    );
}

fn read_child_output(child: &mut Child) -> String {
    let mut output = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        if !text.is_empty() {
            output.push_str("stdout:\n");
            output.push_str(&text);
        }
    }
    if let Some(mut stderr) = child.stderr.take() {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        if !text.is_empty() {
            output.push_str("stderr:\n");
            output.push_str(&text);
        }
    }
    output
}

fn wait_until_body_contains(port: u16, target: &str, expected: &str) -> HttpResponse {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut latest = None;

    while Instant::now() < deadline {
        let response = http_get(port, target);
        if response.body.contains(expected) {
            return response;
        }
        latest = Some(response.body);
        std::thread::sleep(Duration::from_millis(100));
    }

    panic!(
        "timed out waiting for {expected:?}; latest body:\n{}",
        latest.unwrap_or_default()
    );
}

fn git_is_available() -> bool {
    Command::new("git")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn run_git(repo: &Path, args: &[&str]) {
    let mut command = Command::new("git");
    command
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-C")
        .arg(repo)
        .args(args);
    for key in REPOSITORY_GIT_ENV {
        command.env_remove(key);
    }

    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn commit_git_project(repo: &Path, message: &str, body: &str) {
    fs::write(
        repo.join("maki.toml"),
        "[project]\ntitle = \"Git Fixture\"\nsource = \"docs\"\nhome = \"home\"\n",
    )
    .unwrap();
    fs::create_dir_all(repo.join("docs")).unwrap();
    fs::write(
        repo.join("docs").join("home.maki"),
        format!("--^ title: Git Home\n\n{body}\n"),
    )
    .unwrap();
    run_git(repo, &["add", "maki.toml", "docs/home.maki"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
    );
}

fn temp_git_project(name: &str) -> TestProject {
    let project = temp_project(name);
    run_git(&project.root, &["init", "--initial-branch", "main"]);
    run_git(&project.root, &["config", "user.name", "Maki Test"]);
    run_git(
        &project.root,
        &["config", "user.email", "maki-test@example.invalid"],
    );
    commit_git_project(&project.root, "initial", "Git content marker: version one");
    project
}

#[derive(Debug)]
struct HttpResponse {
    status_line: String,
    headers: String,
    body: String,
}

impl HttpResponse {
    fn assert_status(&self, expected: &str) {
        assert!(
            self.status_line.starts_with(expected),
            "expected status {expected:?}, got {:?}",
            self.status_line
        );
    }

    fn assert_header_contains(&self, expected: &str) {
        assert!(
            self.headers.contains(expected),
            "expected headers to contain {expected:?}, got {:?}",
            self.headers
        );
    }

    fn assert_body_contains(&self, expected: &str) {
        assert!(
            self.body.contains(expected),
            "expected body to contain {expected:?}"
        );
    }

    fn assert_body_excludes(&self, unexpected: &str) {
        assert!(
            !self.body.contains(unexpected),
            "expected body not to contain {unexpected:?}"
        );
    }
}

fn http_get(port: u16, target: &str) -> HttpResponse {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(
        stream,
        "GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let raw = String::from_utf8(raw).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let mut head_lines = head.lines();

    HttpResponse {
        status_line: head_lines.next().unwrap_or("").to_string(),
        headers: head_lines.collect::<Vec<_>>().join("\n"),
        body: body.to_string(),
    }
}

fn open_sse(port: u16) -> TcpStream {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    write!(
        stream,
        "GET /.maki/events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: text/event-stream\r\nConnection: keep-alive\r\n\r\n"
    )
    .unwrap();
    stream
}

fn read_until_contains(stream: &mut TcpStream, needle: &str, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    let mut output = String::new();
    let mut buffer = [0_u8; 1024];

    while Instant::now() < deadline {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                output.push_str(&String::from_utf8_lossy(&buffer[..n]));
                if output.contains(needle) {
                    return output;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => panic!("failed to read from SSE stream: {error}"),
        }
    }

    panic!("timed out waiting for {needle:?}; received:\n{output}");
}

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
        "--^ title: Home\n\nSee [[missing]] and [Ghost](ghost).\n",
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

#[test]
fn maki_serve_discovers_project_root_from_maki_toml() {
    let (_project, notes) = temp_project_with_maki_toml("serve-project-root");

    let port = free_port();
    let _server = start_server_with_project_config(&notes, port);

    let home = http_get(port, "/");
    home.assert_status("HTTP/1.1 302 Found");
    home.assert_header_contains("location: /start");

    let root_page = http_get(port, "/start");
    root_page.assert_status("HTTP/1.1 200 OK");

    let nested_page = http_get(port, "/notes/page");
    nested_page.assert_status("HTTP/1.1 200 OK");
    nested_page.assert_body_contains("<a href=\"/start\">Start</a>");
}

#[test]
fn maki_serve_uses_project_source_from_maki_toml() {
    let project = temp_project("serve-project-source");
    let docs = project.root.join("docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(
        project.root.join("maki.toml"),
        "[project]\ntitle = \"Source Fixture\"\nsource = \"docs\"\nhome = \"index\"\n",
    )
    .unwrap();
    fs::write(
        docs.join("index.maki"),
        "--^ title: Source Index\n\nProject source root.\n",
    )
    .unwrap();

    let port = free_port();
    let _server = start_server_with_project_config(&project.root, port);

    let home = http_get(port, "/");
    home.assert_status("HTTP/1.1 302 Found");
    home.assert_header_contains("location: /index");

    let index = http_get(port, "/index");
    index.assert_status("HTTP/1.1 200 OK");
    index.assert_body_contains("<title>Source Index</title>");

    let nested_path = http_get(port, "/docs/index");
    nested_path.assert_status("HTTP/1.1 404 Not Found");
}

#[test]
fn v0_fixture_serves_core_poc_behavior() {
    let port = free_port();
    let root = fixture_path("tests/fixtures/v0");
    let _server = start_server(&root, port, "/README");

    let home = http_get(port, "/");
    home.assert_status("HTTP/1.1 302 Found");
    home.assert_header_contains("location: /README");

    let page = http_get(port, "/README");
    page.assert_status("HTTP/1.1 200 OK");
    page.assert_body_contains("<title>v0 Quality Fixture</title>");
    page.assert_body_contains("<a href=\"/daily\">Daily</a>");
    page.assert_body_contains("<a href=\"/nested/홈랩\">홈랩</a>");
    page.assert_body_contains("<span class=\"broken-link\">ghost</span>");
    page.assert_body_contains("<span class=\"ambiguous-link\">ambiguous</span>");
    page.assert_body_contains(
        "<ul><li>top<ul><li>child<ul><li>grandchild</li></ul></li><li>sibling</li></ul></li><li>second top</li></ul>"
    );
    page.assert_body_contains(
        "<ol><li>supported ordered list</li><li>preserve this ordered shape</li></ol>",
    );
    page.assert_body_contains(
        "<pre><code class=\"language-html\">&lt;main&gt;\n  fixture code\n&lt;/main&gt;</code></pre>"
    );
    page.assert_body_excludes("should-not-render-property");

    let source = http_get(port, "/README.maki");
    source.assert_status("HTTP/1.1 200 OK");
    source.assert_body_contains("--^ hidden: should-not-render-property");
    source.assert_body_excludes("new EventSource");

    let unicode_page = http_get(port, "/nested/%ED%99%88%EB%9E%A9");
    unicode_page.assert_status("HTTP/1.1 200 OK");
    unicode_page.assert_body_contains("<title>홈랩</title>");

    let ignored_file = http_get(port, "/ignore");
    ignored_file.assert_status("HTTP/1.1 404 Not Found");
    ignored_file.assert_body_contains("<title>Not Found</title>");
    ignored_file.assert_body_contains("<header class=\"maki-nav\">");
    ignored_file.assert_body_contains("<a class=\"maki-home-link\" href=\"/\">/</a>");
}

#[test]
fn v1_fixture_supports_serve_options_and_live_reload() {
    let project = temp_project_from_fixture("v1-live-reload", "tests/fixtures/v1");
    let port = free_port();
    let _server = start_server(&project.root, port, "/home");

    let home = http_get(port, "/");
    home.assert_status("HTTP/1.1 302 Found");
    home.assert_header_contains("location: /home");

    let initial = http_get(port, "/home");
    initial.assert_status("HTTP/1.1 200 OK");
    initial.assert_body_contains("Initial content marker: v1-initial");
    initial.assert_body_contains("new EventSource(\"/.maki/events\")");

    let mut events = open_sse(port);
    read_until_contains(&mut events, "event: hello\ndata: ", Duration::from_secs(2));

    fs::write(
        project.root.join("home.maki"),
        "--^ title: v1 Live Reload Fixture\n\n= Status\n\nEdited content marker: v1-edited\n",
    )
    .unwrap();

    read_until_contains(&mut events, "event: reload\ndata: ", Duration::from_secs(5));

    let edited = http_get(port, "/home");
    edited.assert_body_contains("Edited content marker: v1-edited");
}

#[test]
fn git_serve_polls_commits_without_live_reload() {
    if !git_is_available() {
        return;
    }

    let repo = temp_git_project("git-serve-repo");
    let state = temp_project("git-serve-state");
    let port = free_port();
    let _server = start_git_server(&repo.root, &state.root, port);

    let initial = http_get(port, "/home");
    initial.assert_status("HTTP/1.1 200 OK");
    initial.assert_body_contains("Git content marker: version one");
    initial.assert_body_excludes("new EventSource(\"/.maki/events\")");

    commit_git_project(&repo.root, "second", "Git content marker: version two");

    let updated = wait_until_body_contains(port, "/home", "Git content marker: version two");
    updated.assert_status("HTTP/1.1 200 OK");
    updated.assert_body_excludes("new EventSource(\"/.maki/events\")");
}
