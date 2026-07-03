use std::fmt::Display;
use std::path::PathBuf;

mod html;
mod http;
mod maki;
mod parser;
mod web;

use maki::{Maki, MakiConfig};

#[derive(Debug, PartialEq)]
enum Command {
    Serve {
        root: PathBuf,
        options: ServeOptions,
    },
    Build {
        file: PathBuf,
    },
}

#[derive(Debug, PartialEq, Clone)]
struct ServeOptions {
    host: String,
    port: u16,
    index_redirect: String,
}

impl Default for ServeOptions {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 4000,
            index_redirect: "/README".to_string(),
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
    Http(http::Error),
    Maki(maki::Error),
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
            RunError::Http(error) => write!(f, "HTTP error: {:?}", error),
            RunError::Maki(maki_error) => write!(f, "Maki error: {}", maki_error),
        }
    }
}

fn run_serve(root: PathBuf, options: ServeOptions) -> Result<(), RunError> {
    let config = MakiConfig::with_home_redirect(options.index_redirect.clone());
    let maki = Maki::load_with_config(&root, config)?;
    println!("Found {} files", maki.notes_len());
    for note in maki.notes() {
        println!("- {}", note.source_path().display());
    }
    web::serve(&maki, &options.host, options.port)
}

fn run_build(file: PathBuf) -> Result<(), RunError> {
    let content = std::fs::read_to_string(&file).map_err(|e| RunError::IoError { source: e })?;
    let doc = parser::parse(&content);
    println!("{}", html::render_document(&doc));
    Ok(())
}

fn run_command(command: Command) -> Result<(), RunError> {
    match command {
        Command::Serve { root, options } => run_serve(root, options),
        Command::Build { file } => run_build(file),
    }
}

#[derive(Debug, PartialEq)]
enum CliError {
    MissingCommand,
    UnknownCommand(String),
    UnknownOption(String),
    MissingOptionValue(String),
    InvalidPort(String),
    UnexpectedArgument(String),
}

impl Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::UnknownCommand(s) => write!(f, "Unknown command: {}", s),
            CliError::MissingCommand => write!(f, "Missing command"),
            CliError::UnknownOption(s) => write!(f, "Unknown option: {}", s),
            CliError::MissingOptionValue(s) => write!(f, "Missing value for option: {}", s),
            CliError::InvalidPort(s) => write!(f, "Invalid port: {}", s),
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
    let mut options = ServeOptions::default();
    let mut index = 2;

    while index < args.len() {
        match args[index].as_str() {
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
                options.index_redirect = normalize_redirect_target(target);
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

    Ok(Command::Serve {
        root: root.unwrap_or_else(|| PathBuf::from(".")),
        options,
    })
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
    fn test_run_serve_not_exists() {
        let path = PathBuf::from("./tests/not-exists");

        let error = run_command(Command::Serve {
            root: path.clone(),
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
                root: PathBuf::from("path/to/maki"),
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
                root: PathBuf::from("path/to/maki"),
                options: ServeOptions {
                    host: "0.0.0.0".to_string(),
                    port: 8080,
                    index_redirect: "/docs/index".to_string(),
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
                root: PathBuf::from("path/to/maki"),
                options: ServeOptions {
                    host: "0.0.0.0".to_string(),
                    port: 8080,
                    index_redirect: "/README".to_string(),
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
                root: PathBuf::from("."),
                options: ServeOptions::default(),
            })
        )
    }
}
