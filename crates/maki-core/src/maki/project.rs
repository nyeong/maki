use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::{
    analysis::{
        self, AnalysisBlockKind, BlockIdOccurrence, DefinitionTargetKind, DocumentTitleOrigin,
        HeadingOccurrence, LinkResolution, ProjectAnalysis, SnapshotRevision,
    },
    html::{self, AssetMode, DocumentNavigation, DocumentNavigationItem, NoteInfo, RenderContext},
    parser,
};

use super::{
    Error, MAKI_SOURCE_EXTENSION, SearchEntryKind, SitemapEntry,
    config::{MakiConfig, PublishPolicy},
    dates::DateIndex,
    links::{NoteIndex, NoteLinkResolution, normalize_key},
    note::{
        Note, NoteMetadataEntry, NoteRef, RecentEntry, SearchEntry, collect_recent_entries,
        search_match_rank,
    },
};

pub struct Maki {
    pub(super) root: PathBuf, // caller-supplied project identity root
    pub(super) notes: BTreeMap<NoteRef, Note>, // root-relative maki paths
    pub(super) index: NoteIndex,
    public_note_refs: BTreeSet<NoteRef>,
    public_index: NoteIndex,
    pub(super) snapshot: ProjectSnapshot,
    pub(super) search_entries: Vec<SearchEntry>,
    public_search_entries: Vec<SearchEntry>,
    pub(super) recent_entries: Vec<RecentEntry>,
    public_recent_entries: Vec<RecentEntry>,
    pub(super) sitemap_entries: Vec<SitemapEntry>,
    public_sitemap_entries: Vec<SitemapEntry>,
    pub(super) config: MakiConfig,
    snapshot_compile_duration: Duration,
}

pub(super) struct ProjectSnapshot {
    compiled: analysis::ProjectSnapshot,
    read_failures: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSource {
    path: PathBuf,
    source: Option<String>,
    modified: Option<SystemTime>,
}

impl ProjectSource {
    pub fn loaded(
        path: impl Into<PathBuf>,
        source: impl Into<String>,
        modified: Option<SystemTime>,
    ) -> Self {
        Self {
            path: path.into(),
            source: Some(source.into()),
            modified,
        }
    }

    pub fn read_failed(path: impl Into<PathBuf>, modified: Option<SystemTime>) -> Self {
        Self {
            path: path.into(),
            source: None,
            modified,
        }
    }
}

impl ProjectSnapshot {
    fn new(sources: BTreeMap<PathBuf, String>, read_failures: Vec<PathBuf>) -> Self {
        Self {
            compiled: analysis::ProjectSnapshot::compile(sources),
            read_failures,
        }
    }

    pub(super) fn source(&self, path: &Path) -> Option<&str> {
        self.compiled.source(path)
    }

    pub(super) fn analysis(&self) -> &ProjectAnalysis {
        self.compiled.analysis()
    }

    pub(super) fn public_analysis(&self) -> &ProjectAnalysis {
        self.compiled.public_analysis()
    }

    fn is_public_source(&self, path: &Path) -> bool {
        self.compiled.is_public_source(path)
    }

    fn title_origin(&self, path: &Path) -> DocumentTitleOrigin {
        self.compiled.title_origin(path)
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
    fn publish_policy(&self) -> PublishPolicy {
        *self.config.publish_policy()
    }

    fn is_note_visible(&self, note_ref: &NoteRef) -> bool {
        self.publish_policy() == PublishPolicy::Private || self.public_note_refs.contains(note_ref)
    }

    fn active_index(&self) -> &NoteIndex {
        match self.publish_policy() {
            PublishPolicy::Private => &self.index,
            PublishPolicy::Public => &self.public_index,
        }
    }

    pub(super) fn active_analysis(&self) -> &ProjectAnalysis {
        match self.publish_policy() {
            PublishPolicy::Private => self.snapshot.analysis(),
            PublishPolicy::Public => self.snapshot.public_analysis(),
        }
    }

    pub(super) fn snapshot_source(&self, path: &Path) -> Option<&str> {
        self.snapshot.source(path)
    }

    fn note(&self, note_ref: &NoteRef) -> Option<&Note> {
        self.is_note_visible(note_ref)
            .then(|| self.notes.get(note_ref))
            .flatten()
    }

    fn note_by_source_path(&self, path: &Path) -> Option<&Note> {
        self.notes
            .values()
            .find(|note| note.source_path() == path && self.is_note_visible(&note.note_ref()))
    }

    fn unresolved_note_link(&self, private_resolution: NoteLinkResolution) -> NoteLinkResolution {
        match self.publish_policy() {
            PublishPolicy::Private => private_resolution,
            PublishPolicy::Public => NoteLinkResolution::Redacted,
        }
    }

    pub fn resolve_note_link(&self, current: &NoteRef, target: &str) -> NoteLinkResolution {
        let Some(current_note) = self.note(current) else {
            return self.unresolved_note_link(NoteLinkResolution::Broken);
        };
        let resolution = self
            .active_analysis()
            .resolve_note_link(current_note.source_path(), target);

        match resolution {
            LinkResolution::Found(target) => {
                let Some(note) = self.note_by_source_path(&target.path) else {
                    return self.unresolved_note_link(NoteLinkResolution::Broken);
                };
                let note = note.note_ref();
                match (target.kind, target.fragment) {
                    (DefinitionTargetKind::Document, _) => NoteLinkResolution::Found(note),
                    (DefinitionTargetKind::Heading, Some(anchor)) => {
                        NoteLinkResolution::FoundHeading { note, anchor }
                    }
                    (DefinitionTargetKind::Id, Some(id)) => {
                        NoteLinkResolution::FoundId { note, id }
                    }
                    (DefinitionTargetKind::Heading | DefinitionTargetKind::Id, None) => {
                        self.unresolved_note_link(NoteLinkResolution::Broken)
                    }
                }
            }
            LinkResolution::BrokenNote
            | LinkResolution::BrokenHeading
            | LinkResolution::BrokenId => self.unresolved_note_link(NoteLinkResolution::Broken),
            LinkResolution::AmbiguousNote
            | LinkResolution::AmbiguousHeading
            | LinkResolution::AmbiguousId => {
                self.unresolved_note_link(NoteLinkResolution::Ambiguous)
            }
        }
    }

    pub fn get_raw_content(&self, path: &Path) -> Result<String, Error> {
        if self.publish_policy() == PublishPolicy::Public {
            return Err(Error::NoteNotFound(self.root.join(path)));
        }
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

    pub fn set_publish_policy(&mut self, publish_policy: PublishPolicy) {
        self.config.set_publish_policy(publish_policy);
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

    pub fn source(&self, path: &Path) -> Option<&str> {
        match self.publish_policy() {
            PublishPolicy::Private => self.snapshot.source(path),
            PublishPolicy::Public => None,
        }
    }

    pub fn validation_report(&self) -> analysis::ValidationReport<'_> {
        let unavailable_sources = match self.publish_policy() {
            PublishPolicy::Private => self.snapshot.read_failures.as_slice(),
            PublishPolicy::Public => &[],
        };
        analysis::ValidationReport::new(&self.active_analysis().diagnostics, unavailable_sources)
    }

    pub fn notes(&self) -> impl Iterator<Item = &Note> {
        self.notes
            .iter()
            .filter(|(note_ref, _note)| self.is_note_visible(note_ref))
            .map(|(_note_ref, note)| note)
    }

    pub fn notes_len(&self) -> usize {
        match self.publish_policy() {
            PublishPolicy::Private => self.notes.len(),
            PublishPolicy::Public => self.public_note_refs.len(),
        }
    }

    pub fn search_entries(&self) -> &[SearchEntry] {
        match self.publish_policy() {
            PublishPolicy::Private => &self.search_entries,
            PublishPolicy::Public => &self.public_search_entries,
        }
    }

    pub fn published_search_entries(&self) -> &[SearchEntry] {
        self.search_entries()
    }

    pub fn recent_entries(&self) -> &[RecentEntry] {
        match self.publish_policy() {
            PublishPolicy::Private => &self.recent_entries,
            PublishPolicy::Public => &self.public_recent_entries,
        }
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
            .collect::<Vec<_>>();
        let public_note_metadata_entries = self
            .notes
            .iter()
            .filter(|(note_ref, _note)| self.public_note_refs.contains(*note_ref))
            .map(|(_note_ref, note)| snapshot_note_metadata_entry(&self.snapshot, note))
            .collect();
        self.recent_entries = collect_recent_entries(note_metadata_entries);
        self.public_recent_entries = collect_recent_entries(public_note_metadata_entries);
    }

    pub fn sitemap_entries(&self) -> &[SitemapEntry] {
        match self.publish_policy() {
            PublishPolicy::Private => &self.sitemap_entries,
            PublishPolicy::Public => &self.public_sitemap_entries,
        }
    }

    pub fn published_sitemap_entries(&self) -> &[SitemapEntry] {
        self.sitemap_entries()
    }

    #[allow(dead_code)]
    pub fn date_index(&self) -> &DateIndex {
        self.active_analysis().date_index()
    }

    pub fn analysis(&self) -> Result<ProjectAnalysis, Error> {
        if self.publish_policy() == PublishPolicy::Private
            && let Some(path) = self.snapshot.first_read_failure()
        {
            return Err(Error::ReadNoteFailed(self.root.join(path)));
        }

        Ok(self.active_analysis().clone())
    }

    pub fn external_links(&self) -> &[analysis::ProjectExternalLink] {
        self.active_analysis().external_links()
    }

    pub fn snapshot_revision(&self) -> SnapshotRevision {
        self.snapshot.compiled.revision()
    }

    pub fn published_analysis(&self) -> Result<ProjectAnalysis, Error> {
        self.analysis()
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

    /// Compiles caller-supplied project values without reading the filesystem or clock.
    ///
    /// Filesystem adapters should canonicalize `root` before calling when they need a
    /// canonical absolute identity.
    pub fn compile(
        root: PathBuf,
        config: MakiConfig,
        project_sources: impl IntoIterator<Item = ProjectSource>,
    ) -> Self {
        let mut notes = BTreeMap::new();
        let mut sources = BTreeMap::new();
        let mut read_failures = Vec::new();

        for ProjectSource {
            path,
            source,
            modified,
        } in project_sources
        {
            let note = Note::new(&root, &path, modified);
            match source {
                Some(source) => {
                    sources.insert(path, source);
                }
                None => read_failures.push(path),
            }
            notes.insert(note.note_ref(), note);
        }

        read_failures.sort();

        let snapshot = ProjectSnapshot::new(sources, read_failures);

        let index = NoteIndex::build(notes.keys());
        let public_note_refs = notes
            .iter()
            .filter(|(_note_ref, note)| snapshot.is_public_source(note.source_path()))
            .map(|(note_ref, _note)| note_ref.clone())
            .collect::<BTreeSet<_>>();
        let public_index = NoteIndex::build(public_note_refs.iter());

        let note_metadata_entries = notes
            .values()
            .map(|note| snapshot_note_metadata_entry(&snapshot, note))
            .collect::<Vec<_>>();
        let public_note_metadata_entries = public_note_refs
            .iter()
            .filter_map(|note_ref| notes.get(note_ref))
            .map(|note| snapshot_note_metadata_entry(&snapshot, note))
            .collect::<Vec<_>>();
        let search_entries =
            collect_search_entries(notes.values(), &note_metadata_entries, snapshot.analysis());
        let public_search_entries = collect_search_entries(
            public_note_refs
                .iter()
                .filter_map(|note_ref| notes.get(note_ref)),
            &public_note_metadata_entries,
            snapshot.public_analysis(),
        );
        let sitemap_entries = note_metadata_entries
            .iter()
            .cloned()
            .map(NoteMetadataEntry::into_sitemap_entry)
            .collect();
        let public_sitemap_entries = public_note_metadata_entries
            .iter()
            .cloned()
            .map(NoteMetadataEntry::into_sitemap_entry)
            .collect();
        let recent_entries = collect_recent_entries(note_metadata_entries);
        let public_recent_entries = collect_recent_entries(public_note_metadata_entries);

        Self {
            root,
            notes,
            index,
            public_note_refs,
            public_index,
            snapshot,
            search_entries,
            public_search_entries,
            recent_entries,
            public_recent_entries,
            sitemap_entries,
            public_sitemap_entries,
            config,
            snapshot_compile_duration: Duration::ZERO,
        }
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
        let note = self
            .note_by_source_path(path)
            .ok_or_else(|| Error::NoteNotFound(self.root.join(path)))?;
        let raw = self
            .snapshot
            .source(note.source_path())
            .ok_or_else(|| Error::ReadNoteFailed(note.absolute_path.clone()))?;
        let parsed = parser::parse(raw);
        let current = note.note_ref();

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
        let ancestors = self.document_navigation_ancestors(current);
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
        self.active_index()
            .direct_children(current)
            .iter()
            .filter_map(|child| self.document_navigation_item(child))
            .collect()
    }

    fn document_navigation_ancestors(&self, current: &NoteRef) -> Vec<DocumentNavigationItem> {
        let mut ancestors = Vec::new();
        let mut descendant = current.clone();

        while let Some(parent) = self.active_index().direct_parent(&descendant) {
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

        if (is_source && self.publish_policy() == PublishPolicy::Public)
            || !self.notes().any(|n| n.project_path == relative_path)
        {
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

fn collect_search_entries<'a>(
    notes: impl IntoIterator<Item = &'a Note>,
    metadata_entries: &[NoteMetadataEntry],
    analysis: &ProjectAnalysis,
) -> Vec<SearchEntry> {
    let mut entries = metadata_entries
        .iter()
        .cloned()
        .map(NoteMetadataEntry::into_search_entry)
        .collect::<Vec<_>>();

    for note in notes {
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
