use std::fmt::Display;
use std::path::{Path, PathBuf};

use maki_core::{Error as MakiError, Maki, MakiConfig, MakiConfigOverrides, html, parser};
use maki_serve::{git_source, metrics::Metrics, web};

use crate::cli::{Command, ServeOptions, ServeSource};
use crate::output::{emit_parse_warnings, emit_project_diagnostic_summary};

#[derive(Debug)]
pub(crate) enum RunError {
    IoError { source: std::io::Error },
    Git(git_source::Error),
    Serve(maki_serve::RunError),
    Maki(MakiError),
    Lsp(String),
}

impl From<git_source::Error> for RunError {
    fn from(source: git_source::Error) -> Self {
        RunError::Git(source)
    }
}

impl From<maki_serve::RunError> for RunError {
    fn from(source: maki_serve::RunError) -> Self {
        RunError::Serve(source)
    }
}

impl From<MakiError> for RunError {
    fn from(source: MakiError) -> RunError {
        RunError::Maki(source)
    }
}

impl Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::IoError { source } => write!(f, "IO error: {}", source),
            RunError::Git(error) => write!(f, "Git source error: {}", error),
            RunError::Serve(error) => write!(f, "Serve error: {}", error),
            RunError::Maki(maki_error) => write!(f, "Maki error: {}", maki_error),
            RunError::Lsp(message) => write!(f, "LSP error: {message}"),
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
    let metrics = metrics_for_endpoint(&options.metrics);

    let maki = Maki::load_with_config_metered(&source_root, config, &metrics)?;
    if let Some(project_title) = maki.config().project_title() {
        println!("Project: {project_title}");
    }
    println!("Found {} files", maki.notes_len());
    for note in maki.notes() {
        println!("- {}", note.source_path().display());
    }
    emit_project_diagnostic_summary(&maki.diagnostics_without_external_links());
    Ok(web::serve_project(
        maki,
        project_root,
        &options.host,
        options.port,
        config_overrides,
        metrics,
        options.metrics,
    )?)
}

fn run_git_serve(
    git_config: git_source::GitServeConfig,
    options: ServeOptions,
) -> Result<(), RunError> {
    let config_overrides = MakiConfigOverrides::from_home_redirect(options.index_redirect);
    let metrics = metrics_for_endpoint(&options.metrics);
    let source = git_source::GitSource::new(git_config);
    eprintln!("Preparing git source...");
    let checkout = source.prepare()?;
    eprintln!("Loading git checkout {}...", checkout.commit());
    let maki = source.load_maki(&checkout, &config_overrides, &metrics)?;

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
    Ok(web::serve_with_runtime(
        maki,
        checkout.root().to_path_buf(),
        web::ServeConfig {
            host: &options.host,
            port: options.port,
            config_overrides,
            runtime: web::ServeRuntime::Publish,
            metrics,
            metrics_endpoint: options.metrics,
        },
        move |state| {
            git_source::spawn_updater(updater_source, updater_overrides, state, initial_commit);
        },
    )?)
}

fn metrics_for_endpoint(endpoint: &Option<web::MetricsEndpoint>) -> Metrics {
    if endpoint.is_some() {
        Metrics::enabled()
    } else {
        Metrics::disabled()
    }
}

pub(crate) fn run_serve(source: ServeSource, options: ServeOptions) -> Result<(), RunError> {
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
        .map_err(|_source| MakiError::RootNotFound(left.to_path_buf()))?;
    let right = std::fs::canonicalize(right)
        .map_err(|_source| MakiError::RootNotFound(right.to_path_buf()))?;
    Ok(left == right)
}

fn is_project_source_path(path: &Path, source_root: &Path) -> Result<bool, RunError> {
    let path = std::fs::canonicalize(path)
        .map_err(|_source| MakiError::RootNotFound(path.to_path_buf()))?;
    let Ok(source_root) = std::fs::canonicalize(source_root) else {
        return Ok(false);
    };
    Ok(path.starts_with(source_root))
}

pub(crate) fn run_command(command: Command) -> Result<(), RunError> {
    match command {
        Command::Serve { source, options } => run_serve(source, options),
        Command::Build { file } => run_build(file),
        Command::Lsp => maki_lsp::run_stdio().map_err(|error| RunError::Lsp(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_serve_not_exists() {
        let path = PathBuf::from("./tests/not-exists");

        let error = run_command(Command::Serve {
            source: ServeSource::Path(path.clone()),
            options: ServeOptions::default(),
        })
        .unwrap_err();

        match error {
            RunError::Maki(MakiError::RootNotFound(realpath)) => assert_eq!(realpath, path),
            _ => panic!("Unexpected error: {:?}", error),
        }
    }
}
