use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::parser;

use super::note::NoteRef;

#[derive(Default)]
pub(super) struct NoteIndex {
    exact_paths: BTreeMap<PathBuf, NoteRef>,
    children_by_parent: BTreeMap<PathBuf, Vec<NoteRef>>,
}

impl NoteIndex {
    pub(super) fn build<'a>(note_refs: impl Iterator<Item = &'a NoteRef>) -> Self {
        let mut index = Self::default();

        for note_ref in note_refs {
            index.insert(note_ref);
        }

        index
    }

    fn insert(&mut self, note_ref: &NoteRef) {
        self.exact_paths
            .insert(note_ref.canonical_path().to_path_buf(), note_ref.clone());

        let parent = note_ref
            .canonical_path()
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf();
        push_candidate(&mut self.children_by_parent, parent, note_ref);
    }

    pub(super) fn exact_path(&self, target: &Path) -> Option<NoteRef> {
        self.exact_paths.get(target).cloned()
    }

    pub(super) fn direct_parent(&self, note_ref: &NoteRef) -> Option<NoteRef> {
        self.exact_path(note_ref.canonical_path().parent()?)
    }

    pub(super) fn direct_children(&self, note_ref: &NoteRef) -> &[NoteRef] {
        self.children_by_parent
            .get(note_ref.canonical_path())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

fn push_candidate<K>(map: &mut BTreeMap<K, Vec<NoteRef>>, key: K, note_ref: &NoteRef)
where
    K: Ord,
{
    map.entry(key).or_default().push(note_ref.clone());
}

pub(super) fn normalize_key(key: &str) -> String {
    key.to_lowercase()
}

pub fn is_external_href(target: &str) -> bool {
    let target = target.trim();

    target.starts_with("//") || parser::uri_scheme(target).is_some()
}

pub fn is_safe_direct_href(target: &str) -> bool {
    parser::is_local_link_target(target)
}

#[derive(Debug, PartialEq)]
pub enum NoteLinkResolution {
    Found(NoteRef),
    FoundHeading { note: NoteRef, anchor: String },
    FoundId { note: NoteRef, id: String },
    Broken,
    Ambiguous,
}
