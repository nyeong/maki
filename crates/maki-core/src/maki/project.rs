use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use crate::{
    analysis::{self, ProjectAnalysis, SourceSnapshot},
    html::{self, AssetMode, NoteInfo, RenderContext},
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
        normalize_note_link_target,
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
    #[allow(dead_code)]
    pub(super) date_index: DateIndex,
    pub(super) external_links: Vec<ExternalLinkRef>,
    pub(super) search_entries: Vec<SearchEntry>,
    pub(super) recent_entries: Vec<RecentEntry>,
    pub(super) sitemap_entries: Vec<SitemapEntry>,
    pub(super) config: MakiConfig,
}

#[derive(Debug, PartialEq)]
pub enum MakiRoute {
    Home,
    NotePage(PathBuf),
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

    pub fn resolve_note_link(&self, current: &NoteRef, target: &str) -> NoteLinkResolution {
        let Some((note_target, heading_anchor)) = target.split_once('#') else {
            return self.resolve_note_target(current, target);
        };
        if heading_anchor.is_empty() {
            return NoteLinkResolution::Broken;
        }

        let note_resolution = if note_target.is_empty() {
            NoteLinkResolution::Found(current.clone())
        } else {
            self.resolve_note_target(current, note_target)
        };
        let NoteLinkResolution::Found(note_ref) = note_resolution else {
            return note_resolution;
        };

        self.resolve_heading_target(note_ref, heading_anchor)
    }

    fn resolve_note_target(&self, current: &NoteRef, target: &str) -> NoteLinkResolution {
        let target = normalize_note_link_target(target);
        let target = target.as_str();

        if let Some(note_ref) = self.index.exact_path(Path::new(target)) {
            return NoteLinkResolution::Found(note_ref);
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

    fn resolve_heading_target(
        &self,
        note_ref: NoteRef,
        heading_anchor: &str,
    ) -> NoteLinkResolution {
        let Some(note) = self.note(&note_ref) else {
            return NoteLinkResolution::Broken;
        };
        let Ok(source) = std::fs::read_to_string(&note.absolute_path) else {
            return NoteLinkResolution::Broken;
        };
        let parsed = parser::parse(&source);
        let headings = parsed.document.blocks.iter().filter_map(|block| {
            let parser::BlockKind::Heading { raw_body, .. } = &block.kind else {
                return None;
            };
            Some(
                block
                    .property("id")
                    .filter(|id| !id.is_empty())
                    .unwrap_or(raw_body),
            )
        });
        let exact = headings
            .clone()
            .filter(|anchor| *anchor == heading_anchor)
            .collect::<Vec<_>>();
        let matches = if exact.is_empty() {
            let normalized = normalize_key(heading_anchor);
            headings
                .filter(|anchor| normalize_key(anchor) == normalized)
                .collect::<Vec<_>>()
        } else {
            exact
        };

        match matches.as_slice() {
            [anchor] => NoteLinkResolution::FoundHeading {
                note: note_ref,
                anchor: (*anchor).to_string(),
            },
            [] => NoteLinkResolution::Broken,
            _ => NoteLinkResolution::Ambiguous,
        }
    }

    pub fn get_raw_content(&self, path: &Path) -> Result<String, Error> {
        let path = self.root.join(path);

        if !path.exists() || !path.is_file() {
            return Err(Error::NoteNotFound(path));
        }

        std::fs::read_to_string(&path).map_err(|_source| Error::ReadNoteFailed(path))
    }

    pub fn config(&self) -> &MakiConfig {
        &self.config
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

        let note_metadata_entries = self.notes.values().map(Note::metadata_entry).collect();
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
        let mut sources = Vec::new();
        for note in self.notes.values() {
            sources.push((
                note.source_path().to_path_buf(),
                std::fs::read_to_string(&note.absolute_path)
                    .map_err(|_source| Error::ReadNoteFailed(note.absolute_path.clone()))?,
            ));
        }
        let snapshots = sources
            .iter()
            .map(|(path, source)| SourceSnapshot {
                path: path.as_path(),
                source,
            })
            .collect::<Vec<_>>();

        Ok(analysis::analyze_project(&snapshots))
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
        let note_metadata_entries = notes.values().map(Note::metadata_entry).collect::<Vec<_>>();
        let search_entries = collect_search_entries(&notes, &note_metadata_entries);
        let recent_entries = collect_recent_entries(note_metadata_entries);
        let sitemap_entries = notes
            .values()
            .map(Note::metadata_entry)
            .map(NoteMetadataEntry::into_sitemap_entry)
            .collect();
        metrics.record_project_load_phase("metadata", started.elapsed());

        Ok(Self {
            root,
            notes,
            index,
            date_index,
            external_links,
            search_entries,
            recent_entries,
            sitemap_entries,
            config,
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
                .with_date_source_path(path)
                .with_site_title(site_title),
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
    /// ```
    pub fn resolve_route(&self, target: &str) -> Result<MakiRoute, Error> {
        let target = target.strip_prefix('/').unwrap_or(target);

        if target.is_empty() {
            return Ok(MakiRoute::Home);
        }

        self.resolve_note_route(target)
    }
}

fn collect_search_entries(
    notes: &BTreeMap<NoteRef, Note>,
    metadata_entries: &[NoteMetadataEntry],
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

        let Ok(source) = std::fs::read_to_string(&note.absolute_path) else {
            continue;
        };
        let parsed = parser::parse(&source);
        for block in &parsed.document.blocks {
            let parser::BlockKind::Heading { raw_body, .. } = &block.kind else {
                continue;
            };
            let anchor = block
                .property("id")
                .filter(|id| !id.is_empty())
                .unwrap_or(raw_body);
            entries.push(SearchEntry::new(
                SearchEntryKind::Heading,
                *raw_body,
                format!("{}#{anchor}", note.note_ref().web_path()),
                format!("{source_path}#{}", *raw_body),
            ));
        }
    }

    entries
}
