use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use maki_core::{DatePeriod, Error as MakiError, Maki};

use super::error::Error;
use super::routes::cacheable_response;
use super::state::{AppState, ResponseCacheKey};

pub(super) fn response_cache_warmup_keys(maki: &Maki) -> Vec<ResponseCacheKey> {
    let mut keys = Vec::with_capacity(maki.notes_len() + 4);
    keys.push(ResponseCacheKey::MetaIndex);
    keys.push(ResponseCacheKey::Recents);
    keys.push(ResponseCacheKey::SearchIndex);
    keys.push(ResponseCacheKey::Diagnostics);
    keys.push(ResponseCacheKey::DatesIndex);
    let mut date_periods = BTreeSet::new();
    for (date, _backlinks) in maki.date_index().dates() {
        date_periods.insert(DatePeriod::Year(date.year()));
        date_periods.insert(DatePeriod::Month {
            year: date.year(),
            month: date.month(),
        });
        date_periods.insert(DatePeriod::Day(*date));
    }
    for (period, _backlinks) in maki.date_index().periods() {
        date_periods.insert(*period);
    }
    keys.extend(
        date_periods
            .into_iter()
            .map(ResponseCacheKey::DatePeriodPage),
    );
    keys.extend(
        maki.notes()
            .map(|note| ResponseCacheKey::NotePage(note.source_path().to_path_buf())),
    );
    keys
}

pub(super) fn warm_response_cache(state: &AppState) -> Result<(), Error> {
    let started = Instant::now();
    let keys = {
        let project = state.project.read().map_err(|_| Error::Maki {
            source: MakiError::ReadDirectoryFailed(PathBuf::from(".")),
        })?;
        response_cache_warmup_keys(&project.maki)
    };

    for key in keys {
        let kind = key.kind();
        let project = state.project.read().map_err(|_| Error::Maki {
            source: MakiError::ReadDirectoryFailed(PathBuf::from(".")),
        })?;

        match cacheable_response(state, &project, key, false) {
            Ok(_) => state
                .metrics()
                .record_response_cache_warmup_item(kind, "ok"),
            Err(error) => {
                state
                    .metrics()
                    .record_response_cache_warmup_item(kind, "error");
                eprintln!("Failed to warm response cache: {}", error);
            }
        }
    }

    state
        .metrics()
        .record_response_cache_warmup_duration(started.elapsed());
    Ok(())
}

pub(super) fn spawn_response_cache_warmer(state: Arc<AppState>) {
    thread::spawn(move || {
        if let Err(error) = warm_response_cache(&state) {
            eprintln!("Failed to warm response cache: {}", error);
        }
    });
}
