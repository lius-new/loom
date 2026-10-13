//! Lazily loaded directory listings and their expansion state.
//!
//! Watcher notifications are only hints. Refreshes always rebuild a cached
//! directory from the port so duplicate, reordered, or missing events
//! converge on the same state. Directories which were never loaded stay lazy.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::DirEntry;
use super::ports::DirectoryPort;

pub type Listings = HashMap<String, Arc<Vec<DirEntry>>>;

/// Cache key of a directory.
pub fn key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Directories first, then by name.
pub fn sort_entries(entries: &mut [DirEntry]) {
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeCache {
    listings: Listings,
    expanded: HashSet<String>,
}

impl TreeCache {
    pub fn from_parts(listings: Listings, expanded: HashSet<String>) -> Self {
        Self { listings, expanded }
    }

    pub fn into_parts(self) -> (Listings, HashSet<String>) {
        (self.listings, self.expanded)
    }

    /// Refresh one directory only when it is already loaded. Returns whether
    /// the cache changed.
    pub fn refresh_directory(&mut self, port: &dyn DirectoryPort, directory: &Path) -> bool {
        let key = key(directory);
        let Some(current) = self.listings.get(&key) else {
            return false;
        };
        let entries = port.read_directory(directory);
        if current == &entries {
            return false;
        }
        self.listings.insert(key, entries);
        true
    }

    /// Refresh selected directories and discard cached descendants that no
    /// longer exist.
    pub fn refresh_directories(
        &mut self,
        port: &dyn DirectoryPort,
        roots: &[PathBuf],
        directories: impl IntoIterator<Item = PathBuf>,
    ) -> bool {
        let mut changed = false;
        for directory in directories.into_iter().collect::<BTreeSet<_>>() {
            changed |= self.refresh_directory(port, &directory);
        }
        changed | self.prune_unavailable(port, roots)
    }

    /// Refresh every loaded directory. Bounded by the cache rather than a
    /// recursive scan of the roots.
    pub fn refresh_all(&mut self, port: &dyn DirectoryPort, roots: &[PathBuf]) -> bool {
        let directories = self.listings.keys().map(PathBuf::from).collect::<Vec<_>>();
        self.refresh_directories(port, roots, directories)
    }

    /// Refresh the parent listings of changed paths, and a changed path itself
    /// when it is loaded.
    pub fn refresh_affected(
        &mut self,
        port: &dyn DirectoryPort,
        roots: &[PathBuf],
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> bool {
        let mut directories = BTreeSet::new();
        for path in paths {
            if self.listings.contains_key(key(&path).as_str()) {
                directories.insert(path.clone());
            }
            if let Some(parent) = path.parent() {
                directories.insert(parent.to_path_buf());
            }
        }
        self.refresh_directories(port, roots, directories)
    }

    fn prune_unavailable(&mut self, port: &dyn DirectoryPort, roots: &[PathBuf]) -> bool {
        let should_keep = |path: &Path| {
            roots.iter().any(|root| root == path)
                || (port.is_dir(path) && roots.iter().any(|root| path.starts_with(root)))
        };

        let before_listings = self.listings.len();
        self.listings.retain(|key, _| should_keep(Path::new(key)));
        let before_expanded = self.expanded.len();
        self.expanded.retain(|key| should_keep(Path::new(key)));
        before_listings != self.listings.len() || before_expanded != self.expanded.len()
    }
}
