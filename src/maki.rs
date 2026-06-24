//! Maki domain.
//!
//! Owns note/indexing errors. It does not decide HTTP status codes.

use crate::{parser, renderer};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) enum Error {
    ReadDirectoryFailed(PathBuf),
    ReadNoteFailed(PathBuf),
    InvalidNotePath(PathBuf),
    RootNotFound(PathBuf),
    RootNotDirectory(PathBuf),
    NoteNotFound(PathBuf),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::RootNotFound(path) => {
                write!(f, "Root not found: {}", path.display())
            }
            Error::RootNotDirectory(path) => {
                write!(f, "Root not a directory: {}", path.display())
            }
            Error::ReadDirectoryFailed(path) => {
                write!(f, "Read directory failed: {}", path.display())
            }
            Error::InvalidNotePath(path) => {
                write!(f, "Invalid note path: {}", path.display())
            }
            Error::NoteNotFound(path) => {
                write!(f, "Note not found: {}", path.display(),)
            }
            Error::ReadNoteFailed(path) => {
                write!(f, "Read note failed: {}", path.display())
            }
        }
    }
}

fn collect_markdown_files(
    root: &Path,
    current: &Path,
    acc: &mut Vec<PathBuf>,
) -> Result<(), Error> {
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
            collect_markdown_files(root, &path, acc)?;
        } else if path.is_file() && path.extension().is_some_and(|ext| ext == "md") {
            acc.push(get_relative_path(root, &path)?);
        }
    }
    Ok(())
}

/// Lists all markdown files in the given directory.
fn list_markdown_files(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut files = Vec::new();
    collect_markdown_files(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

pub(crate) struct Maki {
    root: PathBuf,    // canonical absolute path
    notes: Vec<Note>, // root-relative markdown paths
    config: MakiConfig,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Note {
    path: PathBuf,
}

#[derive(Debug, PartialEq)]
pub(crate) struct MakiConfig {
    home_mode: HomeMode,
    publish_policy: PublishPolicy,
}

impl MakiConfig {
    pub(crate) fn home_mode(&self) -> &HomeMode {
        &self.home_mode
    }
}

impl Default for MakiConfig {
    fn default() -> Self {
        Self {
            home_mode: HomeMode::Redirect("/README".to_string()),
            publish_policy: PublishPolicy::PublishAll,
        }
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum PublishPolicy {
    PublishAll,
    // TODO: TaggedOnly: publish 설정한 파일만 접근 가능하게 하기,
}

#[derive(Debug, PartialEq)]
pub(crate) enum HomeMode {
    Redirect(String),
}

fn get_relative_path(root: &Path, path: &Path) -> Result<PathBuf, Error> {
    path.strip_prefix(root)
        .map_err(|_s| Error::InvalidNotePath(path.to_path_buf()))
        .map(Path::to_path_buf)
}

#[derive(Debug, PartialEq)]
pub(crate) enum MakiRoute {
    Home,
    NotePage(PathBuf),
    NoteSource(PathBuf),
}

impl Note {
    pub(crate) fn path(&self) -> &Path {
        self.path.as_ref()
    }

    fn load(_root: impl AsRef<Path>, relative_path: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self {
            path: relative_path.as_ref().to_path_buf(),
        })
    }
}

impl Maki {
    pub(crate) fn get_raw_content(&self, path: &Path) -> Result<String, Error> {
        let path = self.root.join(path);

        if !path.exists() || !path.is_file() {
            return Err(Error::NoteNotFound(path));
        }

        std::fs::read_to_string(&path).map_err(|_source| Error::ReadNoteFailed(path))
    }

    pub(crate) fn config(&self) -> &MakiConfig {
        &self.config
    }

    pub(crate) fn notes(&self) -> &[Note] {
        &self.notes
    }

    pub(crate) fn notes_len(&self) -> usize {
        self.notes.len()
    }

    pub(crate) fn load_with_config(root: &Path, config: MakiConfig) -> Result<Self, Error> {
        if !root.exists() {
            return Err(Error::RootNotFound(root.to_path_buf()));
        }
        if !root.is_dir() {
            return Err(Error::RootNotDirectory(root.to_path_buf()));
        }

        let root =
            std::fs::canonicalize(root).map_err(|_source| Error::RootNotFound(root.to_owned()))?;

        let files = list_markdown_files(&root)?;
        let notes = files
            .into_iter()
            .map(|file| Note::load(&root, &file))
            .collect::<Result<Vec<Note>, _>>()?;

        Ok(Self {
            root,
            notes,
            config,
        })
    }

    // root: absolute or relative to the project directory
    pub(crate) fn load(root: impl AsRef<Path>) -> Result<Self, Error> {
        Self::load_with_config(root.as_ref(), MakiConfig::default())
    }

    pub(crate) fn render_html(&self, path: &Path) -> Result<String, Error> {
        let path = self.root.join(path);
        if !path.exists() {
            return Err(Error::NoteNotFound(path));
        }
        if !path.is_file() {
            return Err(Error::NoteNotFound(path));
        }

        let mut html =
            String::from("<!doctype html><html><head><meta charset=\"utf-8\"></head><body><pre>");
        let content =
            std::fs::read_to_string(&path).map_err(|_source| Error::ReadNoteFailed(path))?;
        html.push_str(&renderer::render_html(&parser::parse(&content), |query| {
            if query.target().is_empty() {
                renderer::NoteLinkResult::Broken
            } else {
                renderer::NoteLinkResult::Found {
                    href: format!("/{}", query.target()),
                    label: query.target().to_string(),
                }
            }
        }));
        html.push_str("</pre></body></html>");
        Ok(html)
    }

    /// Resolves a note path relative to the root directory.
    /// # Example
    /// ```
    /// maki.resolve_note_route("maki.md"); // => MakiRoute::NoteSource("maki.md")
    /// maki.resolve_note_route("maki"); // => MakiRoute::NotePage("maki.md")
    /// ```
    fn resolve_note_route(&self, target: &str) -> Result<MakiRoute, Error> {
        let is_source = target.ends_with(".md");

        let relative_path = if is_source {
            PathBuf::from(target)
        } else {
            PathBuf::from(format!("{target}.md"))
        };

        if !self.notes.iter().any(|n| n.path == relative_path) {
            return Err(Error::NoteNotFound(relative_path));
        }

        match is_source {
            true => Ok(MakiRoute::NoteSource(relative_path)),
            false => Ok(MakiRoute::NotePage(relative_path)),
        }
    }

    /// Resolves a page path relative to the root directory.
    /// # Example
    /// ```text
    /// maki.resolve_route("/maki"); // => MakiRoute::NotePage("maki.md")
    /// ```
    pub(crate) fn resolve_route(&self, target: &str) -> Result<MakiRoute, Error> {
        let target = target.strip_prefix('/').unwrap_or(target);

        if target.is_empty() {
            return Ok(MakiRoute::Home);
        }

        self.resolve_note_route(target)
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, Maki, MakiRoute, list_markdown_files};
    use std::path::PathBuf;

    #[test]
    fn test_list_markdown_files() {
        let path = PathBuf::from("./tests/fixtures/basic-maki-project");
        let files = list_markdown_files(&path).unwrap();
        assert_eq!(
            files,
            vec![
                PathBuf::from("README.md"),
                PathBuf::from("daily.md"),
                PathBuf::from("nested/nested.md"),
                PathBuf::from("nested/한글.md"),
            ]
        );
    }

    #[test]
    fn test_resolve_route() {
        let maki = Maki::load("tests/fixtures/basic-maki-project").unwrap();
        assert_eq!(maki.resolve_route("/").unwrap(), MakiRoute::Home);
        assert!(matches!(
            maki.resolve_route("/favicon.ico"),
            Err(Error::NoteNotFound { .. })
        ));
        assert_eq!(
            maki.resolve_route("/daily.md").unwrap(),
            MakiRoute::NoteSource(PathBuf::from("daily.md"))
        );
        assert_eq!(
            maki.resolve_route("/README").unwrap(),
            MakiRoute::NotePage(PathBuf::from("README.md"))
        );
        assert_eq!(
            maki.resolve_route("/nested/nested.md").unwrap(),
            MakiRoute::NoteSource(PathBuf::from("nested/nested.md"))
        );
        assert_eq!(
            maki.resolve_route("/nested/한글").unwrap(),
            MakiRoute::NotePage(PathBuf::from("nested/한글.md"))
        );
    }

    // #[test]
    // fn resolve_wikilink_absolute() {
    //     let files = vec![PathBuf::from("notes/foo.md"), PathBuf::from("notes/bar.md")];
    //     assert_eq!(
    //         resolve_wikilink(&files, &PathBuf::from("notes/bar.md"), "notes/foo"),
    //         WikiLinkTarget::Found(PathBuf::from("notes/foo.md"))
    //     );
    //     assert_eq!(
    //         resolve_wikilink(&files, &PathBuf::from("notes/bar.md"), "notes/foo/foo"),
    //         WikiLinkTarget::Broken
    //     );
    // }

    // #[test]
    // fn resolve_wikilink_same_directory() {
    //     let files = vec![PathBuf::from("notes/foo.md"), PathBuf::from("notes/bar.md")];
    //     assert_eq!(
    //         resolve_wikilink(&files, &PathBuf::from("notes/foo.md"), "bar"),
    //         WikiLinkTarget::Found(PathBuf::from("notes/bar.md"))
    //     );
    //     assert_eq!(
    //         resolve_wikilink(&files, &PathBuf::from("notes/foo.md"), "foo"),
    //         WikiLinkTarget::Broken
    //     );
    // }

    // #[test]
    // fn resolve_wikilink_project_wide() {}
}
