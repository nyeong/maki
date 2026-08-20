use std::path::PathBuf;

use maki_core::DatePeriod;
use percent_encoding::percent_decode_str;

use super::DATES_PATH_PREFIX;
use super::error::Error;

pub(super) struct RequestTarget<'a> {
    pub(super) path: String,
    pub(super) query: Option<&'a str>,
}

pub(super) fn parse_request_target(target: &str) -> Result<RequestTarget<'_>, Error> {
    let (raw_path, query) = target
        .split_once('?')
        .map_or((target, None), |(path, query)| (path, Some(query)));
    let path = percent_decode_str(raw_path)
        .decode_utf8()
        .map_err(|_e| Error::BadRequest)?
        .to_string();

    Ok(RequestTarget { path, query })
}

pub(super) fn decode_query_component(raw: &str) -> Result<String, Error> {
    percent_decode_str(&raw.replace('+', " "))
        .decode_utf8()
        .map(|decoded| decoded.to_string())
        .map_err(|_e| Error::BadRequest)
}

pub(super) fn query_param(query: Option<&str>, name: &str) -> Result<Option<String>, Error> {
    let Some(query) = query else {
        return Ok(None);
    };

    for part in query.split('&') {
        let (raw_key, raw_value) = part.split_once('=').unwrap_or((part, ""));
        if decode_query_component(raw_key)? == name {
            return Ok(Some(decode_query_component(raw_value)?));
        }
    }

    Ok(None)
}

pub(super) fn date_period_for_dates_request_path(path: &str) -> Option<DatePeriod> {
    let raw = path.strip_prefix(DATES_PATH_PREFIX)?;

    if raw.is_empty() || raw.contains('/') {
        return None;
    }

    DatePeriod::parse_path_segment(raw)
}
pub(super) fn has_parent_dir_component(path: &str) -> bool {
    PathBuf::from(path)
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}
