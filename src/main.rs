use std::fmt::Display;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq)]
enum Command {
    Serve { root: PathBuf },
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
}

impl Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::RootNotFound(path) => write!(f, "Root not found: {}", path.display()),
            RunError::RootNotDirectory(path) => {
                write!(f, "Root not a directory: {}", path.display())
            }

            RunError::ReadDirectoryFailed { path, source } => {
                write!(f, "Failed to read directory {}: {}", path.display(), source)
            }
        }
    }
}

fn list_markdown_files(root: &Path) -> Result<Vec<PathBuf>, RunError> {
    let entries = std::fs::read_dir(root).map_err(|source| RunError::ReadDirectoryFailed {
        path: root.to_path_buf(),
        source,
    })?;

    let mut files = Vec::new();

    println!("Markdown files:");
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
    println!("{:?}", files);

    Ok(())
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
    MissingArgument {
        command: &'static str,
        argument: &'static str,
    },
}

impl Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::UnknownCommand(s) => write!(f, "Unknown command: {}", s),
            CliError::MissingArgument { command, argument } => {
                write!(f, "Missing argument: {} for command: {}", argument, command)
            }
            CliError::MissingCommand => write!(f, "Missing command"),
        }
    }
}

fn parse_args(args: &[String]) -> Result<Command, CliError> {
    // 0 is the binary name
    let command = args.get(1).ok_or(CliError::MissingCommand)?;

    match command.as_str() {
        "serve" => {
            let root = args.get(2).ok_or(CliError::MissingArgument {
                command: "serve",
                argument: "markdown-directory",
            })?;

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
    fn test_run_serve() {
        let path = PathBuf::from("./tests/fixtures/basic-maki-project");

        match run_command(Command::Serve { root: path.clone() }) {
            Ok(_) => {}
            Err(e) => panic!("Unexpected error: {:?}", e),
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
    fn test_parse_missing_argument() {
        assert_eq!(
            parse_args(&args(&["maki", "serve"])),
            Err(CliError::MissingArgument {
                command: "serve",
                argument: "markdown-directory",
            })
        )
    }

    #[test]
    fn test_list_markdown_files() {
        let path = PathBuf::from("./tests/fixtures/basic-maki-project");
        let files = list_markdown_files(&path).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0],
            PathBuf::from("./tests/fixtures/basic-maki-project/README.md")
        );
    }
}
