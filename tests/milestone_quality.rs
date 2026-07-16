use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const BIN: &str = env!("CARGO_BIN_EXE_maki");

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
    assert!(stderr.contains(&format!(
        "warning: {}:6: unsupported numbered block rendered as fallback: 1. fallback",
        file.display()
    )));
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
        "<pre><code class=\"language-maki\">1. unsupported numbered fallback\n2. preserve this raw shape</code></pre>"
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
    let hello = read_until_contains(&mut events, "event: hello", Duration::from_secs(2));
    assert!(hello.contains("data: "));

    fs::write(
        project.root.join("home.maki"),
        "--^ title: v1 Live Reload Fixture\n\n= Status\n\nEdited content marker: v1-edited\n",
    )
    .unwrap();

    let reload = read_until_contains(&mut events, "event: reload", Duration::from_secs(5));
    assert!(reload.contains("data: "));

    let edited = http_get(port, "/home");
    edited.assert_body_contains("Edited content marker: v1-edited");
}
