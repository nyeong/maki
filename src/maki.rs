//! Maki domain.
//!
//! ### Properties
//!
//! Parser가 해석한 maki 문서의 properties 중 일부에 의미를 담아 활용함
//!
//! 예)
//! - 문서의 `title`을 문서의 제목으로 활용함
//! - 문서의 `publish`를 publish 정책으로 활용함

const MAKI_EXTENSION: &str = "maki";
const MAKI_SOURCE_EXTENSION: &str = ".maki";

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::{
    html::{self, NoteInfo, RenderContext},
    parser,
};

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
fn list_maki_files(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut files = Vec::new();
    collect_maki_files(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct NoteRef {
    canonical_path: PathBuf,
}

pub(crate) struct Maki {
    root: PathBuf,                  // canonical absolute path
    notes: BTreeMap<NoteRef, Note>, // root-relative maki paths
    config: MakiConfig,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Note {
    /// 실제 파일 시스템 절대경로
    absolute_path: PathBuf,

    /// 프로젝트 root 기준 상대경로
    project_path: PathBuf,
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

impl NoteRef {
    fn new(canonical_path: impl AsRef<Path>) -> Self {
        Self {
            canonical_path: canonical_path.as_ref().to_path_buf(),
        }
    }

    fn canonical_path(&self) -> &Path {
        self.canonical_path.as_ref()
    }

    pub(crate) fn web_path(&self) -> String {
        format!("/{}", self.canonical_path().display())
    }
}

impl Note {
    /// 루트로부터 파일까지의 상대경로
    pub(crate) fn source_path(&self) -> &Path {
        self.project_path.as_ref()
    }

    fn title(&self) -> String {
        let content = std::fs::read_to_string(&self.absolute_path).unwrap();
        let parsed = parser::parse(&content);
        parsed.title().unwrap_or(self.file_stem()).to_string()
    }

    pub(crate) fn note_ref(&self) -> NoteRef {
        NoteRef::new(self.canonical_path())
    }

    /// Maki 내부 identity로 쓰는 경로.
    /// as_path에서 확장자를 생략한 것
    fn canonical_path(&self) -> PathBuf {
        let path = self.project_path.with_extension("");
        path.strip_prefix(".").unwrap_or(&path).to_path_buf()
    }

    /// 파일 이름
    fn file_stem(&self) -> &str {
        self.project_path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("")
    }

    fn load(root: impl AsRef<Path>, project_path: impl AsRef<Path>) -> Result<Self, Error> {
        let root = root.as_ref();
        let project_path = project_path.as_ref();
        let absolute_path = root.join(project_path);
        if !absolute_path.exists() || !absolute_path.is_file() {
            return Err(Error::NoteNotFound(absolute_path.to_path_buf()));
        }

        let absolute_path = std::fs::canonicalize(&absolute_path)
            .map_err(|_s| Error::NoteNotFound(absolute_path))?;

        Ok(Self {
            absolute_path,
            project_path: project_path.to_path_buf(),
        })
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum NoteLinkResolution {
    Found(NoteRef),
    Broken,
    Ambiguous,
}

impl Maki {
    fn note(&self, note_ref: &NoteRef) -> Option<&Note> {
        self.notes.get(note_ref)
    }

    pub(crate) fn resolve_note_link(&self, current: &NoteRef, target: &str) -> NoteLinkResolution {
        let target_ref = NoteRef::new(target);

        if self.notes.contains_key(&target_ref) {
            return NoteLinkResolution::Found(target_ref);
        }

        if target.starts_with('#') {
            return NoteLinkResolution::Ambiguous;
        }
        // 2. sibling stem
        if !target.contains('/')
            && let Some(parent) = current.canonical_path().parent()
        {
            let sibling_ref = NoteRef::new(parent.join(target));
            if self.notes.contains_key(&sibling_ref) {
                return NoteLinkResolution::Found(sibling_ref);
            }
        }

        // 3. project-wide file stem
        let mut candidates = self
            .notes
            .keys()
            .filter(|note_ref| {
                note_ref
                    .canonical_path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    == Some(target)
            })
            .cloned()
            .collect::<Vec<_>>();

        match candidates.len() {
            0 => NoteLinkResolution::Broken,
            1 => NoteLinkResolution::Found(candidates.remove(0)),
            _ => NoteLinkResolution::Ambiguous,
        }
    }

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

    pub(crate) fn notes(&self) -> impl Iterator<Item = &Note> {
        self.notes.values()
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

        let files = list_maki_files(&root)?;

        let mut notes = BTreeMap::new();

        for file in &files {
            let note = Note::load(&root, file).unwrap();
            notes.insert(note.note_ref(), note);
        }

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
        let raw = self.get_raw_content(path)?;
        let document = parser::parse(&raw);
        let current = Note::load(&self.root, path)?.note_ref();

        let resolve_note_link = |target: &str| self.resolve_note_link(&current, target);
        let get_note_info = |note_ref: &NoteRef| {
            self.note(note_ref).map(|note| NoteInfo {
                title: note.title(),
            })
        };

        Ok(html::render_document_with_context(
            &document,
            RenderContext::project(&resolve_note_link, &get_note_info),
        ))
    }

    /// Resolves a note path relative to the root directory.
    /// # Example
    /// ```
    /// maki.resolve_note_route("maki.maki"); // => MakiRoute::NoteSource("maki.maki")
    /// maki.resolve_note_route("maki"); // => MakiRoute::NotePage("maki.maki")
    /// ```
    fn resolve_note_route(&self, target: &str) -> Result<MakiRoute, Error> {
        let is_source = target.ends_with(MAKI_SOURCE_EXTENSION);

        let relative_path = if is_source {
            PathBuf::from(target)
        } else {
            PathBuf::from(format!("{target}{MAKI_SOURCE_EXTENSION}"))
        };

        if !self.notes().any(|n| n.project_path == relative_path) {
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
    /// maki.resolve_route("/maki"); // => MakiRoute::NotePage("maki.maki")
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
    use super::*;

    #[test]
    fn note_path() {
        let note = Note::load(".", "docs/use-cases.maki").unwrap();

        assert_eq!(note.source_path(), PathBuf::from("docs/use-cases.maki"));
        assert_eq!(note.canonical_path(), PathBuf::from("docs/use-cases"));
        assert_eq!(note.file_stem(), "use-cases");
        assert_eq!(note.note_ref().web_path(), "/docs/use-cases");
    }

    #[test]
    fn note_ref() {
        let note = Note::load(".", "docs/use-cases.maki").unwrap();
        let ref_ = note.note_ref();
        assert_eq!(ref_.canonical_path(), PathBuf::from("docs/use-cases"));
        assert_eq!(ref_.web_path(), "/docs/use-cases");
    }

    #[test]
    fn resolve_note_link() {
        let maki = Maki::load("docs").unwrap();
        assert_eq!(
            maki.resolve_note_link(&NoteRef::new("index"), "use-cases"),
            NoteLinkResolution::Found(NoteRef::new("use-cases"))
        );

        assert_eq!(
            maki.resolve_note_link(&NoteRef::new("index"), "v0"),
            NoteLinkResolution::Found(NoteRef::new("milestones/v0"))
        );

        assert_eq!(
            maki.resolve_note_link(&NoteRef::new("index"), "milestones/v0"),
            NoteLinkResolution::Found(NoteRef::new("milestones/v0"))
        );
    }
}
