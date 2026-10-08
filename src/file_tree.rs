//! Filesystem-backed directory listings used by the Explorer.
//!
//! Watcher notifications are only hints. This module always rebuilds a
//! cached directory from disk so duplicate, reordered, or missing events
//! converge on the same state.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::state::{AppState, DirEntry};

/// Read and sort one directory's entries (directories first, then name).
pub(crate) fn read_directory(dir: &Path) -> Arc<Vec<DirEntry>> {
    let mut entries = match fs::read_dir(dir) {
        Ok(iter) => iter
            .flatten()
            .map(|entry| DirEntry {
                is_dir: entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false),
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path(),
            })
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Arc::new(entries)
}

/// Refresh one directory only when it has already been loaded by Explorer.
/// Returns whether the cache changed.
pub(crate) fn refresh_cached_directory(app: &mut AppState, directory: &Path) -> bool {
    let key = directory.to_string_lossy().into_owned();
    let Some(current) = app.dir_entries.get(&key) else {
        return false;
    };
    let entries = read_directory(directory);
    if current == &entries {
        return false;
    }
    app.dir_entries.insert(key, entries);
    true
}

/// Refresh selected directories and discard cached descendants that no
/// longer exist. Directories which were never expanded remain lazy.
pub(crate) fn refresh_loaded_directories(
    app: &mut AppState,
    directories: impl IntoIterator<Item = PathBuf>,
) -> bool {
    if explorer_edit_active(app) {
        app.pending_file_tree_refresh = true;
        return false;
    }

    let mut changed = false;
    for directory in directories.into_iter().collect::<BTreeSet<_>>() {
        changed |= refresh_cached_directory(app, &directory);
    }
    changed | prune_unavailable_directory_caches(app)
}

/// Refresh every directory listing Explorer has loaded. This is deliberately
/// bounded by the cache rather than recursively scanning workspace roots.
pub(crate) fn refresh_all_loaded_directories(app: &mut AppState) -> bool {
    if explorer_edit_active(app) {
        app.pending_file_tree_refresh = true;
        return false;
    }

    let directories = app
        .dir_entries
        .keys()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    refresh_loaded_directories(app, directories)
}

/// Refresh the parent listings of changed paths. If a changed path is itself
/// loaded, refresh it as well.
pub(crate) fn refresh_affected_paths(
    app: &mut AppState,
    paths: impl IntoIterator<Item = PathBuf>,
) -> bool {
    let mut directories = BTreeSet::new();
    for path in paths {
        if app
            .dir_entries
            .contains_key(path.to_string_lossy().as_ref())
        {
            directories.insert(path.clone());
        }
        if let Some(parent) = path.parent() {
            directories.insert(parent.to_path_buf());
        }
    }
    refresh_loaded_directories(app, directories)
}

/// Apply a refresh which was deferred to protect an inline create/rename row.
pub(crate) fn apply_pending_refresh(app: &mut AppState) -> bool {
    if !app.pending_file_tree_refresh || explorer_edit_active(app) {
        return false;
    }
    app.pending_file_tree_refresh = false;
    refresh_all_loaded_directories(app)
}

fn explorer_edit_active(app: &AppState) -> bool {
    app.explorer_create.is_some() || app.explorer_rename.is_some()
}

fn prune_unavailable_directory_caches(app: &mut AppState) -> bool {
    let roots = app.workspace_folders.clone();
    let should_keep = |path: &Path| {
        roots.iter().any(|root| root == path)
            || (path.is_dir() && roots.iter().any(|root| path.starts_with(root)))
    };

    let before_entries = app.dir_entries.len();
    app.dir_entries.retain(|key, _| should_keep(Path::new(key)));
    let before_expanded = app.expanded.len();
    app.expanded.retain(|key| should_keep(Path::new(key)));
    before_entries != app.dir_entries.len() || before_expanded != app.expanded.len()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    fn temp_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "loom-file-tree-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn directory_read_sorts_folders_before_files() {
        let root = temp_directory("sort");
        fs::write(root.join("a.txt"), "a").unwrap();
        fs::create_dir(root.join("z-dir")).unwrap();

        let entries = read_directory(&root);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["z-dir", "a.txt"]
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cached_directory_refresh_is_idempotent() {
        let root = temp_directory("refresh");
        let key = root.to_string_lossy().into_owned();
        let mut app = AppState::new();
        app.workspace_folders.push(root.clone());
        app.dir_entries.insert(key.clone(), read_directory(&root));

        assert!(!refresh_cached_directory(&mut app, &root));
        fs::write(root.join("new.txt"), "new").unwrap();
        assert!(refresh_cached_directory(&mut app, &root));
        assert_eq!(app.dir_entries[&key][0].name, "new.txt");
        assert!(!refresh_cached_directory(&mut app, &root));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refresh_prunes_removed_descendant_cache() {
        let root = temp_directory("prune");
        let child = root.join("child");
        fs::create_dir(&child).unwrap();
        let root_key = root.to_string_lossy().into_owned();
        let child_key = child.to_string_lossy().into_owned();
        let mut app = AppState::new();
        app.workspace_folders.push(root.clone());
        app.dir_entries
            .insert(root_key.clone(), read_directory(&root));
        app.dir_entries.insert(child_key.clone(), Vec::new().into());
        app.expanded.insert(root_key);
        app.expanded.insert(child_key.clone());

        fs::remove_dir_all(&child).unwrap();
        assert!(refresh_all_loaded_directories(&mut app));
        assert!(!app.dir_entries.contains_key(&child_key));
        assert!(!app.expanded.contains(&child_key));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refresh_is_deferred_during_inline_edit() {
        let root = temp_directory("defer");
        let mut app = AppState::new();
        app.workspace_folders.push(root.clone());
        app.dir_entries
            .insert(root.to_string_lossy().into_owned(), Vec::new().into());
        app.explorer_create = Some(crate::state::ExplorerCreateRequest {
            kind: crate::state::ExplorerCreateKind::File,
            parent: root.clone(),
        });

        assert!(!refresh_all_loaded_directories(&mut app));
        assert!(app.pending_file_tree_refresh);
        app.explorer_create = None;
        fs::write(root.join("later.txt"), "later").unwrap();
        assert!(apply_pending_refresh(&mut app));
        assert!(!app.pending_file_tree_refresh);

        fs::remove_dir_all(root).unwrap();
    }
}
