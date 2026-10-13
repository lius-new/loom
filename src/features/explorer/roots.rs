//! Workspace root folders and the most-recently-used folder list.
use std::path::{Path, PathBuf};

use super::ports::DirectoryPort;

pub const MAX_RECENT_FOLDERS: usize = 8;

/// Canonical form of a folder; unresolvable paths are kept as given.
pub fn normalized(port: &dyn DirectoryPort, path: PathBuf) -> PathBuf {
    port.canonicalize(&path).unwrap_or(path)
}

/// Existing folders in their first-seen order, normalized and deduplicated.
pub fn existing_unique(port: &dyn DirectoryPort, paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut unique = Vec::new();
    for path in paths {
        if !port.is_dir(&path) {
            continue;
        }
        let path = normalized(port, path);
        if !unique.contains(&path) {
            unique.push(path);
        }
    }
    unique
}

/// Move `path` to the front of the recent list, bounded by
/// [`MAX_RECENT_FOLDERS`].
pub fn remember(recents: &mut Vec<PathBuf>, path: PathBuf) {
    recents.retain(|recent| recent != &path);
    recents.insert(0, path);
    recents.truncate(MAX_RECENT_FOLDERS);
}

/// Drop restored roots and recents which no longer exist, then make sure
/// every root also appears among the recents.
pub fn hydrate(port: &dyn DirectoryPort, roots: &mut Vec<PathBuf>, recents: &mut Vec<PathBuf>) {
    *roots = existing_unique(port, std::mem::take(roots));
    *recents = existing_unique(port, std::mem::take(recents));
    recents.truncate(MAX_RECENT_FOLDERS);
    for root in roots.iter() {
        if !recents.contains(root) {
            recents.push(root.clone());
        }
    }
    recents.truncate(MAX_RECENT_FOLDERS);
}

/// The deepest root containing `path`, so nested roots resolve to the most
/// specific one.
pub fn root_for_path<'a>(roots: &'a [PathBuf], path: &Path) -> Option<&'a PathBuf> {
    roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
}
