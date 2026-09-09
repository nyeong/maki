use super::note::{NoteMetadataEntry, collect_recent_entries};
use super::*;
use crate::parser::Date;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct TestProject {
    root: PathBuf,
    sources: RefCell<BTreeMap<PathBuf, TestSource>>,
}

struct TestSource {
    content: Option<String>,
    modified: Option<SystemTime>,
}

impl TestProject {
    fn compile(&self) -> Maki {
        let sources = self
            .sources
            .borrow()
            .iter()
            .map(|(path, source)| match &source.content {
                Some(content) => {
                    ProjectSource::loaded(path.clone(), content.clone(), source.modified)
                }
                None => ProjectSource::read_failed(path.clone(), source.modified),
            })
            .collect::<Vec<_>>();

        Maki::compile(self.root.clone(), MakiConfig::default(), sources)
    }
}

fn test_project(name: &str) -> TestProject {
    TestProject {
        root: PathBuf::from("test-projects").join(name),
        sources: RefCell::new(BTreeMap::new()),
    }
}

fn add_source(project: &TestProject, path: &str, content: &str) {
    project.sources.borrow_mut().insert(
        PathBuf::from(path),
        TestSource {
            content: Some(content.to_string()),
            modified: None,
        },
    );
}

fn add_empty_source(project: &TestProject, path: &str) {
    add_source(project, path, "");
}

fn add_source_with_modified(
    project: &TestProject,
    path: &str,
    content: &str,
    modified: SystemTime,
) {
    project.sources.borrow_mut().insert(
        PathBuf::from(path),
        TestSource {
            content: Some(content.to_string()),
            modified: Some(modified),
        },
    );
}

fn add_read_failure(project: &TestProject, path: &str) {
    project.sources.borrow_mut().insert(
        PathBuf::from(path),
        TestSource {
            content: None,
            modified: None,
        },
    );
}

mod config;
mod dates;
mod diagnostics;
mod entries;
mod links;
mod runtime;
