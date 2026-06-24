use std::fmt::Display;
use std::path::PathBuf;

mod http;
mod maki;
mod parser;
mod renderer;
mod web;

use maki::Maki;

#[derive(Debug, PartialEq)]
enum Command {
    Serve { root: PathBuf },
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
        println!("- {}", note.path().display());
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
            RunError::Maki(maki::Error::RootNotFound(realpath)) => assert_eq!(realpath, path),
            _ => panic!("Unexpected error: {:?}", error),
        }
    }

    #[test]
    fn test_run_serve_not_directory() {
        let path = PathBuf::from("./tests/fixtures/basic-maki-project/README.md");

        let error = run_command(Command::Serve { root: path.clone() }).unwrap_err();

        match error {
            RunError::Maki(maki::Error::RootNotDirectory(realpath)) => assert_eq!(realpath, path),
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
    fn test_relative_markdown_paths() {
        let root = PathBuf::from("./tests/fixtures/basic-maki-project");
        let maki = Maki::load(&root).unwrap();
        let relative = maki
            .notes()
            .iter()
            .map(|n| n.path().to_path_buf())
            .collect::<Vec<_>>();
        assert_eq!(
            relative,
            vec![
                PathBuf::from("README.md"),
                PathBuf::from("daily.md"),
                PathBuf::from("nested/nested.md"),
                PathBuf::from("nested/한글.md"),
            ]
        );
    }
}
