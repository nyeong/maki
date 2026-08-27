use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::SystemTime;

use maki_core::PROJECT_FILE_NAME;
use maki_core::html;

use super::state::AppState;
use super::{FILE_WATCH_DEBOUNCE, FILE_WATCH_INTERVAL};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

pub(super) type FileSnapshot = BTreeMap<PathBuf, FileStamp>;

fn insert_file_stamp(
    snapshot: &mut FileSnapshot,
    key: PathBuf,
    path: &Path,
) -> Result<(), std::io::Error> {
    let metadata = path.metadata()?;
    snapshot.insert(
        key,
        FileStamp {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        },
    );
    Ok(())
}

pub(super) fn collect_maki_file_snapshot(root: &Path) -> Result<FileSnapshot, std::io::Error> {
    fn collect(root: &Path, current: &Path, acc: &mut FileSnapshot) -> Result<(), std::io::Error> {
        for entry in std::fs::read_dir(current)? {
            let entry = entry?;
            let file_name = entry.file_name();
            if file_name.to_string_lossy().starts_with('.') {
                continue;
            }

            let path = entry.path();
            if path.is_dir() {
                collect(root, &path, acc)?;
                continue;
            }

            if !path.is_file() {
                continue;
            }

            let relative_path = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            let is_maki_note = path.extension().is_some_and(|ext| ext == "maki");
            let is_project_file = relative_path == Path::new(PROJECT_FILE_NAME);
            if !is_maki_note && !is_project_file {
                continue;
            }

            insert_file_stamp(acc, relative_path, &path)?;
        }

        Ok(())
    }

    let mut snapshot = BTreeMap::new();
    collect(root, root, &mut snapshot)?;
    Ok(snapshot)
}

fn collect_runtime_asset_snapshot(snapshot: &mut FileSnapshot) -> Result<(), std::io::Error> {
    for asset in html::runtime_assets() {
        let source_path = asset.source_path();
        if !source_path.is_file() {
            continue;
        }

        let key = PathBuf::from(asset.request_path().trim_start_matches('/'));
        insert_file_stamp(snapshot, key, &source_path)?;
    }

    Ok(())
}

pub(super) fn collect_watched_file_snapshot(root: &Path) -> Result<FileSnapshot, std::io::Error> {
    let mut snapshot = collect_maki_file_snapshot(root)?;
    collect_runtime_asset_snapshot(&mut snapshot)?;
    Ok(snapshot)
}

pub(super) fn collect_watched_project_snapshot(
    project_root: &Path,
    source_root: &Path,
    favicon: Option<&Path>,
) -> Result<FileSnapshot, std::io::Error> {
    let mut snapshot = collect_watched_file_snapshot(source_root)?;

    if project_root != source_root {
        let project_file = project_root.join(PROJECT_FILE_NAME);
        if project_file.is_file() {
            insert_file_stamp(
                &mut snapshot,
                PathBuf::from("__project__").join(PROJECT_FILE_NAME),
                &project_file,
            )?;
        }
    }

    if let Some(favicon) = favicon {
        let favicon_path = project_root.join(favicon);
        if favicon_path.is_file() {
            insert_file_stamp(
                &mut snapshot,
                PathBuf::from("__project__").join(favicon),
                &favicon_path,
            )?;
        }
    }

    Ok(snapshot)
}

pub(super) fn spawn_file_watcher(state: Arc<AppState>) {
    thread::spawn(move || {
        let mut snapshot = match state.watched_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                eprintln!("Failed to initialize file watcher: {}", error);
                FileSnapshot::new()
            }
        };

        loop {
            thread::sleep(FILE_WATCH_INTERVAL);

            let next_snapshot = match state.watched_snapshot() {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    eprintln!("Failed to scan watched files: {}", error);
                    continue;
                }
            };

            if next_snapshot == snapshot {
                continue;
            }

            thread::sleep(FILE_WATCH_DEBOUNCE);

            let stable_snapshot = match state.watched_snapshot() {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    eprintln!("Failed to scan watched files after debounce: {}", error);
                    continue;
                }
            };

            if stable_snapshot != next_snapshot {
                continue;
            }

            match state.reload() {
                Ok(()) => {
                    snapshot = stable_snapshot;
                }
                Err(error) => {
                    eprintln!("Failed to reload maki files: {}", error);
                }
            }
        }
    });
}
