use std::path::Path;

use crate::parser::Date;

use super::period::date_page_path;

pub fn inline_date_occurrence_id(source_path: &Path, ordinal: usize) -> String {
    date_occurrence_id("inline", source_path, ordinal)
}

pub fn property_date_occurrence_id(source_path: &Path, ordinal: usize) -> String {
    date_occurrence_id("property", source_path, ordinal)
}

pub fn date_occurrence_href(date: Date, occurrence_id: &str) -> String {
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
