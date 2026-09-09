//! Maki domain.
//!
//! ### Properties
//!
//! Parser가 해석한 maki 문서의 properties 중 일부에 의미를 담아 활용함
//!
//! 예)
//! - 문서의 `title`을 문서의 제목으로 활용함
//! - 문서의 `publish`를 publish 정책으로 활용함

pub const PROJECT_FILE_NAME: &str = "maki.toml";
pub(super) const MAKI_SOURCE_EXTENSION: &str = ".maki";

mod config;
mod dates;
mod diagnostics;
mod error;
mod links;
mod note;
mod project;

pub use config::{HomeMode, MakiConfig, MakiConfigOverrides, PublishPolicy};
pub use dates::{
    DateBacklink, DateIndex, DateMarker, DateOccurrence, DateOccurrenceKind, DateOrigin,
    DatePeriod, DateRelation,
};
pub(crate) use dates::{
    collect_parsed_document_dates, date_occurrence_href, date_page_path, date_year_page_path,
    inline_date_occurrence_id, property_date_occurrence_id,
};
pub use diagnostics::{
    ExternalLinkCheck, ProjectDiagnostic, ProjectDiagnosticKind, ProjectDiagnosticSummary,
    external_link_diagnostics,
};
pub use error::Error;
pub use links::NoteLinkResolution;
pub(crate) use links::{is_external_href, is_safe_direct_href};
pub use note::{Note, NoteRef, RecentEntry, SearchEntry, SearchEntryKind, SitemapEntry};
pub use project::{Maki, MakiRoute, ProjectSource};

#[cfg(test)]
mod tests;
