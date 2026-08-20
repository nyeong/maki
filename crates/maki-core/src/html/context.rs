use std::path::Path;

use crate::maki::{NoteLinkResolution, NoteRef};

pub struct NoteInfo {
    pub title: String,
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
}

pub(in crate::html) struct ProjectRenderContext<'a> {
    pub(in crate::html) resolve_note_link: NoteLinkResolver<'a>,
    pub(in crate::html) get_note: NoteInfoGetter<'a>,
}

type NoteLinkResolver<'a> = &'a dyn Fn(&str) -> NoteLinkResolution;
type NoteInfoGetter<'a> = &'a dyn Fn(&NoteRef) -> Option<NoteInfo>;
