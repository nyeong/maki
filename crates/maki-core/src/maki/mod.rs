//! Maki domain.
//!
//! ### Properties
//!
//! Parser가 해석한 maki 문서의 properties 중 일부에 의미를 담아 활용함
//!
//! 예)
//! - 문서의 `title`을 문서의 제목으로 활용함
//! - 문서의 `publish`를 publish 정책으로 활용함

use std::time::Duration;

pub const PROJECT_FILE_NAME: &str = "maki.toml";
pub(super) const MAKI_EXTENSION: &str = "maki";
pub(super) const MAKI_SOURCE_EXTENSION: &str = ".maki";

pub trait ProjectLoadMeter {
    fn record_project_load_phase(&self, phase: &'static str, duration: Duration);
}

pub(super) struct NoopProjectLoadMeter;

impl ProjectLoadMeter for NoopProjectLoadMeter {
    fn record_project_load_phase(&self, _phase: &'static str, _duration: Duration) {}
}

mod config;
mod dates;
mod diagnostics;
mod error;
mod files;
mod links;
mod note;
mod project;

pub use config::{HomeMode, MakiConfig, MakiConfigOverrides, PublishPolicy};
pub use dates::{
    DateBacklink, DateIndex, DateMarker, DateOccurrence, DateOrigin, DatePeriod, DateRelation,
};
pub(crate) use dates::{
    date_occurrence_href, date_page_path, date_year_page_path, inline_date_occurrence_id,
    property_date_occurrence_id,
};
pub use diagnostics::{ProjectDiagnostic, ProjectDiagnosticKind, ProjectDiagnosticSummary};
pub use error::Error;
pub use links::NoteLinkResolution;
pub(crate) use links::{is_external_href, note_link_target_for_href};
pub use note::{Note, NoteRef, RecentEntry, SearchEntry};
pub use project::{Maki, MakiRoute};

#[cfg(test)]
mod tests;
