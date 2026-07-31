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
pub(crate) const PROJECT_FILE_NAME: &str = "maki.toml";

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
    ReadProjectFileFailed(PathBuf),
    InvalidProjectFile(PathBuf, String),
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
            Error::ReadProjectFileFailed(path) => {
                write!(f, "Read project file failed: {}", path.display())
            }
            Error::InvalidProjectFile(path, message) => {
                write!(f, "Invalid project file {}: {}", path.display(), message)
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
    index: NoteIndex,
    config: MakiConfig,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Note {
    /// 실제 파일 시스템 절대경로
    absolute_path: PathBuf,

    /// 프로젝트 root 기준 상대경로
    project_path: PathBuf,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct MakiConfig {
    project_title: Option<String>,
    home_mode: HomeMode,
    publish_policy: PublishPolicy,
}

impl MakiConfig {
    pub(crate) fn project_title(&self) -> Option<&str> {
        self.project_title.as_deref()
    }

    pub(crate) fn home_mode(&self) -> &HomeMode {
        &self.home_mode
    }

    pub(crate) fn load_project(root: &Path) -> Result<Self, Error> {
        let project_file = root.join(PROJECT_FILE_NAME);

        if !project_file.exists() {
            return Ok(Self::default());
        }
        if !project_file.is_file() {
            return Err(Error::InvalidProjectFile(
                project_file,
                "expected a regular file".to_string(),
            ));
        }

        let raw = std::fs::read_to_string(&project_file)
            .map_err(|_source| Error::ReadProjectFileFailed(project_file.clone()))?;
        let project = ProjectToml::parse(&project_file, &raw)?;

        let mut config = Self {
            project_title: project.title,
            ..Default::default()
        };
        if let Some(home) = project.home {
            config.set_home_note_ref(home);
        }
        Ok(config)
    }

    pub(crate) fn set_home_redirect(&mut self, path: impl Into<String>) {
        self.home_mode = HomeMode::Redirect(path.into());
    }

    fn set_home_note_ref(&mut self, note_ref: impl AsRef<str>) {
        self.set_home_redirect(note_ref_to_redirect_path(note_ref.as_ref()));
    }
}

impl Default for MakiConfig {
    fn default() -> Self {
        Self {
            project_title: None,
            home_mode: HomeMode::Redirect("/README".to_string()),
            publish_policy: PublishPolicy::PublishAll,
        }
    }
}

#[derive(Debug, PartialEq, Clone, Default)]
pub(crate) struct MakiConfigOverrides {
    home_redirect: Option<String>,
}

impl MakiConfigOverrides {
    pub(crate) fn from_home_redirect(home_redirect: Option<String>) -> Self {
        Self { home_redirect }
    }

    pub(crate) fn apply_to(&self, config: &mut MakiConfig) {
        if let Some(home_redirect) = &self.home_redirect {
            config.set_home_redirect(home_redirect.clone());
        }
    }
}

#[derive(Default)]
struct ProjectToml {
    title: Option<String>,
    home: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum TomlSection {
    Project,
    Other,
}

impl ProjectToml {
    fn parse(path: &Path, raw: &str) -> Result<Self, Error> {
        let mut project = Self::default();
        let mut section = TomlSection::Other;

        for (index, raw_line) in raw.lines().enumerate() {
            let line_number = index + 1;
            let line = trim_toml_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }

            if line.starts_with('[') {
                section = parse_toml_section(path, line_number, line)?;
                continue;
            }

            if section != TomlSection::Project {
                continue;
            }

            let Some((raw_key, raw_value)) = line.split_once('=') else {
                return Err(invalid_project_file(
                    path,
                    line_number,
                    "expected key = value",
                ));
            };
            match raw_key.trim() {
                "title" => {
                    project.title = Some(parse_toml_string_value(
                        path,
                        line_number,
                        raw_value.trim(),
                    )?)
                }
                "home" => {
                    project.home = Some(parse_toml_string_value(
                        path,
                        line_number,
                        raw_value.trim(),
                    )?)
                }
                _ => continue,
            }
        }

        Ok(project)
    }
}

fn note_ref_to_redirect_path(note_ref: &str) -> String {
    if note_ref.starts_with('/') {
        note_ref.to_string()
    } else {
        format!("/{note_ref}")
    }
}

fn parse_toml_section(path: &Path, line_number: usize, line: &str) -> Result<TomlSection, Error> {
    if !line.ends_with(']') {
        return Err(invalid_project_file(
            path,
            line_number,
            "unterminated table header",
        ));
    }

    let section = line
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or("")
        .trim();

    match section {
        "project" => Ok(TomlSection::Project),
        _ => Ok(TomlSection::Other),
    }
}

fn trim_toml_comment(line: &str) -> &str {
    let mut in_basic_string = false;
    let mut in_literal_string = false;
    let mut escaped = false;

    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if in_basic_string => escaped = true,
            '"' if !in_literal_string => in_basic_string = !in_basic_string,
            '\'' if !in_basic_string => in_literal_string = !in_literal_string,
            '#' if !in_basic_string && !in_literal_string => return &line[..index],
            _ => {}
        }
    }

    line
}

fn parse_toml_string_value(path: &Path, line_number: usize, raw: &str) -> Result<String, Error> {
    let Some(quote) = raw.chars().next() else {
        return Err(invalid_project_file(
            path,
            line_number,
            "expected string value",
        ));
    };

    match quote {
        '"' => parse_basic_toml_string(path, line_number, raw),
        '\'' => parse_literal_toml_string(path, line_number, raw),
        _ => Err(invalid_project_file(
            path,
            line_number,
            "expected string value",
        )),
    }
}

fn parse_literal_toml_string(path: &Path, line_number: usize, raw: &str) -> Result<String, Error> {
    let rest = &raw[1..];
    let Some(end) = rest.find('\'') else {
        return Err(invalid_project_file(
            path,
            line_number,
            "unterminated literal string",
        ));
    };
    let value = &rest[..end];
    reject_trailing_toml_string_content(path, line_number, &rest[end + 1..])?;
    Ok(value.to_string())
}

fn parse_basic_toml_string(path: &Path, line_number: usize, raw: &str) -> Result<String, Error> {
    let mut value = String::new();
    let mut escaped = false;

    for (index, ch) in raw[1..].char_indices() {
        if escaped {
            match ch {
                'n' => value.push('\n'),
                'r' => value.push('\r'),
                't' => value.push('\t'),
                '"' => value.push('"'),
                '\\' => value.push('\\'),
                _ => {
                    return Err(invalid_project_file(
                        path,
                        line_number,
                        "unsupported string escape",
                    ));
                }
            }
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '"' => {
                reject_trailing_toml_string_content(path, line_number, &raw[index + 2..])?;
                return Ok(value);
            }
            _ => value.push(ch),
        }
    }

    Err(invalid_project_file(
        path,
        line_number,
        "unterminated string",
    ))
}

fn reject_trailing_toml_string_content(
    path: &Path,
    line_number: usize,
    trailing: &str,
) -> Result<(), Error> {
    if trailing.trim().is_empty() {
        Ok(())
    } else {
        Err(invalid_project_file(
            path,
            line_number,
            "unexpected content after string value",
        ))
    }
}

fn invalid_project_file(path: &Path, line_number: usize, message: &str) -> Error {
    Error::InvalidProjectFile(path.to_path_buf(), format!("line {line_number}: {message}"))
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum PublishPolicy {
    PublishAll,
    // TODO: TaggedOnly: publish 설정한 파일만 접근 가능하게 하기,
}

#[derive(Debug, PartialEq, Clone)]
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
        let content = match std::fs::read_to_string(&self.absolute_path) {
            Ok(content) => content,
            Err(_) => return self.file_stem().to_string(),
        };
        let parsed = parser::parse(&content);
        parsed
            .document
            .title()
            .unwrap_or(self.file_stem())
            .to_string()
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

#[derive(Default)]
struct NoteIndex {
    exact_paths: BTreeMap<PathBuf, NoteRef>,
    normalized_paths: BTreeMap<String, Vec<NoteRef>>,
    normalized_stems: BTreeMap<String, Vec<NoteRef>>,
    sibling_normalized_stems: BTreeMap<(PathBuf, String), Vec<NoteRef>>,
}

impl NoteIndex {
    fn build<'a>(note_refs: impl Iterator<Item = &'a NoteRef>) -> Self {
        let mut index = Self::default();

        for note_ref in note_refs {
            index.insert(note_ref);
        }

        index
    }

    fn insert(&mut self, note_ref: &NoteRef) {
        self.exact_paths
            .insert(note_ref.canonical_path().to_path_buf(), note_ref.clone());
        push_candidate(
            &mut self.normalized_paths,
            normalize_path(note_ref.canonical_path()),
            note_ref,
        );

        let Some(stem) = note_ref
            .canonical_path()
            .file_name()
            .and_then(|name| name.to_str())
            .map(normalize_key)
        else {
            return;
        };

        push_candidate(&mut self.normalized_stems, stem.clone(), note_ref);

        let parent = note_ref
            .canonical_path()
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf();
        push_candidate(&mut self.sibling_normalized_stems, (parent, stem), note_ref);
    }

    fn exact_path(&self, target: &Path) -> Option<NoteRef> {
        self.exact_paths.get(target).cloned()
    }

    fn resolve_normalized_path(&self, target: &Path) -> Option<NoteLinkResolution> {
        resolve_candidates(self.normalized_paths.get(&normalize_path(target)))
    }

    fn resolve_sibling_stem(&self, current: &NoteRef, target: &str) -> Option<NoteLinkResolution> {
        let parent = current.canonical_path().parent()?.to_path_buf();
        let key = (parent, normalize_key(target));

        resolve_candidates(self.sibling_normalized_stems.get(&key))
    }

    fn resolve_project_stem(&self, target: &str) -> Option<NoteLinkResolution> {
        resolve_candidates(self.normalized_stems.get(&normalize_key(target)))
    }
}

fn push_candidate<K>(map: &mut BTreeMap<K, Vec<NoteRef>>, key: K, note_ref: &NoteRef)
where
    K: Ord,
{
    map.entry(key).or_default().push(note_ref.clone());
}

fn normalize_path(path: &Path) -> String {
    normalize_key(&path.to_string_lossy())
}

fn normalize_key(key: &str) -> String {
    key.to_lowercase()
}

fn resolve_candidates(candidates: Option<&Vec<NoteRef>>) -> Option<NoteLinkResolution> {
    let candidates = candidates?;

    match candidates.as_slice() {
        [] => None,
        [note_ref] => Some(NoteLinkResolution::Found(note_ref.clone())),
        _ => Some(NoteLinkResolution::Ambiguous),
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum NoteLinkResolution {
    Found(NoteRef),
    Broken,
    Ambiguous,
}

impl Maki {
    pub(crate) fn find_project_root(start: &Path) -> Result<Option<PathBuf>, Error> {
        let start = std::fs::canonicalize(start)
            .map_err(|_source| Error::RootNotFound(start.to_owned()))?;
        let start_dir = if start.is_file() {
            start
                .parent()
                .ok_or_else(|| Error::InvalidNotePath(start.clone()))?
        } else {
            start.as_path()
        };

        for ancestor in start_dir.ancestors() {
            if ancestor.join(PROJECT_FILE_NAME).is_file() {
                return Ok(Some(ancestor.to_path_buf()));
            }
        }

        Ok(None)
    }

    fn note(&self, note_ref: &NoteRef) -> Option<&Note> {
        self.notes.get(note_ref)
    }

    pub(crate) fn resolve_note_link(&self, current: &NoteRef, target: &str) -> NoteLinkResolution {
        if let Some(note_ref) = self.index.exact_path(Path::new(target)) {
            return NoteLinkResolution::Found(note_ref);
        }

        if target.starts_with('#') {
            return NoteLinkResolution::Ambiguous;
        }

        if target.contains('/')
            && let Some(resolution) = self.index.resolve_normalized_path(Path::new(target))
        {
            return resolution;
        }

        if !target.contains('/') {
            if let Some(resolution) = self.index.resolve_sibling_stem(current, target) {
                return resolution;
            }
            if let Some(resolution) = self.index.resolve_project_stem(target) {
                return resolution;
            }
        }

        NoteLinkResolution::Broken
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

    pub(crate) fn root(&self) -> &Path {
        &self.root
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
            let note = Note::load(&root, file)?;
            notes.insert(note.note_ref(), note);
        }
        let index = NoteIndex::build(notes.keys());

        Ok(Self {
            root,
            notes,
            index,
            config,
        })
    }

    // root: absolute or relative to the project directory
    #[allow(dead_code)]
    pub(crate) fn load(root: impl AsRef<Path>) -> Result<Self, Error> {
        Self::load_with_config(root.as_ref(), MakiConfig::default())
    }

    pub(crate) fn render_html(&self, path: &Path) -> Result<String, Error> {
        let raw = self.get_raw_content(path)?;
        let parsed = parser::parse(&raw);
        let current = Note::load(&self.root, path)?.note_ref();

        let resolve_note_link = |target: &str| self.resolve_note_link(&current, target);
        let get_note_info = |note_ref: &NoteRef| {
            self.note(note_ref).map(|note| NoteInfo {
                title: note.title(),
            })
        };

        Ok(html::render_document_with_context(
            &parsed.document,
            RenderContext::project(&resolve_note_link, &get_note_info),
        ))
    }

    pub(crate) fn render_file_html(&self, file: &Path) -> Result<String, Error> {
        let absolute_path =
            std::fs::canonicalize(file).map_err(|_source| Error::NoteNotFound(file.to_owned()))?;
        let project_path = get_relative_path(&self.root, &absolute_path)?;

        self.render_html(&project_path)
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
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
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

    fn write_note(project: &TestProject, path: &str) {
        let path = project.root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, "").unwrap();
    }

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

    #[test]
    fn resolve_note_link_uses_case_insensitive_path_lookup() {
        let project = temp_project("case-insensitive-path");
        write_note(&project, "milestones/v0.maki");
        write_note(&project, "index.maki");

        let maki = Maki::load(&project.root).unwrap();

        assert_eq!(
            maki.resolve_note_link(&NoteRef::new("index"), "Milestones/V0"),
            NoteLinkResolution::Found(NoteRef::new("milestones/v0"))
        );
    }

    #[test]
    fn resolve_note_link_uses_case_insensitive_sibling_stem_lookup() {
        let project = temp_project("case-insensitive-sibling");
        write_note(&project, "notes/devenv.maki");
        write_note(&project, "notes/nix.maki");

        let maki = Maki::load(&project.root).unwrap();

        assert_eq!(
            maki.resolve_note_link(&NoteRef::new("notes/devenv"), "Nix"),
            NoteLinkResolution::Found(NoteRef::new("notes/nix"))
        );
    }

    #[test]
    fn resolve_note_link_prefers_sibling_stem_before_project_wide_stem() {
        let project = temp_project("sibling-before-project-stem");
        write_note(&project, "notes/page.maki");
        write_note(&project, "notes/nix.maki");
        write_note(&project, "other/Nix.maki");

        let maki = Maki::load(&project.root).unwrap();

        assert_eq!(
            maki.resolve_note_link(&NoteRef::new("notes/page"), "NIX"),
            NoteLinkResolution::Found(NoteRef::new("notes/nix"))
        );
    }

    #[test]
    fn resolve_note_link_reports_case_insensitive_stem_ambiguity() {
        let project = temp_project("case-insensitive-stem-ambiguity");
        write_note(&project, "start.maki");
        write_note(&project, "alpha/nix.maki");
        write_note(&project, "beta/NIX.maki");

        let maki = Maki::load(&project.root).unwrap();

        assert_eq!(
            maki.resolve_note_link(&NoteRef::new("start"), "Nix"),
            NoteLinkResolution::Ambiguous
        );
    }

    #[test]
    fn resolve_note_link_preserves_exact_path_priority() {
        let project = temp_project("exact-before-sibling");
        write_note(&project, "nix.maki");
        write_note(&project, "notes/page.maki");
        write_note(&project, "notes/nix.maki");

        let maki = Maki::load(&project.root).unwrap();

        assert_eq!(
            maki.resolve_note_link(&NoteRef::new("notes/page"), "nix"),
            NoteLinkResolution::Found(NoteRef::new("nix"))
        );
    }
}
