use std::path::{Path, PathBuf};

use super::{Error, MAKI_EXTENSION};

fn collect_maki_files(root: &Path, current: &Path, acc: &mut Vec<PathBuf>) -> Result<(), Error> {
    let entries = std::fs::read_dir(current)
        .map_err(|_s| Error::ReadDirectoryFailed(current.to_path_buf()))?;

    for entry in entries {
        let entry = entry.map_err(|_s| Error::ReadDirectoryFailed(current.to_path_buf()))?;
        let file_name = entry.file_name();
        if file_name.to_string_lossy().starts_with('.') {
            continue;
        }

        let path = entry.path();

        if path.is_dir() {
            collect_maki_files(root, &path, acc)?;
        } else if path.is_file() && path.extension().is_some_and(|ext| ext == MAKI_EXTENSION) {
            acc.push(get_relative_path(root, &path)?);
        }
    }
    Ok(())
}

/// Lists all markdown files in the given directory.
pub(super) fn list_maki_files(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut files = Vec::new();
    collect_maki_files(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

pub(super) fn get_relative_path(root: &Path, path: &Path) -> Result<PathBuf, Error> {
    path.strip_prefix(root)
        .map_err(|_s| Error::InvalidNotePath(path.to_path_buf()))
        .map(Path::to_path_buf)
}
