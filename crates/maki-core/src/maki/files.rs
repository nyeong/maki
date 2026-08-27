use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use super::{Error, MAKI_EXTENSION};

fn is_ignored_name(name: &OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
        || matches!(name.to_str(), Some("node_modules" | "target"))
}

/// Returns whether a root-relative path is eligible for Maki project discovery.
pub fn is_discoverable_maki_path(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == MAKI_EXTENSION)
        && path.components().all(|component| match component {
            Component::Normal(name) => !is_ignored_name(name),
            Component::CurDir => true,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => false,
        })
}

fn collect_maki_files(root: &Path, current: &Path, acc: &mut Vec<PathBuf>) -> Result<(), Error> {
    let entries = std::fs::read_dir(current)
        .map_err(|_s| Error::ReadDirectoryFailed(current.to_path_buf()))?;

    for entry in entries {
        let entry = entry.map_err(|_s| Error::ReadDirectoryFailed(current.to_path_buf()))?;
        let file_name = entry.file_name();
        if is_ignored_name(&file_name) {
            continue;
        }

        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|_s| Error::ReadDirectoryFailed(current.to_path_buf()))?;

        if file_type.is_dir() {
            collect_maki_files(root, &path, acc)?;
        } else if (file_type.is_file() || (file_type.is_symlink() && path.is_file()))
            && path.extension().is_some_and(|ext| ext == MAKI_EXTENSION)
        {
            acc.push(get_relative_path(root, &path)?);
        }
    }
    Ok(())
}

/// Lists discoverable Maki files below `root` as sorted, root-relative paths.
///
/// Hidden paths, generated dependency/build directories, and directory symlinks are not
/// traversed. This is the canonical project discovery rule for build, serve, and LSP.
pub fn list_maki_files(root: &Path) -> Result<Vec<PathBuf>, Error> {
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
