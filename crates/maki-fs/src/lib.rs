use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use maki_core::{Error, Maki, MakiConfig, PROJECT_FILE_NAME, ProjectSource};

const MAKI_EXTENSION: &str = "maki";

pub trait ProjectLoadMeter {
    fn record_project_load_phase(&self, phase: &'static str, duration: Duration);
}

struct NoopProjectLoadMeter;

impl ProjectLoadMeter for NoopProjectLoadMeter {
    fn record_project_load_phase(&self, _phase: &'static str, _duration: Duration) {}
}

fn is_ignored_name(name: &OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
        || matches!(name.to_str(), Some("node_modules" | "target"))
}

pub fn is_discoverable_maki_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == MAKI_EXTENSION)
        && path.components().all(|component| match component {
            Component::Normal(name) => !is_ignored_name(name),
            Component::CurDir => true,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => false,
        })
}

fn collect_maki_files(root: &Path, current: &Path, files: &mut Vec<PathBuf>) -> Result<(), Error> {
    let entries = std::fs::read_dir(current)
        .map_err(|_| Error::ReadDirectoryFailed(current.to_path_buf()))?;

    for entry in entries {
        let entry = entry.map_err(|_| Error::ReadDirectoryFailed(current.to_path_buf()))?;
        let file_name = entry.file_name();
        if is_ignored_name(&file_name) {
            continue;
        }

        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|_| Error::ReadDirectoryFailed(current.to_path_buf()))?;

        if file_type.is_dir() {
            collect_maki_files(root, &path, files)?;
        } else if (file_type.is_file() || (file_type.is_symlink() && path.is_file()))
            && path
                .extension()
                .is_some_and(|extension| extension == MAKI_EXTENSION)
        {
            files.push(relative_path(root, &path)?);
        }
    }

    Ok(())
}

/// Lists discoverable Maki files below `root` as sorted, root-relative paths.
///
/// Hidden paths, generated dependency/build directories, and directory symlinks are not
/// traversed. File symlinks are included when their target is a regular `.maki` file.
pub fn list_maki_files(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut files = Vec::new();
    collect_maki_files(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

pub fn find_project_root(start: &Path) -> Result<Option<PathBuf>, Error> {
    let start =
        std::fs::canonicalize(start).map_err(|_| Error::RootNotFound(start.to_path_buf()))?;
    let start_dir = if start.is_file() {
        start
            .parent()
            .ok_or_else(|| Error::InvalidNotePath(start.clone()))?
    } else {
        start.as_path()
    };

    for ancestor in start_dir.ancestors() {
        if ancestor.join(PROJECT_FILE_NAME).is_file() {
            return Ok(Some(ancestor.to_path_buf()));
        }
    }

    Ok(None)
}

pub fn load_project_config(root: &Path) -> Result<MakiConfig, Error> {
    let project_file = root.join(PROJECT_FILE_NAME);

    if !project_file.exists() {
        return Ok(MakiConfig::default());
    }
    if !project_file.is_file() {
        return Err(Error::InvalidProjectFile(
            project_file,
            "expected a regular file".to_string(),
        ));
    }

    let raw = std::fs::read_to_string(&project_file)
        .map_err(|_| Error::ReadProjectFileFailed(project_file.clone()))?;
    MakiConfig::parse(&project_file, &raw)
}

pub fn read_maki_sources(root: &Path) -> Result<BTreeMap<PathBuf, String>, Error> {
    list_maki_files(root)?
        .into_iter()
        .map(|path| {
            let source = std::fs::read_to_string(root.join(&path))
                .map_err(|_| Error::ReadNoteFailed(root.join(&path)))?;
            Ok((path, source))
        })
        .collect()
}

pub fn load_project(root: &Path) -> Result<Maki, Error> {
    load_project_with_config(root, MakiConfig::default())
}

pub fn load_project_with_config(root: &Path, config: MakiConfig) -> Result<Maki, Error> {
    load_project_with_config_metered(root, config, &NoopProjectLoadMeter)
}

pub fn load_project_with_config_metered(
    root: &Path,
    config: MakiConfig,
    metrics: &impl ProjectLoadMeter,
) -> Result<Maki, Error> {
    let snapshot_compile_started = Instant::now();

    if !root.exists() {
        return Err(Error::RootNotFound(root.to_path_buf()));
    }
    if !root.is_dir() {
        return Err(Error::RootNotDirectory(root.to_path_buf()));
    }

    let root = std::fs::canonicalize(root).map_err(|_| Error::RootNotFound(root.to_path_buf()))?;

    let started = Instant::now();
    let files = list_maki_files(&root)?;
    metrics.record_project_load_phase("list_files", started.elapsed());

    let started = Instant::now();
    let mut project_sources = Vec::with_capacity(files.len());
    for path in files {
        let file = root.join(&path);
        let metadata = file
            .metadata()
            .map_err(|_| Error::NoteNotFound(file.clone()))?;
        if !metadata.is_file() {
            return Err(Error::NoteNotFound(file));
        }

        let canonical_file =
            std::fs::canonicalize(&file).map_err(|_| Error::NoteNotFound(file.clone()))?;
        let modified = metadata.modified().ok();
        match std::fs::read_to_string(&canonical_file) {
            Ok(source) => project_sources.push(ProjectSource::loaded(path, source, modified)),
            Err(_) => project_sources.push(ProjectSource::read_failed(path, modified)),
        }
    }
    metrics.record_project_load_phase("load_notes", started.elapsed());

    let started = Instant::now();
    let mut maki = Maki::compile(root, config, project_sources);
    metrics.record_project_load_phase("compile", started.elapsed());
    maki.extend_snapshot_compile_duration(snapshot_compile_started.elapsed());
    Ok(maki)
}

pub fn render_file_html(maki: &Maki, file: &Path) -> Result<String, Error> {
    let absolute_path =
        std::fs::canonicalize(file).map_err(|_| Error::NoteNotFound(file.to_path_buf()))?;
    let project_path = relative_path(maki.root(), &absolute_path)?;
    maki.render_html(&project_path)
}

fn relative_path(root: &Path, path: &Path) -> Result<PathBuf, Error> {
    path.strip_prefix(root)
        .map_err(|_| Error::InvalidNotePath(path.to_path_buf()))
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discoverable_paths_exclude_hidden_and_generated_components() {
        assert!(is_discoverable_maki_path(Path::new("notes/today.maki")));

        for path in [
            ".hidden.maki",
            ".direnv/flake-inputs/project/README.maki",
            ".git/README.maki",
            ".jj/repo/store.maki",
            "node_modules/package/README.maki",
            "target/generated.maki",
            "notes/today.md",
            "../outside.maki",
        ] {
            assert!(
                !is_discoverable_maki_path(Path::new(path)),
                "{path} must be excluded"
            );
        }
    }
}
