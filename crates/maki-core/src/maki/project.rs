use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::{
    analysis::{
        self, AnalysisBlockKind, BlockIdOccurrence, DocumentTitleOrigin, HeadingOccurrence,
        ProjectAnalysis, SourceSnapshot,
    },
    html::{self, AssetMode, DocumentNavigation, DocumentNavigationItem, NoteInfo, RenderContext},
    link_target::{InnerSelector, NoteLinkTarget},
    parser,
};

use super::{
    Error, MAKI_SOURCE_EXTENSION, NoopProjectLoadMeter, PROJECT_FILE_NAME, ProjectLoadMeter,
    SearchEntryKind, SitemapEntry,
    config::{MakiConfig, PublishPolicy},
    dates::{DateIndex, collect_date_index},
    files::{get_relative_path, list_maki_files},
    links::{
        ExternalLinkRef, NoteIndex, NoteLinkResolution, collect_external_links, normalize_key,
    },
    note::{
        Note, NoteMetadataEntry, NoteRef, RecentEntry, SearchEntry, collect_recent_entries,
        search_match_rank,
    },
};

pub struct Maki {
    pub(super) root: PathBuf,                  // canonical absolute path
    pub(super) notes: BTreeMap<NoteRef, Note>, // root-relative maki paths
    pub(super) index: NoteIndex,
    pub(super) snapshot: ProjectSnapshot,
    #[allow(dead_code)]
    pub(super) date_index: DateIndex,
    pub(super) external_links: Vec<ExternalLinkRef>,
    pub(super) search_entries: Vec<SearchEntry>,
    pub(super) recent_entries: Vec<RecentEntry>,
    pub(super) sitemap_entries: Vec<SitemapEntry>,
    pub(super) config: MakiConfig,
    snapshot_compile_duration: Duration,
}

pub(super) struct ProjectSnapshot {
    sources: BTreeMap<PathBuf, String>,
    read_failures: Vec<PathBuf>,
    analysis: ProjectAnalysis,
    title_origins: BTreeMap<PathBuf, DocumentTitleOrigin>,
}

impl ProjectSnapshot {
    fn new(sources: BTreeMap<PathBuf, String>, read_failures: Vec<PathBuf>) -> Self {
        let snapshots = sources
            .iter()
            .map(|(path, source)| SourceSnapshot {
                path: path.as_path(),
                source,
            })
            .collect::<Vec<_>>();
        let (analysis, title_origins) = analysis::analyze_project_with_title_origins(&snapshots);

        Self {
            sources,
            read_failures,
            analysis,
            title_origins,
        }
    }

    pub(super) fn source(&self, path: &Path) -> Option<&str> {
        self.sources.get(path).map(String::as_str)
    }

    pub(super) fn sources(&self) -> &BTreeMap<PathBuf, String> {
        &self.sources
    }

    pub(super) fn analysis(&self) -> &ProjectAnalysis {
        &self.analysis
    }

    fn title_origin(&self, path: &Path) -> DocumentTitleOrigin {
        self.title_origins
            .get(path)
            .copied()
            .unwrap_or(DocumentTitleOrigin::FileStem)
    }

    fn first_read_failure(&self) -> Option<&Path> {
        self.read_failures.first().map(PathBuf::as_path)
    }
}

fn snapshot_note_title(snapshot: &ProjectSnapshot, note: &Note) -> String {
    snapshot
        .analysis()
        .document(note.source_path())
        .map(|document| document.title.clone())
        .unwrap_or_else(|| note.file_stem().to_string())
}

fn snapshot_note_metadata_entry(snapshot: &ProjectSnapshot, note: &Note) -> NoteMetadataEntry {
    let Some(document) = snapshot.analysis().document(note.source_path()) else {
        return note.metadata_entry_with_title(note.file_stem().to_string(), true);
    };

    let title_matches_file_stem = note
        .source_path()
        .file_stem()
        .and_then(|file_stem| file_stem.to_str())
        == Some(document.title.as_str());
    // Exact authored file-stem titles use the same lossless Recents label policy as fallbacks.
    let uses_path_label = snapshot.title_origin(note.source_path())
        == DocumentTitleOrigin::FileStem
        || title_matches_file_stem;
    note.metadata_entry_with_title(document.title.clone(), uses_path_label)
}

#[derive(Debug, PartialEq)]
pub enum MakiRoute {
    Home,
    NotePage(PathBuf),
    SubdocumentsPage(PathBuf),
    NoteSource(PathBuf),
}

impl Maki {
    pub fn find_project_root(start: &Path) -> Result<Option<PathBuf>, Error> {
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

    fn note_by_source_path(&self, path: &Path) -> Option<&Note> {
        self.notes.values().find(|note| note.source_path() == path)
    }

    pub fn resolve_note_link(&self, current: &NoteRef, target: &str) -> NoteLinkResolution {
        let target = NoteLinkTarget::parse(target);
        let note_resolution = self.index.resolve_document(current, target.document);
        let NoteLinkResolution::Found(note_ref) = note_resolution else {
            return note_resolution;
        };

        match target.inner {
            None => NoteLinkResolution::Found(note_ref),
            Some(InnerSelector::Heading(heading)) => self.resolve_heading_target(note_ref, heading),
            Some(InnerSelector::Id(id)) => self.resolve_id_target(note_ref, id),
        }
    }

    fn resolve_heading_target(
        &self,
        note_ref: NoteRef,
        heading_anchor: &str,
    ) -> NoteLinkResolution {
        let Some(note) = self.note(&note_ref) else {
            return NoteLinkResolution::Broken;
        };
        let Some(document) = self.snapshot.analysis().document(note.source_path()) else {
            return NoteLinkResolution::Broken;
        };
        let headings = document.headings.iter();
        let exact = headings
            .clone()
            .filter(|heading| heading.anchor == heading_anchor)
            .collect::<Vec<_>>();
        let matches = if exact.is_empty() {
            let normalized = normalize_key(heading_anchor);
            headings
                .filter(|heading| normalize_key(&heading.anchor) == normalized)
                .collect::<Vec<_>>()
        } else {
            exact
        };

        match matches.as_slice() {
            [heading]
                if document
                    .block_ids
                    .iter()
                    .any(|block_id| block_id_conflicts_with_heading(block_id, heading)) =>
            {
                NoteLinkResolution::Ambiguous
            }
            [heading] => NoteLinkResolution::FoundHeading {
                note: note_ref,
                anchor: heading.anchor.clone(),
            },
            [] => NoteLinkResolution::Broken,
            _ => NoteLinkResolution::Ambiguous,
        }
    }

    fn resolve_id_target(&self, note_ref: NoteRef, id: &str) -> NoteLinkResolution {
        if id.is_empty() {
            return NoteLinkResolution::Broken;
        }
        let Some(note) = self.note(&note_ref) else {
            return NoteLinkResolution::Broken;
        };
        let Some(document) = self.snapshot.analysis().document(note.source_path()) else {
            return NoteLinkResolution::Broken;
        };
        let matches = document
            .block_ids
            .iter()
            .filter(|block_id| block_id.id == id)
            .collect::<Vec<_>>();

        match matches.as_slice() {
            [block_id]
                if document
                    .headings
                    .iter()
                    .any(|heading| block_id_conflicts_with_heading(block_id, heading)) =>
            {
                NoteLinkResolution::Ambiguous
            }
            [block_id] => NoteLinkResolution::FoundId {
                note: note_ref,
                id: block_id.id.clone(),
            },
            [] => NoteLinkResolution::Broken,
            _ => NoteLinkResolution::Ambiguous,
        }
    }

    pub fn get_raw_content(&self, path: &Path) -> Result<String, Error> {
        let Some(note) = self.note_by_source_path(path) else {
            return Err(Error::NoteNotFound(self.root.join(path)));
        };

        self.snapshot
            .source(note.source_path())
            .map(str::to_string)
            .ok_or_else(|| Error::ReadNoteFailed(note.absolute_path.clone()))
    }

    pub fn config(&self) -> &MakiConfig {
        &self.config
    }

    pub fn snapshot_compile_duration(&self) -> Duration {
        self.snapshot_compile_duration
    }

    /// Includes source-adapter finalization work in the recorded snapshot compile time.
    pub fn extend_snapshot_compile_duration(&mut self, duration: Duration) {
        self.snapshot_compile_duration = self.snapshot_compile_duration.saturating_add(duration);
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn notes(&self) -> impl Iterator<Item = &Note> {
        self.notes.values()
    }

    pub fn notes_len(&self) -> usize {
        self.notes.len()
    }

    pub fn search_entries(&self) -> &[SearchEntry] {
        &self.search_entries
    }

    pub fn published_search_entries(&self) -> &[SearchEntry] {
        match self.config.publish_policy() {
            PublishPolicy::PublishAll => &self.search_entries,
        }
    }

    pub fn recent_entries(&self) -> &[RecentEntry] {
        &self.recent_entries
    }

    pub fn apply_recent_modified_times(&mut self, modified_times: &BTreeMap<PathBuf, SystemTime>) {
        for note in self.notes.values_mut() {
            if let Some(modified) = modified_times.get(note.source_path()) {
                note.modified = Some(*modified);
            }
        }

        let note_metadata_entries = self
            .notes
            .values()
            .map(|note| snapshot_note_metadata_entry(&self.snapshot, note))
            .collect();
        self.recent_entries = collect_recent_entries(note_metadata_entries);
    }

    pub fn sitemap_entries(&self) -> &[SitemapEntry] {
        &self.sitemap_entries
    }

    pub fn published_sitemap_entries(&self) -> &[SitemapEntry] {
        match self.config.publish_policy() {
            PublishPolicy::PublishAll => &self.sitemap_entries,
        }
    }

    #[allow(dead_code)]
    pub fn date_index(&self) -> &DateIndex {
        &self.date_index
    }

    pub fn analysis(&self) -> Result<ProjectAnalysis, Error> {
        if let Some(path) = self.snapshot.first_read_failure() {
            return Err(Error::ReadNoteFailed(path.to_path_buf()));
        }

        Ok(self.snapshot.analysis().clone())
    }

    pub fn published_analysis(&self) -> Result<ProjectAnalysis, Error> {
        match self.config.publish_policy() {
            PublishPolicy::PublishAll => self.analysis(),
        }
    }

    pub fn search_titles(&self, query: &str, limit: usize) -> Vec<SearchEntry> {
        let query = normalize_key(query.trim());

        if query.is_empty() {
            return self
                .published_search_entries()
                .iter()
                .take(limit)
                .cloned()
                .collect();
        }

        let mut matches = self
            .published_search_entries()
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

    pub fn load_with_config(root: &Path, config: MakiConfig) -> Result<Self, Error> {
        Self::load_with_config_metered(root, config, &NoopProjectLoadMeter)
    }

    pub fn load_with_config_metered(
        root: &Path,
        config: MakiConfig,
        metrics: &impl ProjectLoadMeter,
    ) -> Result<Self, Error> {
        let snapshot_compile_started = Instant::now();

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
        let mut sources = BTreeMap::new();
        let mut read_failures = Vec::new();

        for file in &files {
            let note = Note::load(&root, file)?;
            match std::fs::read_to_string(&note.absolute_path) {
                Ok(source) => {
                    sources.insert(note.source_path().to_path_buf(), source);
                }
                Err(_) => read_failures.push(note.absolute_path.clone()),
            }
            notes.insert(note.note_ref(), note);
        }
        metrics.record_project_load_phase("load_notes", started.elapsed());

        let started = Instant::now();
        let snapshot = ProjectSnapshot::new(sources, read_failures);
        metrics.record_project_load_phase("analyze", started.elapsed());

        let started = Instant::now();
        let index = NoteIndex::build(notes.keys());
        metrics.record_project_load_phase("index", started.elapsed());

        let started = Instant::now();
        let date_index = collect_date_index(&notes, snapshot.sources());
        let external_links = collect_external_links(snapshot.sources());
        let note_metadata_entries = notes
            .values()
            .map(|note| snapshot_note_metadata_entry(&snapshot, note))
            .collect::<Vec<_>>();
        let search_entries =
            collect_search_entries(&notes, &note_metadata_entries, snapshot.analysis());
        let sitemap_entries = note_metadata_entries
            .iter()
            .cloned()
            .map(NoteMetadataEntry::into_sitemap_entry)
            .collect();
        let recent_entries = collect_recent_entries(note_metadata_entries);
        metrics.record_project_load_phase("metadata", started.elapsed());

        let snapshot_compile_duration = snapshot_compile_started.elapsed();

        Ok(Self {
            root,
            notes,
            index,
            snapshot,
            date_index,
            external_links,
            search_entries,
            recent_entries,
            sitemap_entries,
            config,
            snapshot_compile_duration,
        })
    }

    // root: absolute or relative to the project directory
    #[allow(dead_code)]
    pub fn load(root: impl AsRef<Path>) -> Result<Self, Error> {
        Self::load_with_config(root.as_ref(), MakiConfig::default())
    }

    pub fn render_html(&self, path: &Path) -> Result<String, Error> {
        self.render_html_with_asset_mode(path, AssetMode::Inline)
    }

    pub fn render_html_with_asset_mode(
        &self,
        path: &Path,
        asset_mode: AssetMode,
    ) -> Result<String, Error> {
        self.render_html_with_site_title(path, asset_mode, None)
    }

    pub fn render_html_with_site_title(
        &self,
        path: &Path,
        asset_mode: AssetMode,
        site_title: Option<&str>,
    ) -> Result<String, Error> {
        self.render_html_with_site_header(path, asset_mode, site_title, false)
    }

    pub fn render_html_with_site_header(
        &self,
        path: &Path,
        asset_mode: AssetMode,
        site_title: Option<&str>,
        site_header: bool,
    ) -> Result<String, Error> {
        let raw = self.get_raw_content(path)?;
        let parsed = parser::parse(&raw);
        let current = self
            .note_by_source_path(path)
            .ok_or_else(|| Error::NoteNotFound(self.root.join(path)))?
            .note_ref();

        let resolve_note_link = |target: &str| self.resolve_note_link(&current, target);
        let get_note_info = |note_ref: &NoteRef| {
            self.note(note_ref).map(|note| NoteInfo {
                title: snapshot_note_title(&self.snapshot, note),
            })
        };
        let document_navigation = self.document_navigation(&current);

        Ok(html::render_document_with_context(
            &parsed.document,
            RenderContext::project(&resolve_note_link, &get_note_info)
                .with_asset_mode(asset_mode)
                .with_date_source_path(path)
                .with_site_title(site_title)
                .with_site_header(site_header)
                .with_document_navigation(document_navigation),
        ))
    }

    pub fn render_subdocuments_html(&self, path: &Path) -> Result<String, Error> {
        self.render_subdocuments_html_with_asset_mode(path, AssetMode::Inline)
    }

    pub fn render_subdocuments_html_with_asset_mode(
        &self,
        path: &Path,
        asset_mode: AssetMode,
    ) -> Result<String, Error> {
        self.render_subdocuments_html_with_site_header(path, asset_mode, None, false)
    }

    pub fn render_subdocuments_html_with_site_header(
        &self,
        path: &Path,
        asset_mode: AssetMode,
        site_title: Option<&str>,
        site_header: bool,
    ) -> Result<String, Error> {
        let current = self
            .note_by_source_path(path)
            .ok_or_else(|| Error::NoteNotFound(self.root.join(path)))?
            .note_ref();
        let parent = self
            .document_navigation_item(&current)
            .ok_or_else(|| Error::NoteNotFound(self.root.join(path)))?;
        let children = self.published_document_navigation_children(&current);

        Ok(html::render_subdocuments_page(
            &parent,
            &children,
            asset_mode,
            site_title,
            site_header,
        ))
    }

    fn document_navigation(&self, current: &NoteRef) -> DocumentNavigation {
        let ancestors = match self.config.publish_policy() {
            PublishPolicy::PublishAll => self.document_navigation_ancestors(current),
        };
        let children = self.published_document_navigation_children(current);
        let has_subdocuments = !children.is_empty();
        let navigation = DocumentNavigation::from_ancestors(ancestors, children);

        if has_subdocuments {
            navigation.with_subdocuments_path(format!("{}/", current.web_path()))
        } else {
            navigation
        }
    }

    fn published_document_navigation_children(
        &self,
        current: &NoteRef,
    ) -> Vec<DocumentNavigationItem> {
        match self.config.publish_policy() {
            PublishPolicy::PublishAll => self
                .index
                .direct_children(current)
                .iter()
                .filter_map(|child| self.document_navigation_item(child))
                .collect(),
        }
    }

    fn document_navigation_ancestors(&self, current: &NoteRef) -> Vec<DocumentNavigationItem> {
        let mut ancestors = Vec::new();
        let mut descendant = current.clone();

        while let Some(parent) = self.index.direct_parent(&descendant) {
            let Some(item) = self.document_navigation_item(&parent) else {
                break;
            };
            ancestors.push(item);
            descendant = parent;
        }

        ancestors.reverse();
        ancestors
    }

    fn document_navigation_item(&self, note_ref: &NoteRef) -> Option<DocumentNavigationItem> {
        let note = self.note(note_ref)?;
        Some(DocumentNavigationItem::new(
            snapshot_note_title(&self.snapshot, note),
            note_ref.web_path(),
        ))
    }

    pub fn render_file_html(&self, file: &Path) -> Result<String, Error> {
        let absolute_path =
            std::fs::canonicalize(file).map_err(|_source| Error::NoteNotFound(file.to_owned()))?;
        let project_path = get_relative_path(&self.root, &absolute_path)?;

        self.render_html(&project_path)
    }

    /// Resolves a note path relative to the root directory.
    /// # Example
    /// ```text
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
    /// maki.resolve_route("/maki/"); // => MakiRoute::SubdocumentsPage("maki.maki")
    /// ```
    pub fn resolve_route(&self, target: &str) -> Result<MakiRoute, Error> {
        let target = target.strip_prefix('/').unwrap_or(target);

        if target.is_empty() {
            return Ok(MakiRoute::Home);
        }

        if let Some(note_target) = target.strip_suffix('/') {
            if note_target.is_empty()
                || note_target.ends_with('/')
                || note_target.ends_with(MAKI_SOURCE_EXTENSION)
            {
                return Err(Error::NoteNotFound(PathBuf::from(target)));
            }

            return match self.resolve_note_route(note_target)? {
                MakiRoute::NotePage(path) => Ok(MakiRoute::SubdocumentsPage(path)),
                MakiRoute::Home | MakiRoute::SubdocumentsPage(_) | MakiRoute::NoteSource(_) => {
                    unreachable!()
                }
            };
        }

        self.resolve_note_route(target)
    }
}

fn collect_search_entries(
    notes: &BTreeMap<NoteRef, Note>,
    metadata_entries: &[NoteMetadataEntry],
    analysis: &ProjectAnalysis,
) -> Vec<SearchEntry> {
    let mut entries = metadata_entries
        .iter()
        .cloned()
        .map(NoteMetadataEntry::into_search_entry)
        .collect::<Vec<_>>();

    for note in notes.values() {
        let source_path = note.source_path().display().to_string();
        entries.push(SearchEntry::new(
            SearchEntryKind::File,
            source_path.clone(),
            note.note_ref().web_path(),
            source_path.clone(),
        ));

        let Some(document) = analysis.document(note.source_path()) else {
            continue;
        };
        for heading in document.headings.iter().filter(|heading| {
            !document
                .block_ids
                .iter()
                .any(|block_id| block_id_conflicts_with_heading(block_id, heading))
        }) {
            entries.push(SearchEntry::new(
                SearchEntryKind::Heading,
                heading.title.clone(),
                format!("{}#{}", note.note_ref().web_path(), heading.anchor),
                format!("{source_path}#{}", heading.title),
            ));
        }
        for block_id in document.block_ids.iter().filter(|block_id| {
            document
                .block_ids
                .iter()
                .filter(|candidate| candidate.id == block_id.id)
                .count()
                == 1
                && !document
                    .headings
                    .iter()
                    .any(|heading| block_id_conflicts_with_heading(block_id, heading))
        }) {
            entries.push(SearchEntry::new(
                SearchEntryKind::Id,
                block_id.id.clone(),
                format!("{}#{}", note.note_ref().web_path(), block_id.id),
                format!("{source_path}@{}", block_id.id),
            ));
        }
    }

    entries
}

fn block_id_conflicts_with_heading(
    block_id: &BlockIdOccurrence,
    heading: &HeadingOccurrence,
) -> bool {
    block_id.id == heading.anchor
        && !(block_id.owner_kind == AnalysisBlockKind::Heading
            && block_id.owner_span == heading.span)
}
