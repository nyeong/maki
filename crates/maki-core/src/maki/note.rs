use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::Error;
use super::links::normalize_key;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NoteRef {
    canonical_path: PathBuf,
}

#[derive(Debug, PartialEq)]
pub struct Note {
    /// 실제 파일 시스템 절대경로
    pub(super) absolute_path: PathBuf,

    /// 프로젝트 root 기준 상대경로
    pub(super) project_path: PathBuf,

    pub(super) modified: Option<SystemTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchEntry {
    kind: SearchEntryKind,
    title: String,
    path: String,
    source_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchEntryKind {
    Note,
    File,
    Heading,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SitemapEntry {
    title: String,
    path: String,
    source_path: String,
}

#[derive(Clone)]
pub(super) struct NoteMetadataEntry {
    pub(super) title: String,
    pub(super) path: String,
    pub(super) source_path: String,
    pub(super) modified: Option<SystemTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentEntry {
    title: String,
    path: String,
    modified: Option<SystemTime>,
}

impl SearchEntry {
    pub(super) fn new(
        kind: SearchEntryKind,
        title: impl Into<String>,
        path: impl Into<String>,
        source_path: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            title: title.into(),
            path: path.into(),
            source_path: source_path.into(),
        }
    }

    pub fn kind(&self) -> SearchEntryKind {
        self.kind
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn source_path(&self) -> &str {
        &self.source_path
    }
}

impl SearchEntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::File => "file",
            Self::Heading => "heading",
        }
    }
}

impl SitemapEntry {
    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn source_path(&self) -> &str {
        &self.source_path
    }
}

impl RecentEntry {
    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn modified(&self) -> Option<SystemTime> {
        self.modified
    }
}
impl NoteRef {
    pub fn new(canonical_path: impl AsRef<Path>) -> Self {
        Self {
            canonical_path: canonical_path.as_ref().to_path_buf(),
        }
    }

    pub(super) fn canonical_path(&self) -> &Path {
        self.canonical_path.as_ref()
    }

    pub fn web_path(&self) -> String {
        format!("/{}", self.canonical_path().display())
    }
}

impl Note {
    /// 루트로부터 파일까지의 상대경로
    pub fn source_path(&self) -> &Path {
        self.project_path.as_ref()
    }

    pub(super) fn metadata_entry_with_title(&self, title: String) -> NoteMetadataEntry {
        NoteMetadataEntry {
            title,
            path: self.note_ref().web_path(),
            source_path: self.source_path().display().to_string(),
            modified: self.modified,
        }
    }

    pub fn note_ref(&self) -> NoteRef {
        NoteRef::new(self.canonical_path())
    }

    /// Maki 내부 identity로 쓰는 경로.
    /// as_path에서 확장자를 생략한 것
    pub(super) fn canonical_path(&self) -> PathBuf {
        let path = self.project_path.with_extension("");
        path.strip_prefix(".").unwrap_or(&path).to_path_buf()
    }

    /// 파일 이름
    pub(super) fn file_stem(&self) -> &str {
        self.project_path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("")
    }

    pub(super) fn load(
        root: impl AsRef<Path>,
        project_path: impl AsRef<Path>,
    ) -> Result<Self, Error> {
        let root = root.as_ref();
        let project_path = project_path.as_ref();
        let absolute_path = root.join(project_path);
        let metadata = absolute_path
            .metadata()
            .map_err(|_source| Error::NoteNotFound(absolute_path.to_path_buf()))?;
        if !metadata.is_file() {
            return Err(Error::NoteNotFound(absolute_path.to_path_buf()));
        }

        let absolute_path = std::fs::canonicalize(&absolute_path)
            .map_err(|_s| Error::NoteNotFound(absolute_path))?;

        Ok(Self {
            absolute_path,
            project_path: project_path.to_path_buf(),
            modified: metadata.modified().ok(),
        })
    }
}

impl NoteMetadataEntry {
    pub(super) fn into_search_entry(self) -> SearchEntry {
        SearchEntry::new(
            SearchEntryKind::Note,
            self.title,
            self.path,
            self.source_path,
        )
    }

    pub(super) fn into_sitemap_entry(self) -> SitemapEntry {
        SitemapEntry {
            title: self.title,
            path: self.path,
            source_path: self.source_path,
        }
    }

    pub(super) fn into_recent_entry(self) -> RecentEntry {
        RecentEntry {
            title: self.title,
            path: self.path,
            modified: self.modified,
        }
    }
}

pub(super) fn search_match_rank(title: &str, query: &str) -> Option<(usize, usize, usize)> {
    let normalized_title = normalize_key(title);

    if normalized_title == query {
        return Some((0, 0, title.len()));
    }

    if normalized_title.starts_with(query) {
        return Some((1, 0, title.len()));
    }

    normalized_title
        .find(query)
        .map(|index| (2, index, title.len()))
}
pub(super) fn collect_recent_entries(mut entries: Vec<NoteMetadataEntry>) -> Vec<RecentEntry> {
    entries.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| left.source_path.cmp(&right.source_path))
    });
    entries
        .into_iter()
        .map(NoteMetadataEntry::into_recent_entry)
        .collect()
}
