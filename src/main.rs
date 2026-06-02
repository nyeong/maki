use std::fmt::Display;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq)]
enum Command {
    Serve { root: PathBuf },
}

fn handle_request(root: &Path, request: &str, files: &[PathBuf]) -> Result<String, RunError> {
    println!("request: {}", request);
    let html = render_index(root, files)?;
    let http_response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\n\r\n{}",
        html.len(),
        html
    );
    Ok(http_response)
}

#[derive(Debug, PartialEq)]
struct RawRequest {
    method: String,
    path: String,
    protocol: String,
    host: String,
    body: String,
}

/// Parses a raw HTTP request string into a [`RawRequest`] struct.
fn parse_request(request: &str) -> Result<RawRequest, RunError> {
    let parts: Vec<&str> = request.split_whitespace().collect();
    println!("parts: {:?}", parts);

    Ok(RawRequest {
        method: parts[0].to_string(),
        path: parts[1].to_string(),
        protocol: "HTTP/1.1".to_string(),
        host: "localhost:4000".to_string(),
        body: "".to_string(),
    })
}

fn serve_http(root: &Path, files: &[PathBuf]) -> Result<(), RunError> {
    let listener =
        TcpListener::bind("127.0.0.1:4000").map_err(|source| RunError::IoError { source })?;

    println!("Listening on http://localhost:4000");

    let mut buffer = [0u8; 1024];
    for stream in listener.incoming() {
        let mut stream = stream.map_err(|source| RunError::IoError { source })?;
        stream
            .read(&mut buffer)
            .map_err(|source| RunError::IoError { source })?;
        let request = String::from_utf8_lossy(&buffer);
        println!("raw request: {:?}", request);
        let request = parse_request(&request)?;

        println!("{:?}", request);
        let response = handle_request(&root, "hi", files)?;
        stream
            .write_all(response.as_bytes())
            .map_err(|source| RunError::IoError { source })?;
    }

    Ok(())
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
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/favicon.ico");
        assert_eq!(request.protocol, "HTTP/1.1");
        assert_eq!(request.host, "localhost:4000");
        assert_eq!(request.body, "");
    }
}
