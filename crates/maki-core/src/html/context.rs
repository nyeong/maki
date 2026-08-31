use std::path::Path;

use crate::maki::{NoteLinkResolution, NoteRef};

pub struct NoteInfo {
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentNavigationItem {
    title: String,
    path: String,
}

impl DocumentNavigationItem {
    pub fn new(title: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            path: path.into(),
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentNavigation {
    ancestors: Vec<DocumentNavigationItem>,
    children: Vec<DocumentNavigationItem>,
    subdocuments_path: Option<String>,
}

impl DocumentNavigation {
    /// Builds navigation with at most one direct parent.
    pub fn new(
        parent: Option<DocumentNavigationItem>,
        children: Vec<DocumentNavigationItem>,
    ) -> Self {
        Self::from_ancestors(parent.into_iter().collect(), children)
    }

    /// Builds navigation from ancestors ordered root-first through the direct parent.
    pub fn from_ancestors(
        ancestors: Vec<DocumentNavigationItem>,
        children: Vec<DocumentNavigationItem>,
    ) -> Self {
        Self {
            ancestors,
            children,
            subdocuments_path: None,
        }
    }

    pub fn with_subdocuments_path(mut self, path: impl Into<String>) -> Self {
        self.subdocuments_path = Some(path.into());
        self
    }

    /// Returns the direct parent, which is the last ancestor in the breadcrumb.
    pub fn parent(&self) -> Option<&DocumentNavigationItem> {
        self.ancestors.last()
    }

    /// Returns ancestors ordered root-first through the direct parent.
    pub fn ancestors(&self) -> &[DocumentNavigationItem] {
        &self.ancestors
    }

    pub fn children(&self) -> &[DocumentNavigationItem] {
        &self.children
    }

    pub fn subdocuments_path(&self) -> Option<&str> {
        self.subdocuments_path.as_deref()
    }

    pub fn is_empty(&self) -> bool {
        self.ancestors.is_empty() && self.children.is_empty() && self.subdocuments_path.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AssetMode {
    #[default]
    Inline,
    External,
}

#[derive(Default)]
pub struct RenderContext<'a> {
    pub(in crate::html) project: Option<ProjectRenderContext<'a>>,
    pub(in crate::html) asset_mode: AssetMode,
    pub(in crate::html) project_navigation: bool,
    pub(in crate::html) date_source_path: Option<&'a Path>,
    pub(in crate::html) site_title: Option<&'a str>,
    pub(in crate::html) site_header: bool,
    pub(in crate::html) document_navigation: Option<DocumentNavigation>,
}

impl<'a> RenderContext<'a> {
    pub fn project(resolve_note_link: NoteLinkResolver<'a>, get_note: NoteInfoGetter<'a>) -> Self {
        Self {
            project: Some(ProjectRenderContext {
                resolve_note_link,
                get_note,
            }),
            asset_mode: AssetMode::Inline,
            project_navigation: true,
            date_source_path: None,
            site_title: None,
            site_header: false,
            document_navigation: None,
        }
    }

    pub fn with_asset_mode(mut self, asset_mode: AssetMode) -> Self {
        self.asset_mode = asset_mode;
        self
    }

    pub fn with_project_navigation(mut self) -> Self {
        self.project_navigation = true;
        self
    }

    pub fn with_date_source_path(mut self, path: &'a Path) -> Self {
        self.date_source_path = Some(path);
        self
    }

    pub fn with_site_title(mut self, site_title: Option<&'a str>) -> Self {
        self.site_title = site_title.filter(|title| !title.trim().is_empty());
        self
    }

    pub fn with_site_header(mut self, enabled: bool) -> Self {
        self.site_header = enabled;
        self
    }

    pub fn with_document_navigation(mut self, navigation: DocumentNavigation) -> Self {
        self.document_navigation = Some(navigation);
        self
    }
}

pub(in crate::html) struct ProjectRenderContext<'a> {
    pub(in crate::html) resolve_note_link: NoteLinkResolver<'a>,
    pub(in crate::html) get_note: NoteInfoGetter<'a>,
}

type NoteLinkResolver<'a> = &'a dyn Fn(&str) -> NoteLinkResolution;
type NoteInfoGetter<'a> = &'a dyn Fn(&NoteRef) -> Option<NoteInfo>;
