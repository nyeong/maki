use std::{
    fs,
    io::{self, Read, Write},
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

fn start_server_with_metrics(root: &Path, port: u16, metrics_port: u16) -> TestServer {
    let mut child = Command::new(BIN)
        .arg("serve")
        .arg(root)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--metrics")
        .arg(format!("127.0.0.1:{metrics_port}"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    wait_until_child_serving(&mut child, port);
    wait_until_child_serving(&mut child, metrics_port);
    TestServer { child }
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
    run_git_with_env(repo, args, &[]);
}

fn run_git_with_env(repo: &Path, args: &[&str], env: &[(&str, &str)]) {
    let mut command = Command::new("git");
    command
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-C")
        .arg(repo)
        .args(args);
    command.envs(env.iter().copied());
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

fn commit_git_project_at(repo: &Path, message: &str, timestamp: &str) {
    run_git(repo, &["add", "."]);
    run_git_with_env(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
        &[
            ("GIT_AUTHOR_DATE", timestamp),
            ("GIT_COMMITTER_DATE", timestamp),
        ],
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
    commit_git_project_at(repo, message, "2001-01-01T00:00:00+0000");
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

fn assert_body_order(body: &str, needles: &[&str]) {
    let mut previous = None;
    for needle in needles {
        let position = body
            .find(needle)
            .unwrap_or_else(|| panic!("missing {needle:?} in body:\n{body}"));
        if let Some(previous) = previous {
            assert!(previous < position, "{needle:?} is out of order");
        }
        previous = Some(position);
    }
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
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut latest_error = String::new();

    while Instant::now() < deadline {
        match try_http_get(port, target) {
            Ok(response) if !response.status_line.is_empty() => return response,
            Ok(response) => {
                latest_error = format!("empty status line in response: {response:?}");
            }
            Err(error) => {
                latest_error = error.to_string();
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    panic!("GET {target} on port {port} failed: {latest_error}");
}

fn try_http_get(port: u16, target: &str) -> io::Result<HttpResponse> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    write!(
        stream,
        "GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let raw = String::from_utf8(raw).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("response was not UTF-8: {error}"),
        )
    })?;
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let mut head_lines = head.lines();

    Ok(HttpResponse {
        status_line: head_lines.next().unwrap_or("").to_string(),
        headers: head_lines.collect::<Vec<_>>().join("\n"),
        body: body.to_string(),
    })
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

#[path = "milestone_quality/build.rs"]
mod build;
#[path = "milestone_quality/git.rs"]
mod git;
#[path = "milestone_quality/metrics.rs"]
mod metrics;
#[path = "milestone_quality/serve.rs"]
mod serve;
