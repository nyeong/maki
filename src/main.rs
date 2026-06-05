use std::collections::HashMap;
use std::fmt::Display;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use percent_encoding::percent_decode_str;

#[derive(Debug, PartialEq)]
enum Command {
    Serve { root: PathBuf },
}

#[derive(Debug, PartialEq)]
enum HttpStatus {
    Ok,                  // 200
    MovedPermanently,    // 301
    Found,               // 302
    NotFound,            // 404
    BadRequest,          // 400
    InternalServerError, // 500
}

#[derive(Debug, PartialEq)]
enum HttpMethod {
    Get,
}

#[derive(Debug, PartialEq)]
struct MakiConfig {
    home_mode: HomeMode,
    publish_policy: PublishPolicy,
}

#[derive(Debug, PartialEq)]
enum PublishPolicy {
    PublishAll,
    // TODO: TaggedOnly: publish 설정한 파일만 접근 가능하게 하기,
}

#[derive(Debug, PartialEq)]
enum HomeMode {
    // Listing,
    Redirect(String),
}

impl Default for HttpHeaders {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for MakiConfig {
    fn default() -> Self {
        Self {
            home_mode: HomeMode::Redirect("/n/README".to_string()),
            publish_policy: PublishPolicy::PublishAll,
        }
    }
}

impl FromStr for HttpMethod {
    type Err = RunError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_method(s)
    }
}

fn parse_method(method: &str) -> Result<HttpMethod, RunError> {
    match method {
        "GET" => Ok(HttpMethod::Get),
        _ => Err(RunError::RequestParseError),
    }
}

fn parse_protocol(protocol: &str) -> Result<HttpVersion, RunError> {
    match protocol {
        "HTTP/1.1" => Ok(HttpVersion::Http1_1),
        _ => Err(RunError::RequestParseError),
    }
}

struct Maki {
    root: PathBuf,       // canonical absolute path
    files: Vec<PathBuf>, // root-relative markdown paths
    config: MakiConfig,
}

#[derive(Debug, PartialEq)]
struct HttpResponse {
    status: HttpStatus,
    version: HttpVersion,
    headers: HttpHeaders,
    body: Vec<u8>,
}

impl Display for HttpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpStatus::Ok => write!(f, "200 OK"),
            HttpStatus::NotFound => write!(f, "404 Not Found"),
            HttpStatus::Found => write!(f, "302 Found"),
            HttpStatus::MovedPermanently => write!(f, "301 Moved Permanently"),
            HttpStatus::InternalServerError => write!(f, "500 Internal Server Error"),
            HttpStatus::BadRequest => write!(f, "400 Bad Request"),
        }
    }
}

impl Display for HttpHeaders {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (key, value) in &self.0 {
            write!(f, "{}: {}\r\n", key, value)?;
        }
        Ok(())
    }
}

impl HttpResponse {
    fn new(status: HttpStatus) -> Self {
        HttpResponse {
            status,
            version: HttpVersion::Http1_1,
            headers: HttpHeaders::new(),
            body: vec![],
        }
        .set_header("Connection", "close")
    }

    fn set_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    fn set_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self.headers
            .insert("Content-Length".to_string(), self.body.len().to_string());
        self
    }

    fn get_status_line(&self) -> String {
        format!("{} {}", self.version, self.status)
    }

    fn to_raw(&self) -> Vec<u8> {
        let mut raw = Vec::new();
        let status_line = self.get_status_line();

        raw.extend_from_slice(status_line.as_bytes());
        raw.extend_from_slice(b"\r\n");

        raw.extend_from_slice(self.headers.to_string().as_bytes());
        raw.extend_from_slice(b"\r\n");

        if !&self.body.is_empty() {
            raw.extend_from_slice(&self.body);
        }

        raw
    }
}

impl FromStr for HttpVersion {
    type Err = RunError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_protocol(s)
    }
}

#[derive(Debug, PartialEq)]
enum HttpVersion {
    Http1_1,
}

impl Display for HttpVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpVersion::Http1_1 => write!(f, "HTTP/1.1"),
        }
    }
}

#[derive(Debug, PartialEq)]
struct HttpHeaders(HashMap<String, String>);

impl HttpHeaders {
    fn new() -> Self {
        Self(HashMap::new())
    }

    fn insert(&mut self, key: String, value: String) {
        self.0.insert(key, value);
    }

    fn get(&self, key: &str) -> Option<&String> {
        self.0.get(key)
    }
}

#[derive(Debug, PartialEq)]
struct HttpRequest {
    method: HttpMethod,
    target: String,
    version: HttpVersion,
    headers: HttpHeaders,
    body: Option<String>,
}

/// Parses an HTTP request-line.
fn parse_request_line(line: &str) -> Result<(HttpMethod, String, HttpVersion), RunError> {
    let mut parts = line.split_whitespace();
    let method = parts
        .next()
        .ok_or(RunError::RequestParseError)?
        .parse::<HttpMethod>()?;
    let target = parts.next().ok_or(RunError::RequestParseError)?.to_string();
    let version = parts
        .next()
        .ok_or(RunError::RequestParseError)?
        .parse::<HttpVersion>()?;
    if parts.next().is_some() {
        return Err(RunError::RequestParseError);
    }
    Ok((method, target, version))
}

fn parse_request_headers<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
) -> Result<HttpHeaders, RunError> {
    let mut headers = HttpHeaders::new();

    for line in lines {
        if line.is_empty() {
            break;
        }
        let (key, value) = line.split_once(':').ok_or(RunError::RequestParseError)?;

        if key.trim() != key || key.is_empty() {
            return Err(RunError::RequestParseError);
        }

        let key = key.to_ascii_lowercase();
        let value = value.trim().to_string();
        headers.insert(key, value);
    }

    Ok(headers)
}

/// Parses a raw HTTP request string into a [`HttpRequest`] struct.
fn parse_request(request: &str) -> Result<HttpRequest, RunError> {
    let mut lines = request.lines();
    let first_line = lines.next().ok_or(RunError::RequestParseError)?;

    let (method, target, version) = parse_request_line(first_line)?;

    let headers = parse_request_headers(&mut lines)?;

    Ok(HttpRequest {
        method,
        target,
        version,
        headers,
        body: None,
    })
}

const MAX_REQUEST_HEAD_SIZE: usize = 16 * 1024;

fn read_request_head(stream: &mut impl Read) -> Result<Vec<u8>, RunError> {
    // TODO: 최적화 가능
    // 매 요청마다 버퍼, Vec 새로 만들지 않고 만들어진 것 쓰기
    // 단, keep-alive 지원할 경우, 그에 대해 고려해야함
    let mut request = Vec::with_capacity(4096);
    let mut buffer = [0u8; 1024];
    loop {
        let bytes_read = stream
            .read(&mut buffer)
            .map_err(|source| RunError::IoError { source })?;

        if bytes_read == 0 {
            return Err(RunError::RequestParseError);
        }

        request.extend_from_slice(&buffer[..bytes_read]);

        // TODO: 헤더 경계 찾기 최적화 가능
        // 전체를 훑지 말고 최근에 받은 내용 중에서 훑기
        // buffer만 보면 안됨. \r\n | \r\n 이렇게 끊어서 올 수도 있으니까.
        if request.windows(4).any(|w| w == b"\r\n\r\n") {
            return Ok(request);
        }

        if request.len() > MAX_REQUEST_HEAD_SIZE {
            return Err(RunError::RequestParseError);
        }
    }
}

fn serve_http(maki: &Maki) -> Result<(), RunError> {
    let listener =
        TcpListener::bind("127.0.0.1:4000").map_err(|source| RunError::IoError { source })?;

    println!("Listening on http://localhost:4000");

    for stream in listener.incoming() {
        let mut stream = stream.map_err(|source| RunError::IoError { source })?;
        let raw_request = read_request_head(&mut stream)?;
        // TODO: header만 잘라서 먼저 utf8로 변환하기
        let request = String::from_utf8_lossy(&raw_request);
        let request = parse_request(&request)?;

        let response = maki.handle_request(&request)?;
        let http_response = response.to_raw();
        stream
            .write_all(&http_response)
            .map_err(|source| RunError::IoError { source })?;
    }

    Ok(())
}

fn get_relative_path(root: &Path, path: &Path) -> Result<PathBuf, RunError> {
    path.strip_prefix(root)
        .map_err(|source| RunError::InvalidMarkdownPath {
            path: path.to_path_buf(),
            source,
        })
        .map(Path::to_path_buf)
}

#[derive(Debug, PartialEq)]
enum MakiRoute {
    Home,
    NotFound,
    NotePage(PathBuf),
    NoteSource(PathBuf),
}

impl Maki {
    fn handle_request(&self, request: &HttpRequest) -> Result<HttpResponse, RunError> {
        match self.resolve_route(request.target.as_str()) {
            Ok(MakiRoute::NotePage(path)) => Ok(HttpResponse::new(HttpStatus::Ok)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(self.render_html(&path)?)),
            Ok(MakiRoute::NoteSource(path)) => Ok(HttpResponse::new(HttpStatus::Ok)
                .set_header("Content-Type", "text/plain; charset=utf-8")
                .set_body(self.get_raw_content(&path)?)),
            Ok(MakiRoute::Home) => match &self.config.home_mode {
                HomeMode::Redirect(path) => Ok(HttpResponse::new(HttpStatus::Found)
                    .set_header("Location", path)
                    .set_header("Content-Type", "text/plain; charset=utf-8")
                    .set_body(path.as_bytes())),
            },
            Ok(MakiRoute::NotFound) => Ok(HttpResponse::new(HttpStatus::NotFound)
                .set_header("Content-Type", "text/plain; charset=utf-8")
                .set_body("Not Found".to_string())),
            Err(RunError::BadRequest) => Ok(HttpResponse::new(HttpStatus::BadRequest)
                .set_header("Content-Type", "text/plain; charset=utf-8")
                .set_body("Bad Request".to_string())),
            Err(e) => Ok(HttpResponse::new(HttpStatus::InternalServerError)
                .set_header("Content-Type", "text/plain; charset=utf-8")
                .set_body(e.to_string())),
        }
    }
    fn get_raw_content(&self, file: &Path) -> Result<String, RunError> {
        let file = self.root.join(file);

        if !file.exists() || !file.is_file() {
            return Err(RunError::IoError {
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "file not found"),
            });
        }

        std::fs::read_to_string(&file).map_err(|source| RunError::IoError { source })
    }

    // root: absolute or relative to the project directory
    fn load(root: &Path, config: MakiConfig) -> Result<Self, RunError> {
        if !root.exists() {
            return Err(RunError::RootNotFound(root.to_path_buf()));
        }
        if !root.is_dir() {
            return Err(RunError::RootNotDirectory(root.to_path_buf()));
        }

        let root = std::fs::canonicalize(root).map_err(|source| RunError::IoError { source })?;

        let files = list_markdown_files(&root)?
            .into_iter()
            .map(|path| get_relative_path(&root, &path))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            root,
            files,
            config,
        })
    }

    // TODO: render markdown to HTML
    fn render_html(&self, file: &Path) -> Result<String, RunError> {
        let file = self.root.join(file);
        if !file.exists() {
            return Err(RunError::IoError {
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "file not found"),
            });
        }
        if !file.is_file() {
            return Err(RunError::IoError {
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "not a file"),
            });
        }

        let mut html =
            String::from("<!doctype html><html><head><meta charset=\"utf-8\"></head><body>");
        let content =
            std::fs::read_to_string(&file).map_err(|source| RunError::IoError { source })?;
        html.push_str(&content);
        html.push_str("</body></html>");
        Ok(html)
    }

    /// Resolves a note path relative to the root directory.
    /// # Example
    /// ```
    /// maki.resolve_note_route("maki.md"); // => MakiRoute::NoteSource("maki.md")
    /// maki.resolve_note_route("maki"); // => MakiRoute::NotePage("maki.md")
    /// ```
    fn resolve_note_route(&self, target: &str) -> Result<MakiRoute, RunError> {
        let is_source = target.ends_with(".md");

        let relative_path = if is_source {
            PathBuf::from(target)
        } else {
            PathBuf::from(format!("{target}.md"))
        };

        if relative_path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(RunError::BadRequest);
        }

        if !self.files.contains(&relative_path) {
            return Ok(MakiRoute::NotFound);
        }

        match is_source {
            true => Ok(MakiRoute::NoteSource(relative_path)),
            false => Ok(MakiRoute::NotePage(relative_path)),
        }
    }

    /// Resolves a page path relative to the root directory.
    /// # Example
    /// ```
    /// maki.resolve_route("/n/maki"); // => MakiRoute::NotePage("maki.md")
    /// ```
    fn resolve_route(&self, target: &str) -> Result<MakiRoute, RunError> {
        let target = target
            .strip_prefix('/')
            .ok_or(RunError::RequestParseError)?;

        if target.is_empty() {
            return Ok(MakiRoute::Home);
        }

        let target = percent_decode_str(target)
            .decode_utf8()
            .map_err(|_e| RunError::BadRequest)?
            .to_string();

        if let Some(note_target) = target.strip_prefix("n/") {
            return self.resolve_note_route(note_target);
        }

        Ok(MakiRoute::NotFound)
    }
}

fn main() {
    let args = std::env::args().collect::<Vec<String>>();

    let command = parse_args(&args).unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(2);
    });

    run_command(command).unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(1);
    })
}

#[derive(Debug)]
enum RunError {
    RootNotFound(PathBuf),
    RootNotDirectory(PathBuf),
    ReadDirectoryFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    InvalidMarkdownPath {
        path: PathBuf,
        source: std::path::StripPrefixError,
    },
    IoError {
        source: std::io::Error,
    },
    RequestParseError,
    BadRequest,
}

impl Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::RootNotFound(path) => write!(f, "Root not found: {}", path.display()),
            RunError::RootNotDirectory(path) => {
                write!(f, "Root not a directory: {}", path.display())
            }
            RunError::InvalidMarkdownPath { path, source } => {
                write!(f, "Invalid markdown path {}: {}", path.display(), source)
            }
            RunError::ReadDirectoryFailed { path, source } => {
                write!(f, "Failed to read directory {}: {}", path.display(), source)
            }
            RunError::BadRequest => write!(f, "Bad request"),
            RunError::IoError { source } => write!(f, "IO error: {}", source),
            RunError::RequestParseError => write!(f, "Request parse error"),
        }
    }
}

/// Lists all markdown files in the given directory.
/// Currently, it does not recursively search subdirectories.
fn list_markdown_files(root: &Path) -> Result<Vec<PathBuf>, RunError> {
    let entries = std::fs::read_dir(root).map_err(|source| RunError::ReadDirectoryFailed {
        path: root.to_path_buf(),
        source,
    })?;

    let mut files = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|source| RunError::ReadDirectoryFailed {
            path: root.to_path_buf(),
            source,
        })?;

        let path = entry.path();

        if path.is_file() && path.extension().is_some_and(|ext| ext == "md") {
            files.push(path);
        }
    }

    files.sort();
    Ok(files)
}

fn run_serve(root: PathBuf) -> Result<(), RunError> {
    let maki = Maki::load(&root, MakiConfig::default())?;
    println!("Found {} markdown files", maki.files.len());
    for file in &maki.files {
        println!("- {}", file.display());
    }
    serve_http(&maki)
}

fn run_command(command: Command) -> Result<(), RunError> {
    match command {
        Command::Serve { root } => run_serve(root),
    }
}

#[derive(Debug, PartialEq)]
enum CliError {
    MissingCommand,
    UnknownCommand(String),
}

impl Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::UnknownCommand(s) => write!(f, "Unknown command: {}", s),
            CliError::MissingCommand => write!(f, "Missing command"),
        }
    }
}

fn parse_args(args: &[String]) -> Result<Command, CliError> {
    // 0 is the binary name
    let command = args.get(1).ok_or(CliError::MissingCommand)?;

    match command.as_str() {
        "serve" => {
            let root = args.get(2).map_or(".", String::as_str);

            Ok(Command::Serve {
                root: PathBuf::from(root),
            })
        }
        other => Err(CliError::UnknownCommand(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_run_serve_not_exists() {
        let path = PathBuf::from("./tests/not-exists");

        let error = run_command(Command::Serve { root: path.clone() }).unwrap_err();

        match error {
            RunError::RootNotFound(realpath) => assert_eq!(realpath, path),
            _ => panic!("Unexpected error: {:?}", error),
        }
    }

    #[test]
    fn test_run_serve_not_directory() {
        let path = PathBuf::from("./tests/fixtures/basic-maki-project/README.md");

        let error = run_command(Command::Serve { root: path.clone() }).unwrap_err();

        match error {
            RunError::RootNotDirectory(realpath) => assert_eq!(realpath, path),
            _ => panic!("Unexpected error: {:?}", error),
        }
    }

    #[test]
    fn test_parse_serve_command() {
        assert_eq!(
            parse_args(&args(&["maki", "serve", "path/to/markdown"])),
            Ok(Command::Serve {
                root: PathBuf::from("path/to/markdown"),
            })
        )
    }

    #[test]
    fn test_parse_unknown_command() {
        assert_eq!(
            parse_args(&args(&["maki", "unknown"])),
            Err(CliError::UnknownCommand("unknown".to_string()))
        )
    }

    #[test]
    fn test_parse_missing_command() {
        assert_eq!(parse_args(&args(&["maki"])), Err(CliError::MissingCommand))
    }

    #[test]
    fn test_parse_serve_defaults_to_current_directory() {
        assert_eq!(
            parse_args(&args(&["maki", "serve"])),
            Ok(Command::Serve {
                root: PathBuf::from(".")
            })
        )
    }

    #[test]
    fn test_list_markdown_files() {
        let path = PathBuf::from("./tests/fixtures/basic-maki-project");
        let files = list_markdown_files(&path).unwrap();
        assert_eq!(
            files,
            vec![
                PathBuf::from("./tests/fixtures/basic-maki-project/README.md"),
                PathBuf::from("./tests/fixtures/basic-maki-project/daily.md"),
            ]
        );
    }

    #[test]
    fn test_relative_markdown_paths() {
        let root = PathBuf::from("./tests/fixtures/basic-maki-project");
        let maki = Maki::load(&root, MakiConfig::default()).unwrap();
        let relative = maki.files;
        assert_eq!(
            relative,
            vec![PathBuf::from("README.md"), PathBuf::from("daily.md"),]
        );
    }

    #[test]
    fn test_parse_request() {
        let raw_request = "GET /favicon.ico HTTP/1.1\r\nHost: localhost:4000\r\nConnection: keep-alive\r\nsec-ch-ua-platform: \"macOS\"\r\nUser-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36\r\nsec-ch-ua: \"Not/A)Brand\";v=\"99\", \"Chromium\";v=\"148\"\r\nsec-ch-ua-mobile: ?0\r\nAccept: image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8\r\nSec-Fetch-Site: same-origin\r\nSec-Fetch-Mode: no-cors\r\nSec-Fetch-Dest: image\r\nReferer: http://localhost:4000/nice\r\nAccept-Encoding: gzip, deflate, br, zstd\r\nAccept-Language: en-US,en;q=0.9,ko;q=0.8\r\n\r\nst: document\r\nAccept-Encoding: gzip, deflate, br, zstd\r\nAccept-Language: en-US,en;q=0.9,ko;q=0.8\r\n\r\n\0";

        let request = parse_request(raw_request);
        assert!(request.is_ok());
        let request = request.unwrap();
        assert_eq!(request.method, HttpMethod::Get);
        assert_eq!(request.target, "/favicon.ico");
        assert_eq!(request.version, HttpVersion::Http1_1);
        assert_eq!(request.body, None);
        assert_eq!(request.headers.get("host").unwrap(), "localhost:4000");
    }

    #[test]
    fn test_handle_unknown_path_returns_not_found() {
        let request = HttpRequest {
            method: HttpMethod::Get,
            target: "/missing".to_string(),
            version: HttpVersion::Http1_1,
            headers: HttpHeaders::new(),
            body: None,
        };

        let maki = Maki {
            root: PathBuf::from("."),
            files: vec![],
            config: MakiConfig::default(),
        };

        let response = maki.handle_request(&request).unwrap();

        assert_eq!(response.status, HttpStatus::NotFound);
    }

    #[test]
    fn test_render_not_found_response() {
        let response = HttpResponse::new(HttpStatus::NotFound)
            .set_header("Content-Type", "text/plain; charset=utf-8")
            .set_body("Not Found".to_string());
        let rendered = String::from_utf8(response.to_raw()).unwrap();
        assert!(rendered.contains("404 Not Found"));
    }

    #[test]
    fn test_read_request_with_split_header() {
        let mut input = &b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n"[..];
        let raw = read_request_head(&mut input).unwrap();
        assert!(raw.ends_with(b"\r\n\r\n"));
    }

    #[test]
    fn test_parse_request_line() {
        let request = "GET /favicon.ico HTTP/1.1";
        let (method, target, version) = parse_request_line(request).unwrap();
        assert_eq!(method, HttpMethod::Get);
        assert_eq!(target, "/favicon.ico");
        assert_eq!(version, HttpVersion::Http1_1);
    }

    #[test]
    fn test_parse_request_headers() {
        let mut lines = ["Host: localhost", "Connection: close"].into_iter();
        let headers = parse_request_headers(&mut lines).unwrap();
        assert_eq!(headers.get("host").unwrap(), "localhost");
        assert_eq!(headers.get("connection").unwrap(), "close");
    }

    #[test]
    fn test_resolve_route() {
        let maki = Maki {
            root: PathBuf::from("."),
            files: vec![
                PathBuf::from("hi.md"),
                PathBuf::from("좋은아침.md"),
                PathBuf::from("foo.bar.md"),
            ],
            config: MakiConfig::default(),
        };
        assert_eq!(maki.resolve_route("/").unwrap(), MakiRoute::Home);
        assert_eq!(
            maki.resolve_route("/n/favicon.ico").unwrap(),
            MakiRoute::NotFound
        );
        assert_eq!(
            maki.resolve_route("/n/hi.md").unwrap(),
            MakiRoute::NoteSource(PathBuf::from("hi.md"))
        );
        assert_eq!(
            maki.resolve_route("/n/hi").unwrap(),
            MakiRoute::NotePage(PathBuf::from("hi.md"))
        );
        assert!(matches!(
            maki.resolve_route("/n/../hi"),
            Err(RunError::BadRequest)
        ));
        assert!(matches!(
            maki.resolve_route("/n/%2e%2e/hi"),
            Err(RunError::BadRequest)
        ));
        assert_eq!(
            maki.resolve_route("/n/%EC%A2%8B%EC%9D%80%EC%95%84%EC%B9%A8.md")
                .unwrap(),
            MakiRoute::NoteSource(PathBuf::from("좋은아침.md"))
        );
        assert_eq!(
            maki.resolve_route("/n/%EC%A2%8B%EC%9D%80%EC%95%84%EC%B9%A8")
                .unwrap(),
            MakiRoute::NotePage(PathBuf::from("좋은아침.md"))
        );
        assert_eq!(
            maki.resolve_route("/n/foo.bar.md").unwrap(),
            MakiRoute::NoteSource(PathBuf::from("foo.bar.md"))
        );
        assert_eq!(
            maki.resolve_route("/n/foo.bar").unwrap(),
            MakiRoute::NotePage(PathBuf::from("foo.bar.md"))
        );
    }

    #[test]
    fn test_render_response() {
        let response = String::from_utf8(
            HttpResponse::new(HttpStatus::BadRequest)
                .set_header("Content-Type", "text/plain")
                .set_body("Bad Request")
                .to_raw(),
        )
        .unwrap();
        assert!(response.contains("400 Bad Request"));
        assert!(response.contains("text/plain"));

        let response = String::from_utf8(
            HttpResponse::new(HttpStatus::InternalServerError)
                .set_header("Content-Type", "text/plain")
                .set_body("Bad Request")
                .to_raw(),
        )
        .unwrap();
        assert!(response.starts_with("HTTP/1.1 500 Internal Server Error\r\n"));
        assert!(response.contains("text/plain"));

        let response = String::from_utf8(
            HttpResponse::new(HttpStatus::NotFound)
                .set_header("Content-Type", "text/plain")
                .set_body("Bad Request")
                .to_raw(),
        )
        .unwrap();
        assert!(response.contains("HTTP/1.1 404 Not Found\r\n"));
        assert!(response.contains("text/plain"));

        let response = String::from_utf8(
            HttpResponse::new(HttpStatus::Ok)
                .set_header("Content-Type", "text/plain")
                .set_body("Bad Request")
                .to_raw(),
        )
        .unwrap();
        assert!(response.contains("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("text/plain"));
    }

    #[test]
    fn test_handle_request() {
        let maki = Maki::load(
            &PathBuf::from("./tests/fixtures/basic-maki-project"),
            MakiConfig::default(),
        )
        .unwrap();

        let request = HttpRequest {
            method: HttpMethod::Get,
            target: "/n/daily.md".to_string(),
            version: HttpVersion::Http1_1,
            headers: HttpHeaders::default(),
            body: None,
        };

        let response = maki.handle_request(&request).unwrap();
        assert_eq!(response.status, HttpStatus::Ok);
        assert!(
            String::from_utf8(response.body)
                .unwrap()
                .contains("# Daily")
        );
        assert!(
            response
                .headers
                .get("Content-Type")
                .is_some_and(|v| v.contains("plain"))
        );

        let request = HttpRequest {
            method: HttpMethod::Get,
            target: "/n/ignore.txt".to_string(),
            version: HttpVersion::Http1_1,
            headers: HttpHeaders::default(),
            body: None,
        };

        let response = maki.handle_request(&request).unwrap();
        assert_eq!(response.status, HttpStatus::NotFound);

        let request = HttpRequest {
            method: HttpMethod::Get,
            target: "/n/README".to_string(),
            version: HttpVersion::Http1_1,
            headers: HttpHeaders::default(),
            body: None,
        };

        let response = maki.handle_request(&request).unwrap();
        assert_eq!(response.status, HttpStatus::Ok);
        assert!(String::from_utf8(response.body).unwrap().contains("Maki"));
        assert!(
            response
                .headers
                .get("Content-Type")
                .is_some_and(|v| v.contains("html"))
        );
    }
}
