use std::fmt::Display;
use std::path::PathBuf;

mod html;
mod http;
mod maki;
mod parser;
mod web;

use maki::Maki;

#[derive(Debug, PartialEq)]
enum Command {
    Serve { root: PathBuf },
    Build { file: PathBuf },
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

fn run_serve(root: PathBuf) -> Result<(), RunError> {
    let maki = Maki::load(&root)?;
    println!("Found {} markdown files", maki.notes_len());
    for note in maki.notes() {
        println!("- {}", note.source_path().display());
    }
    web::serve(&maki)
}

fn run_build(file: PathBuf) -> Result<(), RunError> {
    let content = std::fs::read_to_string(&file).map_err(|e| RunError::IoError { source: e })?;
    let doc = parser::parse(&content);
    println!("{}", html::render_document(&doc));
    Ok(())
}

fn run_command(command: Command) -> Result<(), RunError> {
    match command {
        Command::Serve { root } => run_serve(root),
        Command::Build { file } => run_build(file),
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

        let error = run_command(Command::Serve { root: path.clone() }).unwrap_err();

        match error {
            RunError::Maki(maki::Error::RootNotFound(realpath)) => assert_eq!(realpath, path),
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
}
