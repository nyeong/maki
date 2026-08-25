use super::*;

fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
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
                metrics: None,
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
                metrics: None,
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
fn test_parse_lsp_command() {
    assert_eq!(
        parse_args(&["maki".to_string(), "lsp".to_string()]),
        Ok(Command::Lsp)
    );
    assert_eq!(
        parse_args(&[
            "maki".to_string(),
            "lsp".to_string(),
            "unexpected".to_string(),
        ]),
        Err(CliError::UnexpectedArgument("unexpected".to_string()))
    );
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
            "--metrics",
            "127.0.0.1:4041",
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
                metrics: Some(web::MetricsEndpoint {
                    host: "127.0.0.1".to_string(),
                    port: 4041,
                }),
            },
        })
    )
}

#[test]
fn test_parse_serve_metrics_endpoint() {
    assert_eq!(
        parse_args(&args(&[
            "maki",
            "serve",
            "docs",
            "--metrics",
            "127.0.0.1:4041"
        ])),
        Ok(Command::Serve {
            source: ServeSource::Path(PathBuf::from("docs")),
            options: ServeOptions {
                host: "127.0.0.1".to_string(),
                port: 4000,
                index_redirect: None,
                metrics: Some(web::MetricsEndpoint {
                    host: "127.0.0.1".to_string(),
                    port: 4041,
                }),
            },
        })
    )
}

#[test]
fn test_parse_serve_invalid_metrics_endpoint() {
    assert_eq!(
        parse_args(&args(&["maki", "serve", "--metrics", "4041"])),
        Err(CliError::InvalidMetricsEndpoint("4041".to_string()))
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
