//! Root-state bridge to the Explorer tree rules until R5.2 moves the cache
//! into the Explorer model. Rules live in `features::explorer::tree`; disk
//! access in `infrastructure::explorer_filesystem`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::features::explorer::ports::DirectoryPort;
use crate::features::explorer::tree::TreeCache;
use crate::infrastructure::explorer_filesystem::ExplorerFilesystem;
use crate::state::{AppState, DirEntry};

/// Read and sort one directory's entries (directories first, then name).
pub(crate) fn read_directory(dir: &Path) -> Arc<Vec<DirEntry>> {
    ExplorerFilesystem.read_directory(dir)
}

fn with_tree<R>(app: &mut AppState, update: impl FnOnce(&mut TreeCache, &[PathBuf]) -> R) -> R {
    let mut tree = TreeCache::from_parts(
        std::mem::take(&mut app.dir_entries),
        std::mem::take(&mut app.expanded),
    );
    let result = update(&mut tree, &app.workspace_folders);
    (app.dir_entries, app.expanded) = tree.into_parts();
    result
}

/// Refresh every directory listing Explorer has loaded, unless an inline
/// create/rename row is open; then the refresh is deferred.
pub(crate) fn refresh_all_loaded_directories(app: &mut AppState) -> bool {
    if explorer_edit_active(app) {
        app.pending_file_tree_refresh = true;
        return false;
    }
    with_tree(app, |tree, roots| {
        tree.refresh_all(&ExplorerFilesystem, roots)
    })
}

/// Refresh the parent listings of changed paths. If a changed path is itself
/// loaded, refresh it as well.
pub(crate) fn refresh_affected_paths(
    app: &mut AppState,
    paths: impl IntoIterator<Item = PathBuf>,
) -> bool {
    if explorer_edit_active(app) {
        app.pending_file_tree_refresh = true;
        return false;
    }
    with_tree(app, |tree, roots| {
        tree.refresh_affected(&ExplorerFilesystem, roots, paths)
    })
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

#[cfg(test)]
mod tests {
    use std::fs;
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
        assert!(!refresh_affected_paths(&mut app, [root.join("x.txt")]));
        assert!(app.pending_file_tree_refresh);
        app.explorer_create = None;
        fs::write(root.join("later.txt"), "later").unwrap();
        assert!(apply_pending_refresh(&mut app));
        assert!(!app.pending_file_tree_refresh);
        assert_eq!(
            app.dir_entries[&root.to_string_lossy().into_owned()][0].name,
            "later.txt"
        );

        fs::remove_dir_all(root).unwrap();
    }
}
