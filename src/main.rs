use std::fmt::Display;
use std::path::{Path, PathBuf};

mod git_source;
mod html;
mod http;
mod maki;
mod parser;
mod web;

use maki::{Maki, MakiConfig, MakiConfigOverrides, ProjectDiagnostic, ProjectDiagnosticSummary};

#[derive(Debug, PartialEq)]
enum Command {
    Serve {
        source: ServeSource,
        options: ServeOptions,
    },
    Build {
        file: PathBuf,
    },
}

#[derive(Debug, PartialEq)]
enum ServeSource {
    Path(PathBuf),
    Git(git_source::GitServeConfig),
}

#[derive(Debug, PartialEq, Clone)]
struct ServeOptions {
    host: String,
    port: u16,
    index_redirect: Option<String>,
}

impl Default for ServeOptions {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 4000,
            index_redirect: None,
        }
    }
}

impl From<http::Error> for RunError {
    fn from(error: http::Error) -> Self {
        RunError::Http(error)
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
    IoError { source: std::io::Error },
    Git(git_source::Error),
    Http(http::Error),
    Maki(maki::Error),
}

impl From<git_source::Error> for RunError {
    fn from(source: git_source::Error) -> Self {
        RunError::Git(source)
    }
}

impl From<maki::Error> for RunError {
    fn from(source: maki::Error) -> RunError {
        RunError::Maki(source)
    }
}

impl Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::IoError { source } => write!(f, "IO error: {}", source),
            RunError::Git(error) => write!(f, "Git source error: {}", error),
            RunError::Http(error) => write!(f, "HTTP error: {:?}", error),
            RunError::Maki(maki_error) => write!(f, "Maki error: {}", maki_error),
        }
    }
}

fn run_path_serve(root: PathBuf, options: ServeOptions) -> Result<(), RunError> {
    let Some(project_root) = Maki::find_project_root(&root)? else {
        return run_directory_serve(root.clone(), root, MakiConfig::default(), options);
    };

    let config = MakiConfig::load_project(&project_root)?;
    let source_root = config.project_source_root(&project_root);
    if same_path(&root, &project_root)? || is_project_source_path(&root, &source_root)? {
        run_directory_serve(project_root, source_root, config, options)
    } else {
        run_directory_serve(root.clone(), root, MakiConfig::default(), options)
    }
}

fn run_directory_serve(
    project_root: PathBuf,
    source_root: PathBuf,
    mut config: MakiConfig,
    options: ServeOptions,
) -> Result<(), RunError> {
    let config_overrides = MakiConfigOverrides::from_home_redirect(options.index_redirect);
    config_overrides.apply_to(&mut config);

    let maki = Maki::load_with_config(&source_root, config)?;
    if let Some(project_title) = maki.config().project_title() {
        println!("Project: {project_title}");
    }
    println!("Found {} files", maki.notes_len());
    for note in maki.notes() {
        println!("- {}", note.source_path().display());
    }
    emit_project_diagnostic_summary(&maki.diagnostics_without_external_links());
    web::serve_project(
        maki,
        project_root,
        &options.host,
        options.port,
        config_overrides,
    )
}

fn run_git_serve(
    git_config: git_source::GitServeConfig,
    options: ServeOptions,
) -> Result<(), RunError> {
    let config_overrides = MakiConfigOverrides::from_home_redirect(options.index_redirect);
    let source = git_source::GitSource::new(git_config);
    eprintln!("Preparing git source...");
    let checkout = source.prepare()?;
    eprintln!("Loading git checkout {}...", checkout.commit());
    let maki = source.load_maki(&checkout, &config_overrides)?;

    if let Some(project_title) = maki.config().project_title() {
        println!("Project: {project_title}");
    }
    println!("Git commit: {}", checkout.commit());
    println!("Found {} files", maki.notes_len());
    for note in maki.notes() {
        println!("- {}", note.source_path().display());
    }
    emit_project_diagnostic_summary(&maki.diagnostics_without_external_links());
    source.record_active(&checkout)?;

    let initial_commit = checkout.commit().to_string();
    let updater_source = source.clone();
    let updater_overrides = config_overrides.clone();
    web::serve_with_runtime(
        maki,
        checkout.root().to_path_buf(),
        &options.host,
        options.port,
        config_overrides,
        web::ServeRuntime::Publish,
        move |state| {
            git_source::spawn_updater(updater_source, updater_overrides, state, initial_commit);
        },
    )
}

fn run_serve(source: ServeSource, options: ServeOptions) -> Result<(), RunError> {
    match source {
        ServeSource::Path(root) => run_path_serve(root, options),
        ServeSource::Git(git_config) => run_git_serve(git_config, options),
    }
}

fn run_build(file: PathBuf) -> Result<(), RunError> {
    let content = std::fs::read_to_string(&file).map_err(|e| RunError::IoError { source: e })?;
    let parsed = parser::parse(&content);

    let html = match Maki::find_project_root(&file)? {
        Some(root) => {
            let config = MakiConfig::load_project(&root)?;
            let source_root = config.project_source_root(&root);
            if is_project_source_path(&file, &source_root)? {
                let maki = Maki::load_with_config(&source_root, config)?;
                emit_project_diagnostic_summary(&maki.diagnostics());
                maki.render_file_html(&file)?
            } else {
                emit_parse_warnings(&file, &parsed.diagnostics);
                html::render_document(&parsed.document)
            }
        }
        None => {
            emit_parse_warnings(&file, &parsed.diagnostics);
            html::render_document(&parsed.document)
        }
    };

    println!("{html}");
    Ok(())
}

fn same_path(left: &Path, right: &Path) -> Result<bool, RunError> {
    let left = std::fs::canonicalize(left)
        .map_err(|_source| maki::Error::RootNotFound(left.to_path_buf()))?;
    let right = std::fs::canonicalize(right)
        .map_err(|_source| maki::Error::RootNotFound(right.to_path_buf()))?;
    Ok(left == right)
}

fn is_project_source_path(path: &Path, source_root: &Path) -> Result<bool, RunError> {
    let path = std::fs::canonicalize(path)
        .map_err(|_source| maki::Error::RootNotFound(path.to_path_buf()))?;
    let Ok(source_root) = std::fs::canonicalize(source_root) else {
        return Ok(false);
    };
    Ok(path.starts_with(source_root))
}

fn emit_parse_warnings(file: &Path, diagnostics: &[parser::ParseDiagnostic<'_>]) {
    for diagnostic in diagnostics {
        eprintln!("{}", format_parse_warning(file, diagnostic));
    }
}

fn format_parse_warning(file: &Path, diagnostic: &parser::ParseDiagnostic<'_>) -> String {
    format!(
        "warning: {}:{}: {}",
        file.display(),
        diagnostic.line,
        format_parse_warning_kind(&diagnostic.kind)
    )
}

fn format_parse_warning_kind(kind: &parser::ParseDiagnosticKind<'_>) -> String {
    parser::format_parse_diagnostic_kind(kind)
}

fn emit_project_diagnostic_summary(diagnostics: &[ProjectDiagnostic]) {
    if diagnostics.is_empty() {
        return;
    }

    eprintln!("{}", format_project_diagnostic_summary(diagnostics));
    for diagnostic in diagnostics {
        eprintln!("{}", format_project_diagnostic(diagnostic));
    }
}

fn format_project_diagnostic_summary(diagnostics: &[ProjectDiagnostic]) -> String {
    let summary = ProjectDiagnosticSummary::from_diagnostics(diagnostics);

    format!(
        "diagnostics: {} issue(s): {} broken link(s), {} ambiguous link(s), {} broken external link(s), {} parser warning(s), {} read failure(s)",
        summary.total(),
        summary.broken_links(),
        summary.ambiguous_links(),
        summary.broken_external_links(),
        summary.parse_warnings(),
        summary.read_failures()
    )
}

fn format_project_diagnostic(diagnostic: &ProjectDiagnostic) -> String {
    let mut location = diagnostic.source_path().display().to_string();
    if let Some(line) = diagnostic.line() {
        location.push(':');
        location.push_str(&line.to_string());
    }

    format!("warning: {}: {}", location, diagnostic.message())
}

fn run_command(command: Command) -> Result<(), RunError> {
    match command {
        Command::Serve { source, options } => run_serve(source, options),
        Command::Build { file } => run_build(file),
    }
}

#[derive(Debug, PartialEq)]
enum CliError {
    MissingCommand,
    UnknownCommand(String),
    UnknownOption(String),
    MissingOptionValue(String),
    InvalidDuration(String),
    InvalidPort(String),
    InvalidServeSource(String),
    UnexpectedArgument(String),
}

impl Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::UnknownCommand(s) => write!(f, "Unknown command: {}", s),
            CliError::MissingCommand => write!(f, "Missing command"),
            CliError::UnknownOption(s) => write!(f, "Unknown option: {}", s),
            CliError::MissingOptionValue(s) => write!(f, "Missing value for option: {}", s),
            CliError::InvalidDuration(s) => write!(f, "Invalid duration: {}", s),
            CliError::InvalidPort(s) => write!(f, "Invalid port: {}", s),
            CliError::InvalidServeSource(s) => write!(f, "Invalid serve source: {}", s),
            CliError::UnexpectedArgument(s) => write!(f, "Unexpected argument: {}", s),
        }
    }
}

fn normalize_redirect_target(target: &str) -> String {
    if target.starts_with('/') {
        target.to_string()
    } else {
        format!("/{target}")
    }
}

fn parse_serve_args(args: &[String]) -> Result<Command, CliError> {
    let mut root = None;
    let mut git_url = None;
    let mut git_branch = None;
    let mut git_state_dir = None;
    let mut git_fetch_interval = None;
    let mut options = ServeOptions::default();
    let mut index = 2;

    while index < args.len() {
        match args[index].as_str() {
            "--git" => {
                index += 1;
                git_url = Some(
                    args.get(index)
                        .ok_or_else(|| CliError::MissingOptionValue("--git".to_string()))?
                        .clone(),
                );
            }
            "--branch" => {
                index += 1;
                git_branch = Some(
                    args.get(index)
                        .ok_or_else(|| CliError::MissingOptionValue("--branch".to_string()))?
                        .clone(),
                );
            }
            "--state-dir" => {
                index += 1;
                git_state_dir =
                    Some(PathBuf::from(args.get(index).ok_or_else(|| {
                        CliError::MissingOptionValue("--state-dir".to_string())
                    })?));
            }
            "--fetch-interval" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| CliError::MissingOptionValue("--fetch-interval".to_string()))?;
                git_fetch_interval =
                    Some(git_source::parse_fetch_interval(raw).map_err(CliError::InvalidDuration)?);
            }
            "--host" => {
                index += 1;
                options.host = args
                    .get(index)
                    .ok_or_else(|| CliError::MissingOptionValue("--host".to_string()))?
                    .clone();
            }
            "--port" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| CliError::MissingOptionValue("--port".to_string()))?;
                options.port = raw
                    .parse()
                    .map_err(|_| CliError::InvalidPort(raw.to_string()))?;
            }
            "--index-redirect" => {
                index += 1;
                let target = args
                    .get(index)
                    .ok_or_else(|| CliError::MissingOptionValue("--index-redirect".to_string()))?;
                options.index_redirect = Some(normalize_redirect_target(target));
            }
            option if option.starts_with("--") => {
                return Err(CliError::UnknownOption(option.to_string()));
            }
            path => {
                if root.is_some() {
                    return Err(CliError::UnexpectedArgument(path.to_string()));
                }
                root = Some(PathBuf::from(path));
            }
        }

        index += 1;
    }

    let source = if let Some(url) = git_url {
        if let Some(root) = root {
            return Err(CliError::InvalidServeSource(format!(
                "cannot combine path {} with --git",
                root.display()
            )));
        }

        let mut config = git_source::GitServeConfig::new(url);
        if let Some(branch) = git_branch {
            config.branch = branch;
        }
        if let Some(state_dir) = git_state_dir {
            config.state_dir = state_dir;
        }
        if let Some(fetch_interval) = git_fetch_interval {
            config.fetch_interval = fetch_interval;
        }
        ServeSource::Git(config)
    } else {
        if git_branch.is_some() || git_state_dir.is_some() || git_fetch_interval.is_some() {
            return Err(CliError::InvalidServeSource(
                "--branch, --state-dir, and --fetch-interval require --git".to_string(),
            ));
        }
        ServeSource::Path(root.unwrap_or_else(|| PathBuf::from(".")))
    };

    Ok(Command::Serve { source, options })
}

fn parse_args(args: &[String]) -> Result<Command, CliError> {
    // 0 is the binary name
    let command = args.get(1).ok_or(CliError::MissingCommand)?;

    match command.as_str() {
        "serve" => parse_serve_args(args),
        "build" => {
            // TODO: 에러 유형 바꾸기
            let file = args.get(2).ok_or(CliError::MissingCommand)?;
            Ok(Command::Build {
                file: PathBuf::from(file),
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
    fn test_format_parse_warning() {
        let diagnostic = parser::ParseDiagnostic {
            line: 3,
            kind: parser::ParseDiagnosticKind::InvalidProperty {
                raw_line: "--^ invalid-property",
            },
        };

        assert_eq!(
            format_parse_warning(Path::new("docs/example.maki"), &diagnostic),
            "warning: docs/example.maki:3: invalid property: --^ invalid-property"
        );
    }

    #[test]
    fn test_run_serve_not_exists() {
        let path = PathBuf::from("./tests/not-exists");

        let error = run_command(Command::Serve {
            source: ServeSource::Path(path.clone()),
            options: ServeOptions::default(),
        })
        .unwrap_err();

        match error {
            RunError::Maki(maki::Error::RootNotFound(realpath)) => assert_eq!(realpath, path),
            _ => panic!("Unexpected error: {:?}", error),
        }
    }

    #[test]
    fn test_parse_serve_command() {
        assert_eq!(
            parse_args(&args(&["maki", "serve", "path/to/maki"])),
            Ok(Command::Serve {
                source: ServeSource::Path(PathBuf::from("path/to/maki")),
                options: ServeOptions::default(),
            })
        )
    }

    #[test]
    fn test_parse_serve_options() {
        assert_eq!(
            parse_args(&args(&[
                "maki",
                "serve",
                "path/to/maki",
                "--host",
                "0.0.0.0",
                "--port",
                "8080",
                "--index-redirect",
                "docs/index",
            ])),
            Ok(Command::Serve {
                source: ServeSource::Path(PathBuf::from("path/to/maki")),
                options: ServeOptions {
                    host: "0.0.0.0".to_string(),
                    port: 8080,
                    index_redirect: Some("/docs/index".to_string()),
                },
            })
        )
    }

    #[test]
    fn test_parse_serve_options_before_root() {
        assert_eq!(
            parse_args(&args(&[
                "maki",
                "serve",
                "--host",
                "0.0.0.0",
                "--port",
                "8080",
                "path/to/maki",
            ])),
            Ok(Command::Serve {
                source: ServeSource::Path(PathBuf::from("path/to/maki")),
                options: ServeOptions {
                    host: "0.0.0.0".to_string(),
                    port: 8080,
                    index_redirect: None,
                },
            })
        )
    }

    #[test]
    fn test_parse_serve_invalid_port() {
        assert_eq!(
            parse_args(&args(&["maki", "serve", "--port", "not-a-port"])),
            Err(CliError::InvalidPort("not-a-port".to_string()))
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
                source: ServeSource::Path(PathBuf::from(".")),
                options: ServeOptions::default(),
            })
        )
    }

    #[test]
    fn test_parse_git_serve_options() {
        let state_dir = PathBuf::from("/tmp/maki-state");
        assert_eq!(
            parse_args(&args(&[
                "maki",
                "serve",
                "--git",
                "git@example.com:nyeong/blog.git",
                "--branch",
                "main",
                "--state-dir",
                "/tmp/maki-state",
                "--fetch-interval",
                "5s",
                "--host",
                "0.0.0.0",
                "--port",
                "8080",
                "--index-redirect",
                "index",
            ])),
            Ok(Command::Serve {
                source: ServeSource::Git(git_source::GitServeConfig {
                    url: "git@example.com:nyeong/blog.git".to_string(),
                    branch: "main".to_string(),
                    state_dir,
                    fetch_interval: std::time::Duration::from_secs(5),
                }),
                options: ServeOptions {
                    host: "0.0.0.0".to_string(),
                    port: 8080,
                    index_redirect: Some("/index".to_string()),
                },
            })
        )
    }

    #[test]
    fn test_parse_git_serve_defaults() {
        assert_eq!(
            parse_args(&args(&[
                "maki",
                "serve",
                "--git",
                "https://example.com/blog.git"
            ])),
            Ok(Command::Serve {
                source: ServeSource::Git(git_source::GitServeConfig::new(
                    "https://example.com/blog.git".to_string()
                )),
                options: ServeOptions::default(),
            })
        )
    }

    #[test]
    fn test_parse_git_options_require_git_source() {
        assert_eq!(
            parse_args(&args(&["maki", "serve", "--branch", "main"])),
            Err(CliError::InvalidServeSource(
                "--branch, --state-dir, and --fetch-interval require --git".to_string()
            ))
        )
    }
}
