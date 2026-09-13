use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use maki_core::{Error as MakiError, FormatError, format_source};
use maki_fs::{find_project_root, list_maki_files, load_project_config};

use crate::cli::FormatTarget;

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormatOutcome {
    Success,
    CheckFailed,
}

#[derive(Debug)]
pub(crate) enum Error {
    Project(MakiError),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    InvalidTarget {
        path: PathBuf,
        message: &'static str,
    },
    UnsafeProjectSource {
        project_root: PathBuf,
        source_root: PathBuf,
    },
    SourceChanged(PathBuf),
    Format {
        path: PathBuf,
        source: FormatError,
    },
}

impl From<MakiError> for Error {
    fn from(source: MakiError) -> Self {
        Self::Project(source)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Project(error) => error.fmt(formatter),
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} {}: {source}",
                path.display()
            ),
            Self::InvalidTarget { path, message } => {
                write!(
                    formatter,
                    "invalid format target {}: {message}",
                    path.display()
                )
            }
            Self::UnsafeProjectSource {
                project_root,
                source_root,
            } => write!(
                formatter,
                "configured source {} resolves outside project {}",
                source_root.display(),
                project_root.display()
            ),
            Self::SourceChanged(path) => write!(
                formatter,
                "source changed while formatting: {}",
                path.display()
            ),
            Self::Format { path, source } => {
                write!(formatter, "{}: {source}", path.display())
            }
        }
    }
}

struct PlannedChange {
    path: PathBuf,
    original: String,
    formatted: String,
    permissions: Permissions,
}

struct StagedChange {
    path: PathBuf,
    temporary_path: PathBuf,
}

impl Drop for StagedChange {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.temporary_path);
    }
}

pub(crate) fn run(target: FormatTarget, check: bool) -> Result<FormatOutcome, Error> {
    match target {
        FormatTarget::Stdin => run_stdin(check),
        FormatTarget::Path(path) => run_path(&path, check),
    }
}

fn run_stdin(check: bool) -> Result<FormatOutcome, Error> {
    let label = PathBuf::from("<stdin>");
    let mut source = String::new();
    io::stdin()
        .lock()
        .read_to_string(&mut source)
        .map_err(|source| io_error("read", &label, source))?;
    let formatted = format_source(&source).map_err(|source| Error::Format {
        path: label.clone(),
        source,
    })?;

    if check {
        if formatted == source {
            return Ok(FormatOutcome::Success);
        }

        eprintln!("would reformat: {}", label.display());
        return Ok(FormatOutcome::CheckFailed);
    }

    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    stdout
        .write_all(formatted.as_bytes())
        .map_err(|source| io_error("write", Path::new("<stdout>"), source))?;
    stdout
        .flush()
        .map_err(|source| io_error("flush", Path::new("<stdout>"), source))?;
    Ok(FormatOutcome::Success)
}

fn run_path(target: &Path, check: bool) -> Result<FormatOutcome, Error> {
    let paths = resolve_format_paths(target)?;
    let mut changes = Vec::new();

    for path in paths {
        if let Some(change) = plan_change(path)? {
            changes.push(change);
        }
    }

    if check {
        for change in &changes {
            eprintln!("would reformat: {}", change.path.display());
        }
        return Ok(if changes.is_empty() {
            FormatOutcome::Success
        } else {
            FormatOutcome::CheckFailed
        });
    }

    let staged = changes
        .iter()
        .map(stage_change)
        .collect::<Result<Vec<_>, _>>()?;

    for change in &changes {
        ensure_source_unchanged(change)?;
    }

    for change in &staged {
        fs::rename(&change.temporary_path, &change.path)
            .map_err(|source| io_error("replace", &change.path, source))?;
    }

    for change in &changes {
        eprintln!("formatted: {}", change.path.display());
    }

    Ok(FormatOutcome::Success)
}

fn resolve_format_paths(target: &Path) -> Result<Vec<PathBuf>, Error> {
    let metadata = symlink_metadata(target)?;
    if metadata.file_type().is_symlink() {
        return Err(invalid_target(target, "symbolic links are not supported"));
    }
    if metadata.is_file() {
        if target.extension() != Some(OsStr::new("maki")) {
            return Err(invalid_target(target, "expected a .maki file"));
        }
        return Ok(vec![target.to_path_buf()]);
    }
    if !metadata.is_dir() {
        return Err(invalid_target(
            target,
            "expected a regular file or directory",
        ));
    }

    let source_root = resolve_source_root(target)?;
    list_maki_files(&source_root)
        .map(|paths| {
            paths
                .into_iter()
                .map(|path| source_root.join(path))
                .collect()
        })
        .map_err(Error::Project)
}

fn resolve_source_root(target: &Path) -> Result<PathBuf, Error> {
    let target = canonicalize("resolve", target)?;
    let Some(project_root) = find_project_root(&target)? else {
        return Ok(target);
    };

    let config = load_project_config(&project_root)?;
    let configured_source = config.project_source_root(&project_root);
    if target != project_root {
        let Ok(source_root) = fs::canonicalize(&configured_source) else {
            return Ok(target);
        };
        if !target.starts_with(&source_root) {
            return Ok(target);
        }
        ensure_source_in_project(&project_root, &source_root)?;
        return Ok(source_root);
    }

    let source_root = canonicalize("resolve configured source", &configured_source)?;
    ensure_source_in_project(&project_root, &source_root)?;
    Ok(source_root)
}

fn ensure_source_in_project(project_root: &Path, source_root: &Path) -> Result<(), Error> {
    if source_root.starts_with(project_root) {
        Ok(())
    } else {
        Err(Error::UnsafeProjectSource {
            project_root: project_root.to_path_buf(),
            source_root: source_root.to_path_buf(),
        })
    }
}

fn plan_change(path: PathBuf) -> Result<Option<PlannedChange>, Error> {
    let metadata = validate_source_file(&path)?;
    let original = fs::read_to_string(&path).map_err(|source| io_error("read", &path, source))?;
    let formatted = format_source(&original).map_err(|source| Error::Format {
        path: path.clone(),
        source,
    })?;

    if formatted == original {
        return Ok(None);
    }
    let formatted = formatted.into_owned();

    Ok(Some(PlannedChange {
        path,
        original,
        formatted,
        permissions: metadata.permissions(),
    }))
}

fn stage_change(change: &PlannedChange) -> Result<StagedChange, Error> {
    let (mut temporary, temporary_path) = create_temporary_file(&change.path)?;
    let staged = StagedChange {
        path: change.path.clone(),
        temporary_path,
    };

    temporary
        .write_all(change.formatted.as_bytes())
        .map_err(|source| io_error("stage formatted contents for", &change.path, source))?;
    temporary
        .sync_all()
        .map_err(|source| io_error("sync staged contents for", &change.path, source))?;
    fs::set_permissions(&staged.temporary_path, change.permissions.clone())
        .map_err(|source| io_error("preserve permissions for", &change.path, source))?;

    Ok(staged)
}

fn create_temporary_file(target: &Path) -> Result<(File, PathBuf), Error> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    for _ in 0..100 {
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".maki-fmt-{}-{sequence}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(io_error("create a temporary file for", target, source)),
        }
    }

    Err(io_error(
        "create a temporary file for",
        target,
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "temporary file name space exhausted",
        ),
    ))
}

fn ensure_source_unchanged(change: &PlannedChange) -> Result<(), Error> {
    validate_source_file(&change.path)?;
    let current = fs::read_to_string(&change.path)
        .map_err(|source| io_error("re-read", &change.path, source))?;
    if current == change.original {
        Ok(())
    } else {
        Err(Error::SourceChanged(change.path.clone()))
    }
}

fn validate_source_file(path: &Path) -> Result<fs::Metadata, Error> {
    let metadata = symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(invalid_target(path, "symbolic links are not supported"));
    }
    if !metadata.is_file() {
        return Err(invalid_target(path, "expected a regular file"));
    }
    Ok(metadata)
}

fn symlink_metadata(path: &Path) -> Result<fs::Metadata, Error> {
    fs::symlink_metadata(path).map_err(|source| io_error("inspect", path, source))
}

fn canonicalize(operation: &'static str, path: &Path) -> Result<PathBuf, Error> {
    fs::canonicalize(path).map_err(|source| io_error(operation, path, source))
}

fn invalid_target(path: &Path, message: &'static str) -> Error {
    Error::InvalidTarget {
        path: path.to_path_buf(),
        message,
    }
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> Error {
    Error::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}
