use std::fmt::Display;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[derive(Debug, PartialEq)]
enum Command {
    Serve { root: PathBuf },
}

#[derive(Debug, PartialEq)]
enum HttpStatus {
    Ok,
    NotFound,
}

#[derive(Debug, PartialEq)]
enum HttpMethod {
    Get,
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

fn parse_protocol(protocol: &str) -> Result<HttpProtocol, RunError> {
    match protocol {
        "HTTP/1.1" => Ok(HttpProtocol::Http1_1),
        _ => Err(RunError::RequestParseError),
    }
}

#[derive(Debug, PartialEq)]
struct HttpResponse {
    status: HttpStatus,
    content_type: &'static str,
    body: String,
}

impl FromStr for HttpProtocol {
    type Err = RunError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_protocol(s)
    }
}

fn handle_request(
    root: &Path,
    request: &HttpRequest,
    files: &[PathBuf],
) -> Result<HttpResponse, RunError> {
    let response = match request.path.as_str() {
        "/" => {
            let body = render_index(root, files)?;
            HttpResponse {
                status: HttpStatus::Ok,
                content_type: "text/html",
                body,
            }
        }
        _ => HttpResponse {
            status: HttpStatus::NotFound,
            content_type: "text/plain",
            body: "Not Found".to_string(),
        },
    };

    Ok(response)
}

#[derive(Debug, PartialEq)]
enum HttpProtocol {
    Http1_1,
}

#[derive(Debug, PartialEq)]
struct HttpRequest {
    method: HttpMethod,
    path: String,
    protocol: HttpProtocol,
    host: String,
    body: Option<String>,
}

/// Parses a raw HTTP request string into a [`HttpRequest`] struct.
fn parse_request(request: &str) -> Result<HttpRequest, RunError> {
    let mut first_line = request
        .lines()
        .next()
        .ok_or(RunError::RequestParseError)?
        .split_whitespace();
    let method = first_line
        .next()
        .ok_or(RunError::RequestParseError)?
        .parse::<HttpMethod>()?;
    let path = first_line
        .next()
        .ok_or(RunError::RequestParseError)?
        .to_string();
    let protocol = first_line
        .next()
        .ok_or(RunError::RequestParseError)?
        .parse::<HttpProtocol>()?;
    if first_line.next().is_some() {
        return Err(RunError::RequestParseError);
    }

    Ok(HttpRequest {
        method,
        path,
        protocol,
        host: "localhost:4000".to_string(),
        body: None,
    })
}

fn serve_http(root: &Path, files: &[PathBuf]) -> Result<(), RunError> {
    let listener =
        TcpListener::bind("127.0.0.1:4000").map_err(|source| RunError::IoError { source })?;

    println!("Listening on http://localhost:4000");

    let mut buffer = [0u8; 1024];
    for stream in listener.incoming() {
        let mut stream = stream.map_err(|source| RunError::IoError { source })?;
        let bytes_read = stream
            .read(&mut buffer)
            .map_err(|source| RunError::IoError { source })?;
        if bytes_read == 0 {
            continue;
        }
        let request = String::from_utf8_lossy(&buffer[..bytes_read]);
        let request = parse_request(&request)?;

        let response = handle_request(&root, &request, files)?;
        let http_response = render_response(&response);
        stream
            .write_all(http_response.as_bytes())
            .map_err(|source| RunError::IoError { source })?;
    }

    Ok(())
}

fn render_response(response: &HttpResponse) -> String {
    let status = match response.status {
        HttpStatus::Ok => "200 OK",
        HttpStatus::NotFound => "404 Not Found",
    };
    format!(
        "HTTP/1.1 {}\r\nContent-Length: {}\r\nContent-Type: {}\r\n\r\n{}",
        status,
        response.body.len(),
        response.content_type,
        response.body
    )
}

fn render_index(root: &Path, files: &[PathBuf]) -> Result<String, RunError> {
    let mut html = String::from("<!doctype html><html><body><ul>");
    let files = relative_markdown_paths(root, files)?;

    for file in files {
        html.push_str(&format!("<li>{}</li>", file.display(),))
    }

    html.push_str("</ul></body></html>");
    Ok(html)
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
    if !root.exists() {
        return Err(RunError::RootNotFound(root));
    }

    if !root.is_dir() {
        return Err(RunError::RootNotDirectory(root));
    }

    let files = list_markdown_files(&root)?;
    println!("Found {} markdown files", files.len());
    let relative_files = relative_markdown_paths(&root, &files)?;
    for file in &relative_files {
        println!("- {}", file.display());
    }
    serve_http(&root, &files)
}

fn relative_markdown_paths(root: &Path, files: &[PathBuf]) -> Result<Vec<PathBuf>, RunError> {
    let mut vec = Vec::new();

    for file in files {
        let relative = file
            .strip_prefix(root)
            .map_err(|source| RunError::InvalidMarkdownPath {
                path: file.to_path_buf(),
                source,
            })?
            .to_path_buf();
        vec.push(relative);
    }

    Ok(vec)
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
        let files = list_markdown_files(&root).unwrap();
        let relative = relative_markdown_paths(&root, &files).unwrap();
        assert_eq!(
            relative,
            vec![PathBuf::from("README.md"), PathBuf::from("daily.md"),]
        );
    }

    #[test]
    fn test_render_index() {
        let html = render_index(
            &PathBuf::from("."),
            &[PathBuf::from("./README.md"), PathBuf::from("./daily.md")],
        );
        assert!(html.is_ok());
        let html = html.unwrap();

        assert!(html.contains("README.md"));
        assert!(html.contains("daily.md"));
    }

    #[test]
    fn test_parse_request() {
        let raw_request = "GET /favicon.ico HTTP/1.1\r\nHost: localhost:4000\r\nConnection: keep-alive\r\nsec-ch-ua-platform: \"macOS\"\r\nUser-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36\r\nsec-ch-ua: \"Not/A)Brand\";v=\"99\", \"Chromium\";v=\"148\"\r\nsec-ch-ua-mobile: ?0\r\nAccept: image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8\r\nSec-Fetch-Site: same-origin\r\nSec-Fetch-Mode: no-cors\r\nSec-Fetch-Dest: image\r\nReferer: http://localhost:4000/nice\r\nAccept-Encoding: gzip, deflate, br, zstd\r\nAccept-Language: en-US,en;q=0.9,ko;q=0.8\r\n\r\nst: document\r\nAccept-Encoding: gzip, deflate, br, zstd\r\nAccept-Language: en-US,en;q=0.9,ko;q=0.8\r\n\r\n\0";

        let request = parse_request(raw_request);
        assert!(request.is_ok());
        let request = request.unwrap();
        assert_eq!(request.method, HttpMethod::Get);
        assert_eq!(request.path, "/favicon.ico");
        assert_eq!(request.protocol, HttpProtocol::Http1_1);
        assert_eq!(request.host, "localhost:4000");
        assert_eq!(request.body, None);
    }

    #[test]
    fn test_handle_unknown_path_returns_not_found() {
        let request = HttpRequest {
            method: HttpMethod::Get,
            path: "/missing".to_string(),
            protocol: HttpProtocol::Http1_1,
            host: "localhost:4000".to_string(),
            body: None,
        };

        let response = handle_request(&PathBuf::from("."), &request, &[]).unwrap();

        assert_eq!(response.status, HttpStatus::NotFound);
    }

    #[test]
    fn test_render_not_found_response() {
        let response = HttpResponse {
            status: HttpStatus::NotFound,
            content_type: "text/plain",
            body: "Not Found".to_string(),
        };
        let rendered = render_response(&response);

        assert!(rendered.contains("404 Not Found"));
    }
}
