use std::fmt::Display;
use std::path::{Path, PathBuf};

mod http;
mod web;

use percent_encoding::percent_decode_str;

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
    Redirect(String),
}

impl Default for MakiConfig {
    fn default() -> Self {
        Self {
            home_mode: HomeMode::Redirect("/README".to_string()),
            publish_policy: PublishPolicy::PublishAll,
        }
    }
}

struct Maki {
    root: PathBuf,       // canonical absolute path
    files: Vec<PathBuf>, // root-relative markdown paths
    config: MakiConfig,
}

fn parse_markdown(markdown: &str) -> String {
    let mut options = pulldown_cmark::Options::empty();
    options.insert(pulldown_cmark::Options::ENABLE_GFM);
    options.insert(pulldown_cmark::Options::ENABLE_WIKILINKS);
    let parser = pulldown_cmark::Parser::new_ext(markdown, options);
    let mut buffer = String::new();
    pulldown_cmark::html::push_html(&mut buffer, parser);

    buffer
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

        let files = list_markdown_files(&root)?;

        Ok(Self {
            root,
            files,
            config,
        })
    }

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
        let content = parse_markdown(&content);
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
    /// ```text
    /// maki.resolve_route("/maki"); // => MakiRoute::NotePage("maki.md")
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

        self.resolve_note_route(&target)
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

fn collect_markdown_files(
    root: &Path,
    current: &Path,
    acc: &mut Vec<PathBuf>,
) -> Result<(), RunError> {
    let entries = std::fs::read_dir(current).map_err(|source| RunError::ReadDirectoryFailed {
        path: current.to_path_buf(),
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| RunError::ReadDirectoryFailed {
            path: current.to_path_buf(),
            source,
        })?;

        let path = entry.path();

        if path.is_dir() {
            collect_markdown_files(root, &path, acc)?;
        } else if path.is_file() && path.extension().is_some_and(|ext| ext == "md") {
            acc.push(get_relative_path(root, &path)?);
        }
    }
    Ok(())
}

/// Lists all markdown files in the given directory.
fn list_markdown_files(root: &Path) -> Result<Vec<PathBuf>, RunError> {
    let mut files = Vec::new();
    collect_markdown_files(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn run_serve(root: PathBuf) -> Result<(), RunError> {
    let maki = Maki::load(&root, MakiConfig::default())?;
    println!("Found {} markdown files", maki.files.len());
    for file in &maki.files {
        println!("- {}", file.display());
    }
    web::serve(&maki)
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
                PathBuf::from("README.md"),
                PathBuf::from("daily.md"),
                PathBuf::from("nested/nested.md"),
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
            vec![
                PathBuf::from("README.md"),
                PathBuf::from("daily.md"),
                PathBuf::from("nested/nested.md"),
            ]
        );
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
            maki.resolve_route("/favicon.ico").unwrap(),
            MakiRoute::NotFound
        );
        assert_eq!(
            maki.resolve_route("/hi.md").unwrap(),
            MakiRoute::NoteSource(PathBuf::from("hi.md"))
        );
        assert_eq!(
            maki.resolve_route("/hi").unwrap(),
            MakiRoute::NotePage(PathBuf::from("hi.md"))
        );
        assert!(matches!(
            maki.resolve_route("/../hi"),
            Err(RunError::BadRequest)
        ));
        assert!(matches!(
            maki.resolve_route("/%2e%2e/hi"),
            Err(RunError::BadRequest)
        ));
        assert_eq!(
            maki.resolve_route("/%EC%A2%8B%EC%9D%80%EC%95%84%EC%B9%A8.md")
                .unwrap(),
            MakiRoute::NoteSource(PathBuf::from("좋은아침.md"))
        );
        assert_eq!(
            maki.resolve_route("/%EC%A2%8B%EC%9D%80%EC%95%84%EC%B9%A8")
                .unwrap(),
            MakiRoute::NotePage(PathBuf::from("좋은아침.md"))
        );
        assert_eq!(
            maki.resolve_route("/foo.bar.md").unwrap(),
            MakiRoute::NoteSource(PathBuf::from("foo.bar.md"))
        );
        assert_eq!(
            maki.resolve_route("/foo.bar").unwrap(),
            MakiRoute::NotePage(PathBuf::from("foo.bar.md"))
        );
    }
}
