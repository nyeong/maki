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
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

use crate::{
    html::{self, AssetMode, NoteInfo, RenderContext},
    metrics::Metrics,
    parser::{self, BlockKind, Date, DateRange, DateStamp, DateStampKind, Inline},
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
    #[allow(dead_code)]
    date_index: DateIndex,
    external_links: Vec<ExternalLinkRef>,
    search_entries: Vec<SearchEntry>,
    config: MakiConfig,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Note {
    /// 실제 파일 시스템 절대경로
    absolute_path: PathBuf,

    /// 프로젝트 root 기준 상대경로
    project_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchEntry {
    title: String,
    path: String,
    source_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ExternalLinkRef {
    source_path: PathBuf,
    target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct DateIndex {
    by_date: BTreeMap<Date, Vec<DateBacklink>>,
    index_by_date: BTreeMap<Date, Vec<DateBacklink>>,
    occurrences: BTreeMap<String, DateOccurrence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DateOccurrence {
    id: String,
    source_path: PathBuf,
    note_ref: NoteRef,
    note_title: String,
    origin: DateOrigin,
    marker: DateMarker,
    context: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DateOrigin {
    Inline,
    Property { key: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DateMarker {
    Single {
        kind: DateStampKind,
        date: Date,
        raw: String,
    },
    Range {
        kind: DateStampKind,
        start: Date,
        end: Date,
        raw: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DateBacklink {
    occurrence_id: String,
    relation: DateRelation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DateRelation {
    Single,
    Range,
    RangeStart,
    RangeMiddle,
    RangeEnd,
}

#[allow(dead_code)]
impl DateIndex {
    fn insert_occurrence(&mut self, occurrence: DateOccurrence) {
        let id = occurrence.id.clone();

        match &occurrence.marker {
            DateMarker::Single { date, .. } => {
                self.push_backlink(*date, &id, DateRelation::Single);
            }
            DateMarker::Range { start, end, .. } => {
                let mut date = *start;
                loop {
                    let relation = if start == end {
                        DateRelation::Range
                    } else if date == *start {
                        DateRelation::RangeStart
                    } else if date == *end {
                        DateRelation::RangeEnd
                    } else {
                        DateRelation::RangeMiddle
                    };
                    self.push_backlink(date, &id, relation);

                    if date == *end {
                        break;
                    }
                    let Some(next) = date.next_day() else {
                        break;
                    };
                    date = next;
                }
            }
        }

        self.occurrences.insert(id, occurrence);
    }

    fn push_backlink(&mut self, date: Date, occurrence_id: &str, relation: DateRelation) {
        let backlink = DateBacklink {
            occurrence_id: occurrence_id.to_string(),
            relation,
        };

        self.by_date.entry(date).or_default().push(backlink.clone());
        if relation.is_indexed() {
            self.index_by_date.entry(date).or_default().push(backlink);
        }
    }

    fn sort_backlinks(&mut self) {
        for backlinks in self.by_date.values_mut() {
            backlinks.sort_by_key(|backlink| backlink.relation.priority());
        }
        for backlinks in self.index_by_date.values_mut() {
            backlinks.sort_by_key(|backlink| backlink.relation.priority());
        }
    }

    pub(crate) fn dates(&self) -> impl DoubleEndedIterator<Item = (&Date, &[DateBacklink])> {
        self.index_by_date
            .iter()
            .map(|(date, backlinks)| (date, backlinks.as_slice()))
    }

    pub(crate) fn backlinks_for(&self, date: &Date) -> Option<&[DateBacklink]> {
        self.by_date.get(date).map(Vec::as_slice)
    }

    pub(crate) fn occurrence(&self, id: &str) -> Option<&DateOccurrence> {
        self.occurrences.get(id)
    }
}

#[allow(dead_code)]
impl DateOccurrence {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub(crate) fn note_ref(&self) -> &NoteRef {
        &self.note_ref
    }

    pub(crate) fn note_title(&self) -> &str {
        &self.note_title
    }

    pub(crate) fn origin(&self) -> &DateOrigin {
        &self.origin
    }

    pub(crate) fn marker(&self) -> &DateMarker {
        &self.marker
    }

    pub(crate) fn context(&self) -> &str {
        &self.context
    }
}

#[allow(dead_code)]
impl DateMarker {
    pub(crate) fn kind(&self) -> DateStampKind {
        match self {
            Self::Single { kind, .. } | Self::Range { kind, .. } => *kind,
        }
    }

    pub(crate) fn raw(&self) -> &str {
        match self {
            Self::Single { raw, .. } | Self::Range { raw, .. } => raw,
        }
    }
}

#[allow(dead_code)]
impl DateBacklink {
    pub(crate) fn occurrence_id(&self) -> &str {
        &self.occurrence_id
    }

    pub(crate) fn relation(&self) -> DateRelation {
        self.relation
    }
}

#[allow(dead_code)]
impl DateRelation {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Range => "range",
            Self::RangeStart => "range start",
            Self::RangeMiddle => "range",
            Self::RangeEnd => "range end",
        }
    }

    fn is_indexed(self) -> bool {
        !matches!(self, Self::RangeMiddle)
    }

    fn priority(self) -> u8 {
        match self {
            Self::RangeMiddle => 1,
            Self::Single | Self::Range | Self::RangeStart | Self::RangeEnd => 0,
        }
    }
}

impl SearchEntry {
    pub(crate) fn title(&self) -> &str {
        &self.title
    }

    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    pub(crate) fn source_path(&self) -> &str {
        &self.source_path
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct MakiConfig {
    project_title: Option<String>,
    source_dir: PathBuf,
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

    pub(crate) fn project_source_root(&self, project_root: &Path) -> PathBuf {
        project_root.join(&self.source_dir)
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
            source_dir: project
                .source
                .map(|source| parse_project_source(&project_file, &source))
                .transpose()?
                .unwrap_or_else(|| PathBuf::from(".")),
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
            source_dir: PathBuf::from("."),
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
    source: Option<String>,
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
                "source" => {
                    project.source = Some(parse_toml_string_value(
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

fn parse_project_source(project_file: &Path, source: &str) -> Result<PathBuf, Error> {
    let path = Path::new(source);
    if source.is_empty() || path.components().any(is_outside_project_component) {
        return Err(Error::InvalidProjectFile(
            project_file.to_path_buf(),
            "project.source must be a relative path inside the project".to_string(),
        ));
    }

    Ok(path.to_path_buf())
}

fn is_outside_project_component(component: Component<'_>) -> bool {
    matches!(
        component,
        Component::ParentDir | Component::RootDir | Component::Prefix(_)
    )
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

    fn search_entry(&self) -> SearchEntry {
        SearchEntry {
            title: self.title(),
            path: self.note_ref().web_path(),
            source_path: self.source_path().display().to_string(),
        }
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

fn has_uri_scheme(target: &str) -> bool {
    let Some((scheme, _rest)) = target.split_once(':') else {
        return false;
    };

    !scheme.is_empty()
        && scheme
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
}

pub(crate) fn is_external_href(target: &str) -> bool {
    let target = target.trim();

    target.starts_with("//") || has_uri_scheme(target)
}

fn is_checkable_external_href(target: &str) -> bool {
    let target = target.trim();

    target.starts_with("https://") || target.starts_with("http://")
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExternalLinkCheck {
    Ok,
    Broken { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExternalLinkCheckError {
    Status(u16),
    Transport(String),
}

impl std::fmt::Display for ExternalLinkCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(status) => write!(f, "HTTP {status}"),
            Self::Transport(message) => write!(f, "{message}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalLinkCheckMethod {
    Head,
    Get,
}

fn check_external_link(target: &str) -> ExternalLinkCheck {
    if !is_checkable_external_href(target) {
        return ExternalLinkCheck::Ok;
    }

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(3))
        .redirects(5)
        .build();

    let result = match request_external_link(&agent, ExternalLinkCheckMethod::Head, target) {
        Ok(()) => return ExternalLinkCheck::Ok,
        Err(ExternalLinkCheckError::Status(_)) => {
            request_external_link(&agent, ExternalLinkCheckMethod::Get, target)
        }
        Err(error) => Err(error),
    };

    match result {
        Ok(()) => ExternalLinkCheck::Ok,
        Err(error) => ExternalLinkCheck::Broken {
            reason: error.to_string(),
        },
    }
}

fn request_external_link(
    agent: &ureq::Agent,
    method: ExternalLinkCheckMethod,
    target: &str,
) -> Result<(), ExternalLinkCheckError> {
    let response = match method {
        ExternalLinkCheckMethod::Head => agent.head(target).call(),
        ExternalLinkCheckMethod::Get => agent.get(target).call(),
    };

    match response {
        Ok(response) if response.status() < 400 => Ok(()),
        Ok(response) => Err(ExternalLinkCheckError::Status(response.status())),
        Err(ureq::Error::Status(status, _response)) => Err(ExternalLinkCheckError::Status(status)),
        Err(ureq::Error::Transport(error)) => {
            Err(ExternalLinkCheckError::Transport(error.to_string()))
        }
    }
}

fn normalize_note_link_target(target: &str) -> String {
    let target = target.strip_prefix('/').unwrap_or(target);
    target
        .strip_suffix(MAKI_SOURCE_EXTENSION)
        .unwrap_or(target)
        .to_string()
}

pub(crate) fn note_link_target_for_href(target: &str) -> Option<String> {
    let target = target.trim();

    if target.is_empty()
        || target.contains('#')
        || target.contains('?')
        || target.starts_with("//")
        || has_uri_scheme(target)
    {
        return None;
    }

    let path_part = target.strip_prefix('/').unwrap_or(target);
    if path_part == "@" || path_part.starts_with("@/") || path_part.starts_with(".maki/") {
        return None;
    }

    let extension = Path::new(path_part)
        .extension()
        .and_then(|ext| ext.to_str());

    match extension {
        Some("maki") | None => Some(normalize_note_link_target(path_part)),
        Some(_) => None,
    }
}

pub(crate) fn date_page_path(date: Date) -> String {
    format!("/@/dates/{date}")
}

pub(crate) fn date_year_page_path(year: u16) -> String {
    format!("/@/dates/{year:04}")
}

pub(crate) fn date_month_page_path(year: u16, month: u8) -> String {
    format!("/@/dates/{year:04}-{month:02}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DatePeriod {
    Year(u16),
    Month { year: u16, month: u8 },
    Day(Date),
}

impl DatePeriod {
    const MIN_YEAR: u16 = 1;
    const MAX_YEAR: u16 = 9999;

    pub(crate) fn year(year: u16) -> Option<Self> {
        Self::valid_year(year).then_some(Self::Year(year))
    }

    pub(crate) fn month(year: u16, month: u8) -> Option<Self> {
        (Self::valid_year(year) && (1..=12).contains(&month)).then_some(Self::Month { year, month })
    }

    pub(crate) fn day(date: Date) -> Option<Self> {
        Self::valid_year(date.year()).then_some(Self::Day(date))
    }

    pub(crate) fn parse_path_segment(raw: &str) -> Option<Self> {
        match raw.len() {
            4 => Self::parse_year(raw).and_then(Self::year),
            7 if raw.as_bytes().get(4) == Some(&b'-') => {
                let year = Self::parse_year(&raw[..4])?;
                let month = Self::parse_two_digits(&raw[5..7])?;
                Self::month(year, month)
            }
            10 if raw.as_bytes().get(4) == Some(&b'-') && raw.as_bytes().get(7) == Some(&b'-') => {
                Date::parse(raw).and_then(Self::day)
            }
            _ => None,
        }
    }

    pub(crate) fn title(self) -> String {
        match self {
            Self::Year(year) => format!("{year:04}"),
            Self::Month { year, month } => format!("{year:04}-{month:02}"),
            Self::Day(date) => date.to_string(),
        }
    }

    pub(crate) fn path(self) -> String {
        match self {
            Self::Year(year) => date_year_page_path(year),
            Self::Month { year, month } => date_month_page_path(year, month),
            Self::Day(date) => date_page_path(date),
        }
    }

    pub(crate) fn parent_path(self) -> String {
        match self {
            Self::Year(_) => "/@/dates".to_string(),
            Self::Month { year, .. } => date_year_page_path(year),
            Self::Day(date) => date_month_page_path(date.year(), date.month()),
        }
    }

    pub(crate) fn previous(self) -> Option<Self> {
        match self {
            Self::Year(year) => year
                .checked_sub(1)
                .filter(|year| Self::valid_year(*year))
                .map(Self::Year),
            Self::Month { year, month } if month > 1 => Self::month(year, month - 1),
            Self::Month { year, .. } => year.checked_sub(1).and_then(|year| Self::month(year, 12)),
            Self::Day(date) => date.previous_day().and_then(Self::day),
        }
    }

    pub(crate) fn next(self) -> Option<Self> {
        match self {
            Self::Year(year) => year
                .checked_add(1)
                .filter(|year| Self::valid_year(*year))
                .map(Self::Year),
            Self::Month { year, month } if month < 12 => Self::month(year, month + 1),
            Self::Month { year, .. } => year.checked_add(1).and_then(|year| Self::month(year, 1)),
            Self::Day(date) => date.next_day().and_then(Self::day),
        }
    }

    fn valid_year(year: u16) -> bool {
        (Self::MIN_YEAR..=Self::MAX_YEAR).contains(&year)
    }

    fn parse_year(raw: &str) -> Option<u16> {
        if raw.len() != 4 || !raw.as_bytes().iter().all(u8::is_ascii_digit) {
            return None;
        }

        raw.parse::<u16>().ok().filter(|year| *year > 0)
    }

    fn parse_two_digits(raw: &str) -> Option<u8> {
        if raw.len() != 2 || !raw.as_bytes().iter().all(u8::is_ascii_digit) {
            return None;
        }

        raw.parse::<u8>().ok()
    }
}

pub(crate) fn inline_date_occurrence_id(source_path: &Path, ordinal: usize) -> String {
    date_occurrence_id("inline", source_path, ordinal)
}

pub(crate) fn property_date_occurrence_id(source_path: &Path, ordinal: usize) -> String {
    date_occurrence_id("property", source_path, ordinal)
}

pub(crate) fn date_occurrence_href(date: Date, occurrence_id: &str) -> String {
    format!("{}#{occurrence_id}", date_page_path(date))
}

fn date_occurrence_id(kind: &str, source_path: &Path, ordinal: usize) -> String {
    format!(
        "date-{kind}-{}-{ordinal}",
        stable_ascii_path_slug(source_path)
    )
}

fn stable_ascii_path_slug(path: &Path) -> String {
    let mut slug = String::new();
    for byte in path.to_string_lossy().as_bytes() {
        match byte {
            b'a'..=b'z' | b'0'..=b'9' => slug.push(*byte as char),
            b'A'..=b'Z' => slug.push(byte.to_ascii_lowercase() as char),
            b'/' | b'.' | b'-' | b'_' => slug.push('-'),
            _ => slug.push_str(&format!("x{byte:02x}")),
        }
    }

    slug.trim_matches('-').to_string()
}

fn search_match_rank(title: &str, query: &str) -> Option<(usize, usize, usize)> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectDiagnostic {
    source_path: PathBuf,
    line: Option<usize>,
    kind: ProjectDiagnosticKind,
}

impl ProjectDiagnostic {
    fn new(
        source_path: impl Into<PathBuf>,
        line: Option<usize>,
        kind: ProjectDiagnosticKind,
    ) -> Self {
        Self {
            source_path: source_path.into(),
            line,
            kind,
        }
    }

    pub(crate) fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub(crate) fn line(&self) -> Option<usize> {
        self.line
    }

    pub(crate) fn kind(&self) -> &ProjectDiagnosticKind {
        &self.kind
    }

    pub(crate) fn message(&self) -> String {
        self.kind.message()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjectDiagnosticKind {
    ParseWarning { message: String },
    BrokenLink { target: String },
    AmbiguousLink { target: String },
    BrokenExternalLink { target: String, reason: String },
    ReadFailed,
}

impl ProjectDiagnosticKind {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::ParseWarning { .. } => "parser",
            Self::BrokenLink { .. } => "broken link",
            Self::AmbiguousLink { .. } => "ambiguous link",
            Self::BrokenExternalLink { .. } => "external link",
            Self::ReadFailed => "read",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::ParseWarning { message } => message.clone(),
            Self::BrokenLink { target } => format!("broken link: {target}"),
            Self::AmbiguousLink { target } => format!("ambiguous link: {target}"),
            Self::BrokenExternalLink { target, reason } => {
                format!("broken external link: {target} ({reason})")
            }
            Self::ReadFailed => "failed to read note".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ProjectDiagnosticSummary {
    total: usize,
    parse_warnings: usize,
    broken_links: usize,
    ambiguous_links: usize,
    broken_external_links: usize,
    read_failures: usize,
}

impl ProjectDiagnosticSummary {
    pub(crate) fn from_diagnostics(diagnostics: &[ProjectDiagnostic]) -> Self {
        let mut summary = Self {
            total: diagnostics.len(),
            ..Default::default()
        };

        for diagnostic in diagnostics {
            match diagnostic.kind() {
                ProjectDiagnosticKind::ParseWarning { .. } => summary.parse_warnings += 1,
                ProjectDiagnosticKind::BrokenLink { .. } => summary.broken_links += 1,
                ProjectDiagnosticKind::AmbiguousLink { .. } => summary.ambiguous_links += 1,
                ProjectDiagnosticKind::BrokenExternalLink { .. } => {
                    summary.broken_external_links += 1
                }
                ProjectDiagnosticKind::ReadFailed => summary.read_failures += 1,
            }
        }

        summary
    }

    pub(crate) fn total(&self) -> usize {
        self.total
    }

    pub(crate) fn parse_warnings(&self) -> usize {
        self.parse_warnings
    }

    pub(crate) fn broken_links(&self) -> usize {
        self.broken_links
    }

    pub(crate) fn ambiguous_links(&self) -> usize {
        self.ambiguous_links
    }

    pub(crate) fn broken_external_links(&self) -> usize {
        self.broken_external_links
    }

    pub(crate) fn read_failures(&self) -> usize {
        self.read_failures
    }
}

fn collect_inline_external_links(
    external_links: &mut BTreeSet<ExternalLinkRef>,
    source_path: &Path,
    inlines: &[Inline<'_>],
) {
    for inline in inlines {
        match inline {
            Inline::Link { target, .. } if is_checkable_external_href(target) => {
                external_links.insert(ExternalLinkRef {
                    source_path: source_path.to_path_buf(),
                    target: target.trim().to_string(),
                });
            }
            Inline::NoteLink { .. }
            | Inline::Link { .. }
            | Inline::DateStamp(_)
            | Inline::DateRange(_)
            | Inline::Text(_)
            | Inline::SoftBreak
            | Inline::Code(_) => {}
        }
    }
}

fn collect_table_row_external_links(
    external_links: &mut BTreeSet<ExternalLinkRef>,
    source_path: &Path,
    row: &parser::TableRow<'_>,
) {
    if row.is_separator() {
        return;
    }

    for cell in &row.cells {
        collect_inline_external_links(external_links, source_path, &cell.body);
    }
}

fn collect_block_external_links(
    external_links: &mut BTreeSet<ExternalLinkRef>,
    source_path: &Path,
    block: &BlockKind<'_>,
) {
    match block {
        BlockKind::Paragraph { body } => {
            collect_inline_external_links(external_links, source_path, body)
        }
        BlockKind::Heading { body, .. } => {
            let inlines = parser::parse_inline(body);
            collect_inline_external_links(external_links, source_path, &inlines);
        }
        BlockKind::List { items } => {
            for item in items {
                collect_inline_external_links(external_links, source_path, &item.body);
                for child in &item.children {
                    collect_block_external_links(external_links, source_path, &child.kind);
                }
            }
        }
        BlockKind::Quote { lines } => {
            collect_maki_lines_external_links(external_links, source_path, lines)
        }
        BlockKind::Table { header, rows, .. } => {
            collect_table_row_external_links(external_links, source_path, header);
            for row in rows {
                collect_table_row_external_links(external_links, source_path, row);
            }
        }
        BlockKind::Container { kind, lines, .. } if *kind == "quote" => {
            collect_maki_lines_external_links(external_links, source_path, lines)
        }
        BlockKind::Code { .. } | BlockKind::Container { .. } => {}
    }
}

fn collect_maki_lines_external_links(
    external_links: &mut BTreeSet<ExternalLinkRef>,
    source_path: &Path,
    lines: &[&str],
) {
    let source = lines.join("\n");
    let parsed = parser::parse(&source);

    for block in &parsed.document.blocks {
        collect_block_external_links(external_links, source_path, &block.kind);
    }
}

fn collect_external_links(notes: &BTreeMap<NoteRef, Note>) -> Vec<ExternalLinkRef> {
    let mut external_links = BTreeSet::new();

    for note in notes.values() {
        let Ok(source) = std::fs::read_to_string(&note.absolute_path) else {
            continue;
        };
        let parsed = parser::parse(&source);

        for block in &parsed.document.blocks {
            collect_block_external_links(&mut external_links, note.source_path(), &block.kind);
        }
    }

    external_links.into_iter().collect()
}

struct DateIndexCollector<'a> {
    index: &'a mut DateIndex,
    source_path: &'a Path,
    note_ref: NoteRef,
    note_title: String,
    inline_ordinal: usize,
    property_ordinal: usize,
}

impl<'a> DateIndexCollector<'a> {
    fn new(
        index: &'a mut DateIndex,
        source_path: &'a Path,
        note_ref: NoteRef,
        note_title: String,
    ) -> Self {
        Self {
            index,
            source_path,
            note_ref,
            note_title,
            inline_ordinal: 0,
            property_ordinal: 0,
        }
    }

    fn push_occurrence(
        &mut self,
        id: String,
        origin: DateOrigin,
        marker: DateMarker,
        context: &str,
    ) {
        self.index.insert_occurrence(DateOccurrence {
            id,
            source_path: self.source_path.to_path_buf(),
            note_ref: self.note_ref.clone(),
            note_title: self.note_title.clone(),
            origin,
            marker,
            context: context.to_string(),
        });
    }

    fn push_inline_stamp(&mut self, stamp: DateStamp<'_>, context: &str) {
        self.inline_ordinal += 1;
        self.push_occurrence(
            inline_date_occurrence_id(self.source_path, self.inline_ordinal),
            DateOrigin::Inline,
            date_stamp_marker(stamp),
            context,
        );
    }

    fn push_inline_range(&mut self, range: DateRange<'_>, context: &str) {
        self.inline_ordinal += 1;
        self.push_occurrence(
            inline_date_occurrence_id(self.source_path, self.inline_ordinal),
            DateOrigin::Inline,
            date_range_marker(range),
            context,
        );
    }

    fn push_property_stamp(&mut self, key: &str, stamp: DateStamp<'_>, context: &str) {
        self.property_ordinal += 1;
        self.push_occurrence(
            property_date_occurrence_id(self.source_path, self.property_ordinal),
            DateOrigin::Property {
                key: key.to_string(),
            },
            date_stamp_marker(stamp),
            context,
        );
    }

    fn push_property_range(&mut self, key: &str, range: DateRange<'_>, context: &str) {
        self.property_ordinal += 1;
        self.push_occurrence(
            property_date_occurrence_id(self.source_path, self.property_ordinal),
            DateOrigin::Property {
                key: key.to_string(),
            },
            date_range_marker(range),
            context,
        );
    }
}

fn date_stamp_marker(stamp: DateStamp<'_>) -> DateMarker {
    DateMarker::Single {
        kind: stamp.kind(),
        date: stamp.date(),
        raw: date_stamp_raw(stamp),
    }
}

fn date_range_marker(range: DateRange<'_>) -> DateMarker {
    DateMarker::Range {
        kind: range.kind(),
        start: range.start().date(),
        end: range.end().date(),
        raw: date_range_raw(range),
    }
}

fn date_stamp_raw(stamp: DateStamp<'_>) -> String {
    let (open, close) = match stamp.kind() {
        DateStampKind::Date => ('[', ']'),
        DateStampKind::Event => ('<', '>'),
    };

    format!("{open}{}{close}", stamp.body())
}

fn date_range_raw(range: DateRange<'_>) -> String {
    format!(
        "{}--{}",
        date_stamp_raw(range.start()),
        date_stamp_raw(range.end())
    )
}

const DATE_CONTEXT_MAX_CHARS: usize = 500;

#[derive(Debug, Clone)]
struct DateHeadingContext {
    level: usize,
    context: String,
}

#[derive(Debug, Clone, Default)]
struct DateTraversalContext {
    headings: Vec<DateHeadingContext>,
    top_list_item: Option<String>,
}

impl DateTraversalContext {
    fn current_heading_context(&self) -> Option<&str> {
        self.headings.last().map(|heading| heading.context.as_str())
    }

    fn parent_heading_context(&self, level: usize) -> Option<&str> {
        self.headings
            .iter()
            .rev()
            .find(|heading| heading.level < level)
            .map(|heading| heading.context.as_str())
    }

    fn enter_heading(&mut self, level: usize, body: &str) {
        self.headings.retain(|heading| heading.level < level);
        self.headings.push(DateHeadingContext {
            level,
            context: heading_date_context(level, body),
        });
    }

    fn with_top_list_item(&self, top_list_item: String) -> Self {
        let mut context = self.clone();
        if context.top_list_item.is_none() {
            context.top_list_item = Some(top_list_item);
        }
        context
    }

    fn contextualize(&self, local_context: &str) -> String {
        date_context_with_scope(
            self.current_heading_context(),
            self.top_list_item.as_deref(),
            local_context,
        )
    }

    fn contextualize_heading(&self, level: usize, local_context: &str) -> String {
        date_context_with_scope(
            self.parent_heading_context(level),
            self.top_list_item.as_deref(),
            local_context,
        )
    }
}

fn truncate_date_context(mut input: String) -> String {
    if let Some((byte_index, _)) = input.char_indices().nth(DATE_CONTEXT_MAX_CHARS) {
        input.truncate(byte_index);
        input.push_str("...");
    }

    input
}

fn push_date_context_part(context: &mut String, part: &str, indent: usize) {
    let part = part.trim_end();
    if part.trim().is_empty() {
        return;
    }

    if !context.is_empty() {
        context.push('\n');
    }

    let prefix = " ".repeat(indent);
    for (index, line) in part.lines().enumerate() {
        if index > 0 {
            context.push('\n');
        }
        if indent > 0 && !line.is_empty() {
            context.push_str(&prefix);
        }
        context.push_str(line);
    }
}

fn date_context_with_scope(
    heading_context: Option<&str>,
    top_list_item: Option<&str>,
    local_context: &str,
) -> String {
    let mut context = String::new();
    if let Some(heading_context) = heading_context {
        push_date_context_part(&mut context, heading_context, 0);
    }
    if let Some(top_list_item) = top_list_item {
        push_date_context_part(&mut context, top_list_item, 0);
    }

    let local_context = local_context.trim_end();
    let duplicates_top_list_item =
        top_list_item.is_some_and(|top_list_item| top_list_item.trim_end() == local_context);
    if !duplicates_top_list_item {
        let indent = if top_list_item.is_some() { 2 } else { 0 };
        push_date_context_part(&mut context, local_context, indent);
    }

    truncate_date_context(context)
}

fn inline_date_context(inlines: &[Inline<'_>]) -> String {
    let mut context = String::new();

    for inline in inlines {
        match inline {
            Inline::NoteLink { target } => {
                context.push_str("[[");
                context.push_str(target);
                context.push_str("]]");
            }
            Inline::Link { title, target } => {
                context.push('[');
                context.push_str(title);
                context.push_str("](");
                context.push_str(target);
                context.push(')');
            }
            Inline::DateStamp(stamp) => context.push_str(&date_stamp_raw(*stamp)),
            Inline::DateRange(range) => context.push_str(&date_range_raw(*range)),
            Inline::Text(text) => context.push_str(text),
            Inline::SoftBreak => context.push(' '),
            Inline::Code(text) => {
                context.push('`');
                context.push_str(text);
                context.push('`');
            }
        }
    }

    truncate_date_context(context)
}

fn heading_date_context(level: usize, body: &str) -> String {
    format!("{} {body}", "=".repeat(level))
}

fn list_item_marker_prefix(kind: parser::ListKind) -> &'static str {
    match kind {
        parser::ListKind::Unordered => "- ",
        parser::ListKind::Ordered => "1. ",
    }
}

fn list_item_line_date_context(item: &parser::ListItem<'_>) -> String {
    let mut context = String::new();
    context.push_str(list_item_marker_prefix(item.kind));
    context.push_str(&inline_date_context(&item.body));

    truncate_date_context(context)
}

fn list_item_date_context(item: &parser::ListItem<'_>) -> String {
    let mut context = String::new();
    context.push_str(list_item_marker_prefix(item.kind));
    context.push_str(&inline_date_context(&item.body));

    for child in &item.children {
        let child_context = block_date_context(child);
        if child_context.trim().is_empty() {
            continue;
        }
        for line in child_context.lines() {
            context.push('\n');
            context.push_str("  ");
            context.push_str(line);
        }
    }

    truncate_date_context(context)
}

fn table_row_date_context(row: &parser::TableRow<'_>) -> String {
    if row.is_separator() {
        return String::from("| --- |");
    }

    let mut context = String::from("| ");
    context.push_str(
        &row.cells
            .iter()
            .map(|cell| inline_date_context(&cell.body))
            .collect::<Vec<_>>()
            .join(" | "),
    );
    context.push_str(" |");

    truncate_date_context(context)
}

fn table_date_context(header: &parser::TableRow<'_>, rows: &[parser::TableRow<'_>]) -> String {
    let mut context = table_row_date_context(header);

    for row in rows {
        context.push('\n');
        context.push_str(&table_row_date_context(row));
    }

    truncate_date_context(context)
}

fn block_date_context(block: &parser::Block<'_>) -> String {
    let context = match &block.kind {
        BlockKind::Paragraph { body } => inline_date_context(body),
        BlockKind::Code { lines, .. } => lines.join("\n"),
        BlockKind::Heading { level, body } => heading_date_context(*level, body),
        BlockKind::List { items } => items
            .iter()
            .map(list_item_date_context)
            .collect::<Vec<_>>()
            .join("\n"),
        BlockKind::Quote { lines } => lines.join("\n"),
        BlockKind::Table { header, rows, .. } => table_date_context(header, rows),
        BlockKind::Container { kind, args, lines } => {
            let mut context = String::from("--- ");
            context.push_str(kind);
            if !args.is_empty() {
                context.push(' ');
                context.push_str(&args.join(" "));
            }
            if !lines.is_empty() {
                context.push('\n');
                context.push_str(&lines.join("\n"));
            }
            context
        }
    };

    truncate_date_context(context)
}

fn property_date_context(key: &str, value: &str, owner_context: &str) -> String {
    let mut context = format!("{key}: {value}");
    if !owner_context.trim().is_empty() {
        context.push('\n');
        context.push_str(owner_context);
    }

    truncate_date_context(context)
}

fn document_date_context(document: &parser::Document<'_>, fallback_title: &str) -> String {
    truncate_date_context(document.title().unwrap_or(fallback_title).to_string())
}

fn collect_inline_dates(
    collector: &mut DateIndexCollector<'_>,
    inlines: &[Inline<'_>],
    context: &str,
) {
    for inline in inlines {
        match inline {
            Inline::DateStamp(stamp) => collector.push_inline_stamp(*stamp, context),
            Inline::DateRange(range) => collector.push_inline_range(*range, context),
            Inline::NoteLink { .. }
            | Inline::Link { .. }
            | Inline::Text(_)
            | Inline::SoftBreak
            | Inline::Code(_) => {}
        }
    }
}

fn collect_property_dates<'a>(
    collector: &mut DateIndexCollector<'_>,
    properties: impl Iterator<Item = (&'a str, &'a str)>,
    owner_context: &str,
) {
    for (key, value) in properties {
        let context = property_date_context(key, value, owner_context);
        let inlines = parser::parse_inline(value);
        for inline in &inlines {
            match inline {
                Inline::DateStamp(stamp) => collector.push_property_stamp(key, *stamp, &context),
                Inline::DateRange(range) => collector.push_property_range(key, *range, &context),
                Inline::NoteLink { .. }
                | Inline::Link { .. }
                | Inline::Text(_)
                | Inline::SoftBreak
                | Inline::Code(_) => {}
            }
        }
    }
}

fn collect_list_item_dates(
    collector: &mut DateIndexCollector<'_>,
    item: &parser::ListItem<'_>,
    context: &DateTraversalContext,
) {
    let item_line_context = list_item_line_date_context(item);
    let mut item_context = context.with_top_list_item(item_line_context.clone());
    let occurrence_context = item_context.contextualize(&item_line_context);

    collect_inline_dates(collector, &item.body, &occurrence_context);
    for child in &item.children {
        collect_block_dates(collector, child, &mut item_context);
    }
}

fn collect_table_row_dates(
    collector: &mut DateIndexCollector<'_>,
    row: &parser::TableRow<'_>,
    context: &str,
) {
    if row.is_separator() {
        return;
    }

    for cell in &row.cells {
        collect_inline_dates(collector, &cell.body, context);
    }
}

fn collect_block_dates(
    collector: &mut DateIndexCollector<'_>,
    block: &parser::Block<'_>,
    context: &mut DateTraversalContext,
) {
    let local_context = block_date_context(block);
    let block_context = match &block.kind {
        BlockKind::Heading { level, .. } => context.contextualize_heading(*level, &local_context),
        _ => context.contextualize(&local_context),
    };
    collect_property_dates(collector, block.properties(), &block_context);

    match &block.kind {
        BlockKind::Paragraph { body } => collect_inline_dates(collector, body, &block_context),
        BlockKind::Heading { level, body } => {
            let inlines = parser::parse_inline(body);
            collect_inline_dates(collector, &inlines, &block_context);
            context.enter_heading(*level, body);
        }
        BlockKind::List { items } => {
            for item in items {
                collect_list_item_dates(collector, item, context);
            }
        }
        BlockKind::Quote { lines } => collect_maki_lines_dates(collector, lines, context),
        BlockKind::Table { header, rows, .. } => {
            collect_table_row_dates(collector, header, &block_context);
            for row in rows {
                collect_table_row_dates(collector, row, &block_context);
            }
        }
        BlockKind::Container { kind, lines, .. } if *kind == "quote" => {
            collect_maki_lines_dates(collector, lines, context)
        }
        BlockKind::Code { .. } | BlockKind::Container { .. } => {}
    }
}

fn collect_maki_lines_dates(
    collector: &mut DateIndexCollector<'_>,
    lines: &[&str],
    context: &DateTraversalContext,
) {
    let source = lines.join("\n");
    let parsed = parser::parse(&source);
    let mut nested_context = context.clone();
    collect_document_dates_with_context(collector, &parsed.document, &mut nested_context);
}

fn collect_document_dates_with_context(
    collector: &mut DateIndexCollector<'_>,
    document: &parser::Document<'_>,
    context: &mut DateTraversalContext,
) {
    let document_context =
        context.contextualize(&document_date_context(document, &collector.note_title));

    collect_property_dates(collector, document.properties(), &document_context);
    for block in &document.blocks {
        collect_block_dates(collector, block, context);
    }
}

fn collect_document_dates(collector: &mut DateIndexCollector<'_>, document: &parser::Document<'_>) {
    let mut context = DateTraversalContext::default();
    collect_document_dates_with_context(collector, document, &mut context);
}

fn collect_date_index(notes: &BTreeMap<NoteRef, Note>) -> DateIndex {
    let mut date_index = DateIndex::default();

    for note in notes.values() {
        let Ok(source) = std::fs::read_to_string(&note.absolute_path) else {
            continue;
        };
        let parsed = parser::parse(&source);
        let note_ref = note.note_ref();
        let note_title = parsed
            .document
            .title()
            .unwrap_or(note.file_stem())
            .to_string();
        let mut collector =
            DateIndexCollector::new(&mut date_index, note.source_path(), note_ref, note_title);
        collect_document_dates(&mut collector, &parsed.document);
    }

    date_index.sort_backlinks();
    date_index
}

fn collect_inline_link_diagnostics(
    diagnostics: &mut Vec<ProjectDiagnostic>,
    maki: &Maki,
    current: &NoteRef,
    source_path: &Path,
    inlines: &[Inline<'_>],
) {
    for inline in inlines {
        match inline {
            Inline::NoteLink { target } => push_link_diagnostic(
                diagnostics,
                source_path,
                maki.resolve_note_link(current, target),
                target,
            ),
            Inline::Link { target, .. } => {
                if let Some(note_target) = note_link_target_for_href(target) {
                    push_link_diagnostic(
                        diagnostics,
                        source_path,
                        maki.resolve_note_link(current, &note_target),
                        &note_target,
                    );
                }
            }
            Inline::DateStamp(_)
            | Inline::DateRange(_)
            | Inline::Text(_)
            | Inline::SoftBreak
            | Inline::Code(_) => {}
        }
    }
}

fn collect_table_row_link_diagnostics(
    diagnostics: &mut Vec<ProjectDiagnostic>,
    maki: &Maki,
    current: &NoteRef,
    source_path: &Path,
    row: &parser::TableRow<'_>,
) {
    if row.is_separator() {
        return;
    }

    for cell in &row.cells {
        collect_inline_link_diagnostics(diagnostics, maki, current, source_path, &cell.body);
    }
}

fn collect_block_link_diagnostics(
    diagnostics: &mut Vec<ProjectDiagnostic>,
    maki: &Maki,
    current: &NoteRef,
    source_path: &Path,
    block: &BlockKind<'_>,
) {
    match block {
        BlockKind::Paragraph { body } => {
            collect_inline_link_diagnostics(diagnostics, maki, current, source_path, body)
        }
        BlockKind::Heading { body, .. } => {
            let inlines = parser::parse_inline(body);
            collect_inline_link_diagnostics(diagnostics, maki, current, source_path, &inlines);
        }
        BlockKind::List { items } => {
            for item in items {
                collect_inline_link_diagnostics(
                    diagnostics,
                    maki,
                    current,
                    source_path,
                    &item.body,
                );
                for child in &item.children {
                    collect_block_link_diagnostics(
                        diagnostics,
                        maki,
                        current,
                        source_path,
                        &child.kind,
                    );
                }
            }
        }
        BlockKind::Quote { lines } => {
            collect_maki_lines_link_diagnostics(diagnostics, maki, current, source_path, lines)
        }
        BlockKind::Table { header, rows, .. } => {
            collect_table_row_link_diagnostics(diagnostics, maki, current, source_path, header);
            for row in rows {
                collect_table_row_link_diagnostics(diagnostics, maki, current, source_path, row);
            }
        }
        BlockKind::Container { kind, lines, .. } if *kind == "quote" => {
            collect_maki_lines_link_diagnostics(diagnostics, maki, current, source_path, lines)
        }
        BlockKind::Code { .. } | BlockKind::Container { .. } => {}
    }
}

fn collect_maki_lines_link_diagnostics(
    diagnostics: &mut Vec<ProjectDiagnostic>,
    maki: &Maki,
    current: &NoteRef,
    source_path: &Path,
    lines: &[&str],
) {
    let source = lines.join("\n");
    let parsed = parser::parse(&source);

    for block in &parsed.document.blocks {
        collect_block_link_diagnostics(diagnostics, maki, current, source_path, &block.kind);
    }
}

fn push_link_diagnostic(
    diagnostics: &mut Vec<ProjectDiagnostic>,
    source_path: &Path,
    resolution: NoteLinkResolution,
    target: &str,
) {
    match resolution {
        NoteLinkResolution::Found(_) => {}
        NoteLinkResolution::Broken => diagnostics.push(ProjectDiagnostic::new(
            source_path,
            None,
            ProjectDiagnosticKind::BrokenLink {
                target: target.to_string(),
            },
        )),
        NoteLinkResolution::Ambiguous => diagnostics.push(ProjectDiagnostic::new(
            source_path,
            None,
            ProjectDiagnosticKind::AmbiguousLink {
                target: target.to_string(),
            },
        )),
    }
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
        let target = normalize_note_link_target(target);
        let target = target.as_str();

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

    pub(crate) fn diagnostics(&self) -> Vec<ProjectDiagnostic> {
        self.diagnostics_with_external_link_checker(&check_external_link)
    }

    pub(crate) fn diagnostics_without_external_links(&self) -> Vec<ProjectDiagnostic> {
        self.collect_note_diagnostics()
    }

    fn diagnostics_with_external_link_checker(
        &self,
        check_external_link: &dyn Fn(&str) -> ExternalLinkCheck,
    ) -> Vec<ProjectDiagnostic> {
        let mut diagnostics = self.collect_note_diagnostics();
        self.push_external_link_diagnostics(&mut diagnostics, check_external_link);
        diagnostics
    }

    fn collect_note_diagnostics(&self) -> Vec<ProjectDiagnostic> {
        let mut diagnostics = vec![];

        for note in self.notes.values() {
            let source_path = note.source_path();
            let source = match std::fs::read_to_string(&note.absolute_path) {
                Ok(source) => source,
                Err(_) => {
                    diagnostics.push(ProjectDiagnostic::new(
                        source_path,
                        None,
                        ProjectDiagnosticKind::ReadFailed,
                    ));
                    continue;
                }
            };
            let parsed = parser::parse(&source);

            for diagnostic in &parsed.diagnostics {
                diagnostics.push(ProjectDiagnostic::new(
                    source_path,
                    Some(diagnostic.line),
                    ProjectDiagnosticKind::ParseWarning {
                        message: parser::format_parse_diagnostic_kind(&diagnostic.kind),
                    },
                ));
            }

            let current = note.note_ref();
            for block in &parsed.document.blocks {
                collect_block_link_diagnostics(
                    &mut diagnostics,
                    self,
                    &current,
                    source_path,
                    &block.kind,
                );
            }
        }

        diagnostics
    }

    fn push_external_link_diagnostics(
        &self,
        diagnostics: &mut Vec<ProjectDiagnostic>,
        check_external_link: &dyn Fn(&str) -> ExternalLinkCheck,
    ) {
        let mut checks = BTreeMap::new();

        for external_link in &self.external_links {
            let check = checks
                .entry(external_link.target.clone())
                .or_insert_with(|| check_external_link(&external_link.target))
                .clone();

            if let ExternalLinkCheck::Broken { reason } = check {
                diagnostics.push(ProjectDiagnostic::new(
                    external_link.source_path.clone(),
                    None,
                    ProjectDiagnosticKind::BrokenExternalLink {
                        target: external_link.target.clone(),
                        reason,
                    },
                ));
            }
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

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn notes(&self) -> impl Iterator<Item = &Note> {
        self.notes.values()
    }

    pub(crate) fn notes_len(&self) -> usize {
        self.notes.len()
    }

    pub(crate) fn search_entries(&self) -> &[SearchEntry] {
        &self.search_entries
    }

    #[allow(dead_code)]
    pub(crate) fn date_index(&self) -> &DateIndex {
        &self.date_index
    }

    pub(crate) fn search_titles(&self, query: &str, limit: usize) -> Vec<SearchEntry> {
        let query = normalize_key(query.trim());

        if query.is_empty() {
            return self.search_entries.iter().take(limit).cloned().collect();
        }

        let mut matches = self
            .search_entries
            .iter()
            .filter_map(|entry| search_match_rank(entry.title(), &query).map(|rank| (rank, entry)))
            .collect::<Vec<_>>();

        matches.sort_by(|(left_rank, left), (right_rank, right)| {
            left_rank
                .cmp(right_rank)
                .then_with(|| left.title().cmp(right.title()))
                .then_with(|| left.path().cmp(right.path()))
        });

        matches
            .into_iter()
            .take(limit)
            .map(|(_rank, entry)| entry.clone())
            .collect()
    }

    pub(crate) fn load_with_config(root: &Path, config: MakiConfig) -> Result<Self, Error> {
        Self::load_with_config_metered(root, config, &Metrics::disabled())
    }

    pub(crate) fn load_with_config_metered(
        root: &Path,
        config: MakiConfig,
        metrics: &Metrics,
    ) -> Result<Self, Error> {
        if !root.exists() {
            return Err(Error::RootNotFound(root.to_path_buf()));
        }
        if !root.is_dir() {
            return Err(Error::RootNotDirectory(root.to_path_buf()));
        }

        let root =
            std::fs::canonicalize(root).map_err(|_source| Error::RootNotFound(root.to_owned()))?;

        let started = Instant::now();
        let files = list_maki_files(&root)?;
        metrics.record_project_load_phase("list_files", started.elapsed());

        let started = Instant::now();
        let mut notes = BTreeMap::new();

        for file in &files {
            let note = Note::load(&root, file)?;
            notes.insert(note.note_ref(), note);
        }
        metrics.record_project_load_phase("load_notes", started.elapsed());

        let started = Instant::now();
        let index = NoteIndex::build(notes.keys());
        metrics.record_project_load_phase("index", started.elapsed());

        let started = Instant::now();
        let date_index = collect_date_index(&notes);
        let external_links = collect_external_links(&notes);
        let search_entries = notes.values().map(Note::search_entry).collect();
        metrics.record_project_load_phase("metadata", started.elapsed());

        Ok(Self {
            root,
            notes,
            index,
            date_index,
            external_links,
            search_entries,
            config,
        })
    }

    // root: absolute or relative to the project directory
    #[allow(dead_code)]
    pub(crate) fn load(root: impl AsRef<Path>) -> Result<Self, Error> {
        Self::load_with_config(root.as_ref(), MakiConfig::default())
    }

    pub(crate) fn render_html(&self, path: &Path) -> Result<String, Error> {
        self.render_html_with_asset_mode(path, AssetMode::Inline)
    }

    pub(crate) fn render_html_with_asset_mode(
        &self,
        path: &Path,
        asset_mode: AssetMode,
    ) -> Result<String, Error> {
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
            RenderContext::project(&resolve_note_link, &get_note_info)
                .with_asset_mode(asset_mode)
                .with_date_source_path(path),
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
        cell::RefCell,
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

    #[test]
    fn project_config_can_set_source_directory() {
        let project = temp_project("project-source");
        fs::write(
            project.root.join(PROJECT_FILE_NAME),
            "[project]\ntitle = \"Source Fixture\"\nsource = \"docs\"\nhome = \"index\"\n",
        )
        .unwrap();

        let config = MakiConfig::load_project(&project.root).unwrap();

        assert_eq!(
            config.project_source_root(&project.root),
            project.root.join("docs")
        );
        assert_eq!(
            config.home_mode(),
            &HomeMode::Redirect("/index".to_string())
        );
    }

    #[test]
    fn project_config_rejects_source_outside_project() {
        let project = temp_project("project-source-invalid");
        fs::write(
            project.root.join(PROJECT_FILE_NAME),
            "[project]\nsource = \"../docs\"\n",
        )
        .unwrap();

        assert!(matches!(
            MakiConfig::load_project(&project.root),
            Err(Error::InvalidProjectFile(_, message))
                if message == "project.source must be a relative path inside the project"
        ));
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

    #[test]
    fn markdown_style_links_can_resolve_to_notes_with_custom_titles() {
        let project = temp_project("markdown-style-note-link");
        write_note_with_content(&project, "start.maki", "See [the page](page).");
        write_note_with_content(&project, "page.maki", "--^ title: Page\n\nbody");

        let maki = Maki::load(&project.root).unwrap();
        let html = maki.render_html(Path::new("start.maki")).unwrap();

        assert!(html.contains("<a href=\"/page\">the page</a>"));
    }

    #[test]
    fn markdown_style_external_links_render_as_plain_hrefs() {
        let project = temp_project("markdown-style-external-link");
        write_note_with_content(
            &project,
            "start.maki",
            "See [djot](https://github.com/jgm/djot).",
        );

        let maki = Maki::load(&project.root).unwrap();
        let html = maki.render_html(Path::new("start.maki")).unwrap();

        assert!(
            html.contains(
                "<a class=\"external-link\" href=\"https://github.com/jgm/djot\">djot</a>"
            )
        );
    }

    #[test]
    fn plain_external_urls_render_as_links() {
        let project = temp_project("plain-external-link");
        write_note_with_content(&project, "start.maki", "See https://example.com/docs.");

        let maki = Maki::load(&project.root).unwrap();
        let html = maki.render_html(Path::new("start.maki")).unwrap();

        assert!(html.contains(
            "<a class=\"external-link\" href=\"https://example.com/docs\">https://example.com/docs</a>."
        ));
    }

    #[test]
    fn diagnostics_collect_parse_warnings_and_link_resolution_issues() {
        let project = temp_project("diagnostics");
        write_note_with_content(
            &project,
            "start.maki",
            r#"--^ invalid-property

See [[missing]], [Ghost](ghost), and [[same]].

> See [[quoted-missing]].

--- quote
See [[container-missing]].
---"#,
        );
        write_note(&project, "alpha/same.maki");
        write_note(&project, "beta/same.maki");

        let maki = Maki::load(&project.root).unwrap();
        let diagnostics = maki.diagnostics();

        assert!(diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.kind(),
                ProjectDiagnosticKind::ParseWarning { message }
                    if message == "invalid property: --^ invalid-property"
            ) && diagnostic.line() == Some(1)
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.kind(),
                ProjectDiagnosticKind::BrokenLink { target }
                    if target == "missing"
            )
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.kind(),
                ProjectDiagnosticKind::BrokenLink { target }
                    if target == "ghost"
            )
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.kind(),
                ProjectDiagnosticKind::BrokenLink { target }
                    if target == "quoted-missing"
            )
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.kind(),
                ProjectDiagnosticKind::BrokenLink { target }
                    if target == "container-missing"
            )
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.kind(),
                ProjectDiagnosticKind::AmbiguousLink { target }
                    if target == "same"
            )
        }));
    }

    #[test]
    fn diagnostics_without_external_links_skips_external_link_checks() {
        let project = temp_project("local-diagnostics");
        write_note_with_content(
            &project,
            "start.maki",
            "See [Down](https://down.example/path) and [[missing]].",
        );

        let maki = Maki::load(&project.root).unwrap();
        let diagnostics = maki.diagnostics_without_external_links();

        assert!(diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.kind(),
                ProjectDiagnosticKind::BrokenLink { target } if target == "missing"
            )
        }));
        assert!(!diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.kind(),
            ProjectDiagnosticKind::BrokenExternalLink { .. }
        )));
    }

    #[test]
    fn diagnostics_collect_broken_external_links() {
        let project = temp_project("external-link-diagnostics");
        write_note_with_content(
            &project,
            "start.maki",
            r#"See [Down](https://down.example/path), https://ok.example/docs, and `https://code.example`.

--- quote
See https://down.example/path.
---"#,
        );

        let maki = Maki::load(&project.root).unwrap();
        let checked = RefCell::new(vec![]);
        let diagnostics = maki.diagnostics_with_external_link_checker(&|target| {
            checked.borrow_mut().push(target.to_string());
            if target == "https://down.example/path" {
                ExternalLinkCheck::Broken {
                    reason: "HTTP 404".to_string(),
                }
            } else {
                ExternalLinkCheck::Ok
            }
        });

        assert_eq!(
            checked.into_inner(),
            vec![
                "https://down.example/path".to_string(),
                "https://ok.example/docs".to_string(),
            ]
        );
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.source_path() == Path::new("start.maki")
                && matches!(
                    diagnostic.kind(),
                    ProjectDiagnosticKind::BrokenExternalLink { target, reason }
                        if target == "https://down.example/path" && reason == "HTTP 404"
                )
        }));
        assert!(!diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.kind(),
                ProjectDiagnosticKind::BrokenExternalLink { target, .. }
                    if target == "https://ok.example/docs"
                        || target == "https://code.example"
            )
        }));
    }

    #[test]
    fn date_index_collects_inline_property_and_range_dates() {
        let project = temp_project("date-index");
        write_note_with_content(
            &project,
            "start.maki",
            r#"--^ title: Start
--^ date: [2026-08-15]

Meet <2026-08-16 토>.

Track [2026-08-17]--[2026-08-19]."#,
        );

        let maki = Maki::load(&project.root).unwrap();
        let property_date = Date::parse("2026-08-15").unwrap();
        let range_start_date = Date::parse("2026-08-17").unwrap();
        let middle_date = Date::parse("2026-08-18").unwrap();
        let range_end_date = Date::parse("2026-08-19").unwrap();

        let index_dates = maki
            .date_index()
            .dates()
            .map(|(date, _backlinks)| *date)
            .collect::<Vec<_>>();
        assert!(index_dates.contains(&property_date));
        assert!(index_dates.contains(&range_start_date));
        assert!(!index_dates.contains(&middle_date));
        assert!(index_dates.contains(&range_end_date));

        let property_backlinks = maki.date_index().backlinks_for(&property_date).unwrap();
        assert_eq!(property_backlinks.len(), 1);
        let property_occurrence = maki
            .date_index()
            .occurrence(property_backlinks[0].occurrence_id())
            .unwrap();
        assert!(matches!(
            property_occurrence.origin(),
            DateOrigin::Property { key } if key == "date"
        ));
        assert_eq!(property_occurrence.marker().raw(), "[2026-08-15]");

        let middle_backlinks = maki.date_index().backlinks_for(&middle_date).unwrap();
        assert_eq!(middle_backlinks.len(), 1);
        assert_eq!(middle_backlinks[0].relation(), DateRelation::RangeMiddle);
        let middle_occurrence = maki
            .date_index()
            .occurrence(middle_backlinks[0].occurrence_id())
            .unwrap();
        assert_eq!(
            middle_occurrence.marker().raw(),
            "[2026-08-17]--[2026-08-19]"
        );

        let html = maki.render_html(Path::new("start.maki")).unwrap();
        assert!(html.contains("href=\"/@/dates/2026-08-16#date-inline-start-maki-1\""));
        assert!(html.contains("href=\"/@/dates/2026-08-17#date-inline-start-maki-2\""));
        assert!(html.contains("href=\"/@/dates/2026-08-19#date-inline-start-maki-2\""));
    }

    #[test]
    fn date_index_orders_range_middle_backlinks_after_direct_dates() {
        let project = temp_project("date-index-priority");
        write_note_with_content(
            &project,
            "start.maki",
            r#"--^ title: Start

Track [2026-08-17]--[2026-08-19].

Target [2026-08-18]."#,
        );

        let maki = Maki::load(&project.root).unwrap();
        let middle_date = Date::parse("2026-08-18").unwrap();

        let middle_backlinks = maki.date_index().backlinks_for(&middle_date).unwrap();
        assert_eq!(middle_backlinks.len(), 2);
        assert_eq!(middle_backlinks[0].relation(), DateRelation::Single);
        assert_eq!(middle_backlinks[1].relation(), DateRelation::RangeMiddle);

        let index_backlinks = maki
            .date_index()
            .dates()
            .find_map(|(date, backlinks)| (*date == middle_date).then_some(backlinks))
            .unwrap();
        assert_eq!(index_backlinks.len(), 1);
        assert_eq!(index_backlinks[0].relation(), DateRelation::Single);
    }

    #[test]
    fn date_index_context_includes_parent_heading_and_top_list_item() {
        let project = temp_project("date-index-context");
        write_note_with_content(
            &project,
            "start.maki",
            r#"--^ title: Start

= Roadmap

- Decide timing
  - still thinking
  - [2026-08-15] done

== Sprint [2026-08-16]"#,
        );

        let maki = Maki::load(&project.root).unwrap();
        let nested_date = Date::parse("2026-08-15").unwrap();
        let heading_date = Date::parse("2026-08-16").unwrap();

        let nested_backlinks = maki.date_index().backlinks_for(&nested_date).unwrap();
        let nested_occurrence = maki
            .date_index()
            .occurrence(nested_backlinks[0].occurrence_id())
            .unwrap();
        assert_eq!(
            nested_occurrence.context(),
            "= Roadmap\n- Decide timing\n  - [2026-08-15] done"
        );

        let heading_backlinks = maki.date_index().backlinks_for(&heading_date).unwrap();
        let heading_occurrence = maki
            .date_index()
            .occurrence(heading_backlinks[0].occurrence_id())
            .unwrap();
        assert_eq!(
            heading_occurrence.context(),
            "= Roadmap\n== Sprint [2026-08-16]"
        );
    }

    #[test]
    fn search_entries_use_title_property_or_file_stem() {
        let project = temp_project("search-entry-title");
        write_note_with_content(&project, "alpha.maki", "--^ title: Alpha Note\n\nbody");
        write_note_with_content(&project, "beta-note.maki", "body");

        let maki = Maki::load(&project.root).unwrap();

        assert!(maki.search_entries().iter().any(|entry| {
            entry.title() == "Alpha Note"
                && entry.path() == "/alpha"
                && entry.source_path() == "alpha.maki"
        }));
        assert!(maki.search_entries().iter().any(|entry| {
            entry.title() == "beta-note"
                && entry.path() == "/beta-note"
                && entry.source_path() == "beta-note.maki"
        }));
    }

    #[test]
    fn search_titles_matches_case_insensitive_title_substrings() {
        let project = temp_project("search-title-match");
        write_note_with_content(&project, "alpha.maki", "--^ title: Alpha Note\n\nbody");
        write_note_with_content(&project, "beta.maki", "--^ title: Beta Note\n\nbody");
        write_note_with_content(&project, "gamma.maki", "--^ title: Gamma\n\nbody");

        let maki = Maki::load(&project.root).unwrap();
        let titles = maki
            .search_titles("NOTE", 10)
            .iter()
            .map(|entry| entry.title().to_string())
            .collect::<Vec<_>>();

        assert_eq!(titles, vec!["Beta Note", "Alpha Note"]);
    }
}
