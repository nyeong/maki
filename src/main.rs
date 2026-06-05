use std::fmt::Display;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;

mod http;

#[derive(Debug, PartialEq)]
enum Command {
    Serve { root: PathBuf },
}

impl From<http::Error> for RunError {
    fn from(error: http::Error) -> Self {
        RunError::Http(error)
    }
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

impl Default for MakiConfig {
    fn default() -> Self {
        Self {
            home_mode: HomeMode::Redirect("/n/README".to_string()),
            publish_policy: PublishPolicy::PublishAll,
        }
    }
}

struct Maki {
    root: PathBuf,       // canonical absolute path
    files: Vec<PathBuf>, // root-relative markdown paths
    config: MakiConfig,
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
        let request = http::parse_request(&request)?;

        let response = maki.handle_request(&request)?;
        let http_response = response.to_bytes();
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
    fn handle_request(&self, request: &http::Request) -> Result<http::Response, RunError> {
        match self.resolve_route(request.target()) {
            Ok(MakiRoute::NotePage(path)) => Ok(http::Response::new(http::StatusCode::Ok)
                .set_header("Content-Type", "text/html; charset=utf-8")
                .set_body(self.render_html(&path)?)),
            Ok(MakiRoute::NoteSource(path)) => Ok(http::Response::new(http::StatusCode::Ok)
                .set_header("Content-Type", "text/plain; charset=utf-8")
                .set_body(self.get_raw_content(&path)?)),
            Ok(MakiRoute::Home) => match &self.config.home_mode {
                HomeMode::Redirect(path) => Ok(http::Response::new(http::StatusCode::Found)
                    .set_header("Location", path)
                    .set_header("Content-Type", "text/plain; charset=utf-8")
                    .set_body(path.as_bytes())),
            },
            Ok(MakiRoute::NotFound) => Ok(http::Response::new(http::StatusCode::NotFound)
                .set_header("Content-Type", "text/plain; charset=utf-8")
                .set_body("Not Found".to_string())),
            Err(RunError::BadRequest) => Ok(http::Response::new(http::StatusCode::BadRequest)
                .set_header("Content-Type", "text/plain; charset=utf-8")
                .set_body("Bad Request".to_string())),
            Err(e) => Ok(http::Response::new(http::StatusCode::InternalServerError)
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
    Http(http::Error),
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
            RunError::Http(error) => write!(f, "HTTP error: {:?}", error),
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
    fn test_handle_unknown_path_returns_not_found() {
        let request = http::Request::get("/missing");

        let maki = Maki {
            root: PathBuf::from("."),
            files: vec![],
            config: MakiConfig::default(),
        };

        let response = maki.handle_request(&request).unwrap();

        assert_eq!(response.status(), http::StatusCode::NotFound);
    }

    #[test]
    fn test_render_not_found_response() {
        let response = http::Response::new(http::StatusCode::NotFound)
            .set_header("Content-Type", "text/plain; charset=utf-8")
            .set_body("Not Found".to_string());
        assert_eq!(response.status(), http::StatusCode::NotFound);
        assert_eq!(response.body(), b"Not Found".to_vec());
        assert_eq!(
            response.get_header("Content-Type"),
            Some("text/plain; charset=utf-8")
        );
    }

    #[test]
    fn test_read_request_with_split_header() {
        let mut input = &b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n"[..];
        let raw = read_request_head(&mut input).unwrap();
        assert!(raw.ends_with(b"\r\n\r\n"));
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
    fn test_handle_request() {
        let maki = Maki::load(
            &PathBuf::from("./tests/fixtures/basic-maki-project"),
            MakiConfig::default(),
        )
        .unwrap();

        let request = http::Request::get("/n/daily.md");
        let response = maki.handle_request(&request).unwrap();
        assert_eq!(response.status(), http::StatusCode::Ok);
        assert!(
            String::from_utf8(response.body().to_vec())
                .unwrap()
                .contains("# Daily")
        );
        assert!(
            response
                .get_header("Content-Type")
                .is_some_and(|v| v.contains("plain"))
        );

        let request = http::Request::get("/n/ignore.txt");
        let response = maki.handle_request(&request).unwrap();
        assert_eq!(response.status(), http::StatusCode::NotFound);

        let request = http::Request::get("/n/README");
        let response = maki.handle_request(&request).unwrap();
        assert_eq!(response.status(), http::StatusCode::Ok);
        assert!(
            String::from_utf8(response.body().to_vec())
                .unwrap()
                .contains("Maki")
        );
        assert!(
            response
                .get_header("Content-Type")
                .is_some_and(|v| v.contains("html"))
        );
    }
}
