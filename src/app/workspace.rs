//! Application composition of the pure workspace and its platform adapters.
use crate::domain::identity::{FileId, PaneId};
use crate::domain::pane_layout::Direction;
use crate::domain::workspace as domain;
use crate::features::git::DiffRequest;
use crate::features::git::diff_document::DiffDocument;
use crate::infrastructure::workspace_filesystem as filesystem;
pub use domain::{AppPage, OpenMode, ReconcileResult};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
pub(crate) type Core = domain::Workspace<DiffDocument, DiffRequest>;
pub(crate) type PreparedTextFile = domain::PreparedTextFile<DiffDocument, DiffRequest>;
pub type PreparedDiskUpdate = domain::PreparedDiskUpdate<DiffDocument, DiffRequest>;
#[derive(Clone)]
pub struct Workspace {
    pub(crate) core: Core,
}
impl Deref for Workspace {
    type Target = Core;
    fn deref(&self) -> &Core {
        &self.core
    }
}
impl DerefMut for Workspace {
    fn deref_mut(&mut self) -> &mut Core {
        &mut self.core
    }
}
impl Workspace {
    pub fn new() -> Self {
        Self { core: Core::new() }
    }
    fn transaction<R>(&mut self, apply: impl FnOnce(&mut Core) -> R) -> R {
        let result = apply(&mut self.core);
        let retired = self.core.take_retired();
        for item in retired {
            match item {
                domain::Retired::Document(value) => {
                    crate::infrastructure::ui_dispatch::retire(value)
                }
                domain::Retired::Update(value) => crate::infrastructure::ui_dispatch::retire(value),
            }
        }
        result
    }
    pub fn mark_saved(&mut self, id: FileId) -> bool {
        let Some(doc) = self.core.documents.get(&id).filter(|doc| doc.is_text()) else {
            return false;
        };
        let disk = filesystem::capture(&doc.meta.path, doc.buffer.text());
        self.transaction(|core| core.mark_saved_with_disk(id, disk))
    }

    pub fn open_path(&mut self, path: PathBuf, contents: String) -> FileId {
        self.open_path_with_mode(path, contents, OpenMode::Permanent)
    }

    pub fn preview_path(&mut self, path: PathBuf, contents: String) -> FileId {
        self.open_path_with_mode(path, contents, OpenMode::Preview)
    }

    fn open_path_with_mode(&mut self, path: PathBuf, contents: String, mode: OpenMode) -> FileId {
        if let Some(id) = self.file_id_for_path(&path) {
            return self.transaction(|core| core.attach_text_tab(id, mode));
        }
        self.open_prepared_text(PreparedTextFile::new(path, contents), mode)
    }

    pub fn reconcile_disk(&mut self) -> Vec<ReconcileResult> {
        self.ordered_documents()
            .into_iter()
            .map(|id| self.reconcile_document(id))
            .collect()
    }

    pub fn prepare_disk_updates(
        &self,
        changed_paths: &[PathBuf],
        full_rescan: bool,
    ) -> Vec<PreparedDiskUpdate> {
        let mut working = self.clone();
        let ids = self
            .ordered_documents()
            .into_iter()
            .filter(|id| {
                self.core.documents.get(id).is_some_and(|document| {
                    document.is_text()
                        && (full_rescan
                            || changed_paths
                                .iter()
                                .any(|path| document.meta.path.starts_with(path)))
                })
            })
            .collect::<Vec<_>>();
        let mut updates = Vec::new();
        for id in ids {
            let original = &self.core.documents[&id];
            let outcome = working.reconcile_document(id);
            if matches!(outcome, ReconcileResult::Failed(_, _)) {
                continue;
            }
            let replacement = working.documents.remove(&id).unwrap();
            updates.push(PreparedDiskUpdate {
                id,
                path: original.meta.path.clone(),
                expected_disk: original.disk_state.clone(),
                expected_saved: original.saved_text.clone(),
                expected_version: original.buffer.version(),
                replacement,
                outcome,
            });
        }
        updates
    }

    pub fn reconcile_changed_paths(
        &mut self,
        changed_paths: &[PathBuf],
        full_rescan: bool,
    ) -> Vec<ReconcileResult> {
        let ids = self
            .ordered_documents()
            .into_iter()
            .filter(|id| {
                full_rescan
                    || self.core.documents.get(id).is_some_and(|document| {
                        document.is_text()
                            && changed_paths.iter().any(|changed| {
                                document.meta.path == *changed
                                    || document.meta.path.starts_with(changed)
                            })
                    })
            })
            .collect::<Vec<_>>();
        ids.into_iter()
            .map(|id| self.reconcile_document(id))
            .collect()
    }

    pub fn reconcile_document(&mut self, id: FileId) -> ReconcileResult {
        let observation = self
            .core
            .documents
            .get(&id)
            .filter(|doc| doc.is_text())
            .map_or(crate::domain::document::DiskRead::Missing, |doc| {
                filesystem::read(&doc.meta.path)
            });
        self.transaction(|core| core.reconcile_with_disk(id, observation))
    }

    pub fn accept_disk_version(&mut self, id: FileId) -> Result<(), String> {
        let document = self
            .core
            .documents
            .get(&id)
            .ok_or_else(|| "Document is not open.".to_owned())?;
        if !document.is_text() {
            return Err("Diff documents are read-only.".to_owned());
        }
        let contents = filesystem::read_contents(&document.meta.path).map_err(|error| {
            format!("Could not reload {}: {error}", document.meta.path.display())
        })?;
        let disk = filesystem::capture(&document.meta.path, &contents);
        self.transaction(|core| core.accept_disk_with(id, contents, disk))
    }

    pub fn keep_editor_version(&mut self, id: FileId) -> bool {
        let Some(document) = self.core.documents.get(&id).filter(|doc| doc.is_text()) else {
            return false;
        };
        let disk = filesystem::read_contents(&document.meta.path)
            .ok()
            .map(|contents| filesystem::capture(&document.meta.path, &contents));
        self.transaction(|core| core.keep_editor_with_disk(id, disk))
    }

    pub fn move_open_path(&mut self, old_path: &Path, new_path: PathBuf) -> bool {
        let Some(id) = self.core.paths.get(old_path) else {
            return false;
        };
        let Some(document) = self.core.documents.get(id) else {
            return false;
        };
        let disk = filesystem::capture(&new_path, document.buffer.text());
        self.transaction(|core| core.move_path_with_disk(old_path, new_path, disk))
    }

    pub fn move_open_paths_under(&mut self, old_root: &Path, new_root: &Path) -> usize {
        let moves = self
            .paths
            .keys()
            .filter_map(|path| {
                path.strip_prefix(old_root)
                    .ok()
                    .map(|relative| (path.clone(), new_root.join(relative)))
            })
            .collect::<Vec<_>>();
        let mut moved = 0;
        for (old_path, new_path) in moves {
            moved += usize::from(self.move_open_path(&old_path, new_path));
        }
        moved
    }
    pub fn close_item(&mut self, pane: PaneId, id: FileId) {
        self.transaction(|core| core.close_item(pane, id))
    }

    pub fn close(&mut self, id: FileId) {
        self.transaction(|core| core.close(id))
    }

    pub fn close_pane(&mut self, pane: PaneId) {
        self.transaction(|core| core.close_pane(pane))
    }

    pub fn close_paths_under(&mut self, root: &Path) -> usize {
        self.transaction(|core| core.close_paths_under(root))
    }

    pub(crate) fn open_prepared_text(
        &mut self,
        prepared: PreparedTextFile,
        mode: OpenMode,
    ) -> FileId {
        self.transaction(|core| core.open_prepared_text(prepared, mode))
    }

    pub(crate) fn install_restored(
        &mut self,
        id: FileId,
        path: &Path,
        prepared: PreparedTextFile,
    ) -> bool {
        self.transaction(|core| core.install_restored(id, path, prepared))
    }

    pub fn apply_disk_updates(&mut self, updates: Vec<PreparedDiskUpdate>) -> Vec<ReconcileResult> {
        self.transaction(|core| core.apply_disk_updates(updates))
    }

    pub fn split(&mut self, pane: PaneId, direction: Direction) -> Option<PaneId> {
        self.transaction(|core| core.split(pane, direction))
    }

    pub fn split_empty(&mut self, pane: PaneId, direction: Direction) -> Option<PaneId> {
        self.transaction(|core| core.split_empty(pane, direction))
    }

    pub fn split_with_item(
        &mut self,
        from: PaneId,
        id: FileId,
        target: PaneId,
        direction: Direction,
        copy: bool,
    ) -> bool {
        self.transaction(|core| core.split_with_item(from, id, target, direction, copy))
    }

    pub fn move_item(&mut self, from: PaneId, id: FileId, to: PaneId, index: usize) -> bool {
        self.transaction(|core| core.move_item(from, id, to, index))
    }

    pub fn copy_item(&mut self, from: PaneId, id: FileId, to: PaneId, index: usize) -> bool {
        self.transaction(|core| core.copy_item(from, id, to, index))
    }
    pub fn move_tab(&mut self, pane: PaneId, id: FileId, index: usize) -> bool {
        self.core.move_tab(pane, id, index)
    }
    pub fn move_active_item(&mut self, direction: Direction) -> bool {
        self.transaction(|core| core.move_active_item(direction))
    }
}
impl PreparedTextFile {
    pub fn new(path: PathBuf, contents: String) -> Self {
        let disk = filesystem::capture(&path, &contents);
        Self::with_disk_state(path, contents, disk)
    }
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
