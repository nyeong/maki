use std::fmt::Display;
use std::path::PathBuf;

use maki_serve::{git_source, web};

#[derive(Debug, PartialEq)]
pub(crate) enum Command {
    Serve {
        source: ServeSource,
        options: ServeOptions,
    },
    Build {
        file: PathBuf,
    },
}

#[derive(Debug, PartialEq)]
pub(crate) enum ServeSource {
    Path(PathBuf),
    Git(git_source::GitServeConfig),
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct ServeOptions {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) index_redirect: Option<String>,
    pub(crate) metrics: Option<web::MetricsEndpoint>,
}

impl Default for ServeOptions {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 4000,
            index_redirect: None,
            metrics: None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum CliError {
    MissingCommand,
    UnknownCommand(String),
    UnknownOption(String),
    MissingOptionValue(String),
    InvalidDuration(String),
    InvalidMetricsEndpoint(String),
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
            CliError::InvalidMetricsEndpoint(s) => write!(f, "Invalid metrics endpoint: {}", s),
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

fn parse_metrics_endpoint(raw: &str) -> Result<web::MetricsEndpoint, CliError> {
    let (host, port) = raw
        .rsplit_once(':')
        .ok_or_else(|| CliError::InvalidMetricsEndpoint(raw.to_string()))?;
    if host.is_empty() {
        return Err(CliError::InvalidMetricsEndpoint(raw.to_string()));
    }
    let port = port
        .parse()
        .map_err(|_| CliError::InvalidMetricsEndpoint(raw.to_string()))?;

    Ok(web::MetricsEndpoint {
        host: host.to_string(),
        port,
    })
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
            "--metrics" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| CliError::MissingOptionValue("--metrics".to_string()))?;
                options.metrics = Some(parse_metrics_endpoint(raw)?);
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

pub(crate) fn parse_args(args: &[String]) -> Result<Command, CliError> {
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
mod tests;
