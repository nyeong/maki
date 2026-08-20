use super::links::ExternalLinkCheck;
use super::note::{NoteMetadataEntry, collect_recent_entries};
use super::*;
use crate::parser::Date;
use std::path::{Path, PathBuf};
use std::{
    cell::RefCell,
    fs,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct TestProject {
    root: PathBuf,
}

impl Drop for TestProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn temp_project(name: &str) -> TestProject {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("maki-{name}-{}-{nanos}", std::process::id()));

    fs::create_dir_all(&root).unwrap();

    TestProject { root }
}

fn repo_path(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn write_note_with_content(project: &TestProject, path: &str, content: &str) {
    let path = project.root.join(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn write_note(project: &TestProject, path: &str) {
    write_note_with_content(project, path, "");
}

mod config;
mod dates;
mod diagnostics;
mod entries;
mod links;
