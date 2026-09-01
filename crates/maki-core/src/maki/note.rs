use std::collections::BTreeMap;
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
    Id,
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
    pub(super) title_is_file_stem: bool,
    pub(super) path: String,
    pub(super) source_path: String,
    pub(super) modified: Option<SystemTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentEntry {
    title: String,
    title_is_file_stem: bool,
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
            Self::Id => "id",
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

    pub(crate) fn title_is_file_stem(&self) -> bool {
        self.title_is_file_stem
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

    pub(super) fn metadata_entry_with_title(
        &self,
        title: String,
        title_is_file_stem: bool,
    ) -> NoteMetadataEntry {
        NoteMetadataEntry {
            title,
            title_is_file_stem,
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
            title_is_file_stem: self.title_is_file_stem,
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
    disambiguate_recent_file_stem_titles(&mut entries);
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

fn disambiguate_recent_file_stem_titles(entries: &mut [NoteMetadataEntry]) {
    let mut entries_by_title = BTreeMap::<String, Vec<usize>>::new();
    for (index, entry) in entries.iter().enumerate() {
        if entry.title_is_file_stem {
            entries_by_title
                .entry(entry.title.clone())
                .or_default()
                .push(index);
        }
    }

    for indices in entries_by_title
        .values()
        .filter(|indices| indices.len() > 1)
    {
        let paths = indices
            .iter()
            .map(|&index| recent_label_components(&entries[index]))
            .collect::<Vec<_>>();
        let labels = minimal_unique_suffixes(&paths);

        for (&index, label) in indices.iter().zip(labels) {
            entries[index].title = label;
        }
    }
}

fn recent_label_components(entry: &NoteMetadataEntry) -> Vec<String> {
    let path = Path::new(&entry.source_path).with_extension("");
    let components = path
        .iter()
        .map(|component| component.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    if components.is_empty() {
        vec![entry.title.clone()]
    } else {
        components
    }
}

fn minimal_unique_suffixes(paths: &[Vec<String>]) -> Vec<String> {
    let max_shared_prefix = paths
        .iter()
        .map(Vec::len)
        .min()
        .unwrap_or_default()
        .saturating_sub(1);
    let shared_prefix = (0..max_shared_prefix)
        .take_while(|&index| {
            paths
                .iter()
                .skip(1)
                .all(|path| path[index] == paths[0][index])
        })
        .count();
    let paths = paths
        .iter()
        .map(|path| &path[shared_prefix..])
        .collect::<Vec<_>>();
    let mut suffix_lengths = vec![1; paths.len()];

    loop {
        let labels = paths
            .iter()
            .zip(&suffix_lengths)
            .map(|(path, &suffix_length)| path[path.len() - suffix_length..].join("/"))
            .collect::<Vec<_>>();
        let mut indices_by_label = BTreeMap::<&str, Vec<usize>>::new();
        for (index, label) in labels.iter().enumerate() {
            indices_by_label
                .entry(label.as_str())
                .or_default()
                .push(index);
        }

        let mut expanded = false;
        for indices in indices_by_label
            .values()
            .filter(|indices| indices.len() > 1)
        {
            for &index in indices {
                if suffix_lengths[index] < paths[index].len() {
                    suffix_lengths[index] += 1;
                    expanded = true;
                }
            }
        }

        if !expanded {
            return labels;
        }
    }
}
