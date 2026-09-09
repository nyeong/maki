use super::note::{NoteMetadataEntry, collect_recent_entries};
use super::*;
use crate::parser::Date;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct TestProject {
    root: PathBuf,
    sources: BTreeMap<PathBuf, TestSource>,
}

struct TestSource {
    content: Option<String>,
    modified: Option<SystemTime>,
}

impl TestProject {
    fn add_source(&mut self, path: &str, content: &str) {
        self.sources.insert(
            PathBuf::from(path),
            TestSource {
                content: Some(content.to_string()),
                modified: None,
            },
        );
    }

    fn add_empty_source(&mut self, path: &str) {
        self.add_source(path, "");
    }

    fn add_source_with_modified(&mut self, path: &str, content: &str, modified: SystemTime) {
        self.sources.insert(
            PathBuf::from(path),
            TestSource {
                content: Some(content.to_string()),
                modified: Some(modified),
            },
        );
    }

    fn add_read_failure(&mut self, path: &str) {
        self.sources.insert(
            PathBuf::from(path),
            TestSource {
                content: None,
                modified: None,
            },
        );
    }

    fn compile(&self) -> Maki {
        let sources = self
            .sources
            .iter()
            .map(|(path, source)| match &source.content {
                Some(content) => {
                    ProjectSource::loaded(path.clone(), content.clone(), source.modified)
                }
                None => ProjectSource::read_failed(path.clone(), source.modified),
            });

        Maki::compile(self.root.clone(), MakiConfig::default(), sources)
    }
}

fn test_project(name: &str) -> TestProject {
    TestProject {
        root: PathBuf::from("test-projects").join(name),
        sources: BTreeMap::new(),
    }
}

mod config;
mod dates;
mod diagnostics;
mod entries;
mod links;
mod runtime;
