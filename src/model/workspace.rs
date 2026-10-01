//! Open disk-backed documents and the active editor buffer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::git::DiffTarget;
use crate::model::buffer::TextBuffer;
use crate::model::diff_document::DiffDocument;
use crate::model::document::{DiskState, FileId, FileMeta};

#[derive(Clone)]
struct OpenDocument {
    meta: FileMeta,
    buffer: TextBuffer,
    saved_text: String,
    diff: Option<DiffDocument>,
    scroll_x: f32,
    scroll_y: f32,
    disk_state: DiskState,
    disk_conflict: bool,
    missing_on_disk: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReconcileResult {
    Unchanged(PathBuf),
    Reloaded(PathBuf),
    Conflict(PathBuf),
    Missing(PathBuf),
    Failed(PathBuf, String),
}

#[derive(Clone)]
pub struct Workspace {
    open: Vec<FileId>,
    active: Option<FileId>,
    documents: HashMap<FileId, OpenDocument>,
    paths: HashMap<PathBuf, FileId>,
    next_file_id: u64,
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            open: Vec::new(),
            active: None,
            documents: HashMap::new(),
            paths: HashMap::new(),
            next_file_id: 1,
        }
    }

    pub fn open_files(&self) -> &[FileId] {
        &self.open
    }

    pub fn open_paths(&self) -> Vec<PathBuf> {
        self.open
            .iter()
            .filter_map(|id| self.documents.get(id))
            .filter(|document| document.diff.is_none())
            .map(|document| document.meta.path.clone())
            .collect()
    }

    pub fn active(&self) -> Option<FileId> {
        self.active
    }

    pub fn meta(&self, id: FileId) -> Option<&FileMeta> {
        self.documents.get(&id).map(|document| &document.meta)
    }

    pub fn active_meta(&self) -> Option<&FileMeta> {
        self.active.and_then(|id| self.meta(id))
    }

    pub fn active_path(&self) -> Option<&Path> {
        self.active_meta().map(|meta| meta.path.as_path())
    }

    pub fn active_diff(&self) -> Option<&DiffDocument> {
        self.active
            .and_then(|id| self.documents.get(&id))
            .and_then(|document| document.diff.as_ref())
    }

    pub fn is_diff(&self, id: FileId) -> bool {
        self.documents
            .get(&id)
            .is_some_and(|document| document.diff.is_some())
    }

    pub fn file_id_for_path(&self, path: &Path) -> Option<FileId> {
        self.paths.get(path).copied()
    }

    pub fn active_buffer(&self) -> Option<&TextBuffer> {
        self.active
            .and_then(|id| self.documents.get(&id))
            .filter(|document| document.diff.is_none())
            .map(|document| &document.buffer)
    }

    pub fn active_buffer_mut(&mut self) -> Option<&mut TextBuffer> {
        self.active
            .and_then(|id| self.documents.get_mut(&id))
            .filter(|document| document.diff.is_none())
            .map(|document| &mut document.buffer)
    }

    pub fn is_dirty(&self, id: FileId) -> bool {
        self.documents.get(&id).is_some_and(|document| {
            document.diff.is_none() && document.buffer.text() != document.saved_text
        })
    }

    pub fn dirty_paths(&self) -> Vec<PathBuf> {
        self.open
            .iter()
            .filter(|id| self.is_dirty(**id))
            .filter_map(|id| self.documents.get(id))
            .map(|document| document.meta.path.clone())
            .collect()
    }

    pub fn has_dirty_paths_under(&self, root: &Path) -> bool {
        self.documents
            .iter()
            .any(|(id, document)| document.meta.path.starts_with(root) && self.is_dirty(*id))
    }

    pub fn has_disk_conflict(&self, id: FileId) -> bool {
        self.documents
            .get(&id)
            .is_some_and(|document| document.diff.is_none() && document.disk_conflict)
    }

    pub fn is_missing_on_disk(&self, id: FileId) -> bool {
        self.documents
            .get(&id)
            .is_some_and(|document| document.diff.is_none() && document.missing_on_disk)
    }

    pub fn active_save_snapshot(&self) -> Option<(FileId, PathBuf, String)> {
        let id = self.active?;
        let document = self.documents.get(&id)?;
        if document.diff.is_some() {
            return None;
        }
        Some((
            id,
            document.meta.path.clone(),
            document.buffer.text().to_owned(),
        ))
    }

    pub fn mark_saved(&mut self, id: FileId) -> bool {
        let Some(document) = self.documents.get_mut(&id) else {
            return false;
        };
        if document.diff.is_some() {
            return false;
        }
        document.saved_text = document.buffer.text().to_owned();
        document.disk_state = DiskState::capture(&document.meta.path, &document.saved_text);
        document.disk_conflict = false;
        document.missing_on_disk = false;
        document.buffer.break_undo_group();
        true
    }

    pub fn active_scroll(&self) -> (f32, f32) {
        self.active
            .and_then(|id| self.documents.get(&id))
            .map_or((0.0, 0.0), |document| {
                (document.scroll_x, document.scroll_y)
            })
    }

    pub fn set_active_scroll(&mut self, x: f32, y: f32) {
        if let Some(document) = self.active.and_then(|id| self.documents.get_mut(&id)) {
            document.scroll_x = x.max(0.0);
            document.scroll_y = y.max(0.0);
        }
    }

    pub fn open_path(&mut self, path: PathBuf, contents: String) -> FileId {
        if let Some(id) = self.file_id_for_path(&path) {
            self.active = Some(id);
            return id;
        }

        let id = FileId::new(self.next_file_id);
        self.next_file_id += 1;
        let saved_text = contents.clone();
        let disk_state = DiskState::capture(&path, &contents);
        self.documents.insert(
            id,
            OpenDocument {
                meta: FileMeta::from_path(path.clone()),
                buffer: TextBuffer::new(contents),
                saved_text,
                diff: None,
                scroll_x: 0.0,
                scroll_y: 0.0,
                disk_state,
                disk_conflict: false,
                missing_on_disk: false,
            },
        );
        self.paths.insert(path, id);
        self.open.push(id);
        self.active = Some(id);
        id
    }

    /// Open or refresh a read-only diff tab. A tab is uniquely identified by
    /// repository, relative path and comparison target.
    pub fn open_diff(&mut self, diff: DiffDocument) -> FileId {
        if let Some((&id, document)) = self.documents.iter_mut().find(|(_, document)| {
            document.diff.as_ref().is_some_and(|current| {
                current.matches(&diff.repository_root, &diff.path, diff.target)
            })
        }) {
            document.meta.name = diff.title();
            document.meta.path = diff.absolute_path();
            document.diff = Some(diff);
            self.active = Some(id);
            return id;
        }

        let id = FileId::new(self.next_file_id);
        self.next_file_id += 1;
        let absolute_path = diff.absolute_path();
        let mut meta = FileMeta::from_path(absolute_path.clone());
        meta.name = diff.title();
        self.documents.insert(
            id,
            OpenDocument {
                meta,
                buffer: TextBuffer::new(String::new()),
                saved_text: String::new(),
                diff: Some(diff),
                scroll_x: 0.0,
                scroll_y: 0.0,
                disk_state: DiskState::capture(&absolute_path, ""),
                disk_conflict: false,
                missing_on_disk: false,
            },
        );
        self.open.push(id);
        self.active = Some(id);
        id
    }

    pub fn diff_id_for(
        &self,
        repository_root: &Path,
        path: &Path,
        target: DiffTarget,
    ) -> Option<FileId> {
        self.documents.iter().find_map(|(&id, document)| {
            document
                .diff
                .as_ref()
                .is_some_and(|diff| diff.matches(repository_root, path, target))
                .then_some(id)
        })
    }

    pub fn set_active(&mut self, id: FileId) {
        if self.documents.contains_key(&id) {
            self.active = Some(id);
        }
    }

    pub fn close(&mut self, id: FileId) {
        if let Some(pos) = self.open.iter().position(|&file| file == id) {
            self.open.remove(pos);
            if let Some(document) = self.documents.remove(&id) {
                if document.diff.is_none() {
                    self.paths.remove(&document.meta.path);
                }
            }
            if self.active == Some(id) {
                self.active = self
                    .open
                    .get(pos.min(self.open.len().saturating_sub(1)))
                    .copied();
            }
        }
    }

    pub fn next(&mut self) {
        if let Some(active) = self.active {
            if let Some(pos) = self.open.iter().position(|&file| file == active) {
                self.active = Some(self.open[(pos + 1) % self.open.len()]);
            }
        }
    }

    pub fn prev(&mut self) {
        if let Some(active) = self.active {
            if let Some(pos) = self.open.iter().position(|&file| file == active) {
                self.active = Some(self.open[(pos + self.open.len() - 1) % self.open.len()]);
            }
        }
    }

    /// Reconcile every open document with disk. Clean documents reload
    /// automatically; dirty documents retain their buffer and are marked as a
    /// conflict for explicit user resolution.
    pub fn reconcile_disk(&mut self) -> Vec<ReconcileResult> {
        let ids = self.open.clone();
        ids.into_iter()
            .map(|id| self.reconcile_document(id))
            .collect()
    }

    pub fn reconcile_document(&mut self, id: FileId) -> ReconcileResult {
        let Some(document) = self.documents.get(&id) else {
            return ReconcileResult::Failed(PathBuf::new(), "Document is not open.".into());
        };
        let path = document.meta.path.clone();
        if document.diff.is_some() {
            return ReconcileResult::Unchanged(path);
        }
        let dirty = document.buffer.text() != document.saved_text;
        let previous = document.disk_state.clone();
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Some(document) = self.documents.get_mut(&id) {
                    document.missing_on_disk = true;
                    document.disk_conflict = dirty;
                }
                return ReconcileResult::Missing(path);
            }
            Err(error) => return ReconcileResult::Failed(path, error.to_string()),
        };
        let current = DiskState::capture(&path, &contents);
        if previous.content_hash == current.content_hash && previous.size == current.size {
            if let Some(document) = self.documents.get_mut(&id) {
                document.disk_state = current;
            }
            return ReconcileResult::Unchanged(path);
        }
        let document = self.documents.get_mut(&id).expect("document remains open");
        document.missing_on_disk = false;
        if dirty {
            document.disk_conflict = true;
            ReconcileResult::Conflict(path)
        } else {
            document.buffer = TextBuffer::new(contents.clone());
            document.saved_text = contents;
            document.disk_state = current;
            document.disk_conflict = false;
            ReconcileResult::Reloaded(path)
        }
    }

    pub fn accept_disk_version(&mut self, id: FileId) -> Result<(), String> {
        let document = self
            .documents
            .get_mut(&id)
            .ok_or_else(|| "Document is not open.".to_owned())?;
        if document.diff.is_some() {
            return Err("Diff documents are read-only.".to_owned());
        }
        let contents = std::fs::read_to_string(&document.meta.path).map_err(|error| {
            format!("Could not reload {}: {error}", document.meta.path.display())
        })?;
        document.buffer = TextBuffer::new(contents.clone());
        document.saved_text = contents;
        document.disk_state = DiskState::capture(&document.meta.path, &document.saved_text);
        document.disk_conflict = false;
        document.missing_on_disk = false;
        Ok(())
    }

    pub fn keep_editor_version(&mut self, id: FileId) -> bool {
        let Some(document) = self.documents.get_mut(&id) else {
            return false;
        };
        if document.diff.is_some() {
            return false;
        }
        if let Ok(contents) = std::fs::read_to_string(&document.meta.path) {
            document.disk_state = DiskState::capture(&document.meta.path, &contents);
        }
        document.disk_conflict = false;
        true
    }

    pub fn move_open_path(&mut self, old_path: &Path, new_path: PathBuf) -> bool {
        let Some(id) = self.paths.remove(old_path) else {
            return false;
        };
        let Some(document) = self.documents.get_mut(&id) else {
            return false;
        };
        document.meta = FileMeta::from_path(new_path.clone());
        document.disk_state = DiskState::capture(&new_path, document.buffer.text());
        document.missing_on_disk = false;
        self.paths.insert(new_path, id);
        true
    }

    /// Retarget every open file below a renamed directory while preserving its
    /// buffer, dirty state, active tab, and scroll position.
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

    /// Close every editable document below a directory removed on disk.
    pub fn close_paths_under(&mut self, root: &Path) -> usize {
        let ids = self
            .paths
            .iter()
            .filter_map(|(path, id)| path.starts_with(root).then_some(*id))
            .collect::<Vec<_>>();
        let count = ids.len();
        for id in ids {
            self.close(id);
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::diff::UnifiedDiff;

    #[test]
    fn opens_disk_files_once_and_reactivates_existing_document() {
        let mut workspace = Workspace::new();
        let first = workspace.open_path(PathBuf::from("src/main.rs"), "fn main() {}".into());
        let second = workspace.open_path(PathBuf::from("README.md"), "hello".into());
        let reopened = workspace.open_path(PathBuf::from("src/main.rs"), "ignored".into());

        assert_eq!(reopened, first);
        assert_eq!(workspace.active(), Some(first));
        assert_eq!(workspace.open_files(), &[first, second]);
        assert_eq!(
            workspace.open_paths(),
            vec![PathBuf::from("src/main.rs"), PathBuf::from("README.md")]
        );
        assert_eq!(workspace.active_buffer().unwrap().text(), "fn main() {}");
    }

    #[test]
    fn closing_document_removes_its_path_and_activates_a_neighbor() {
        let mut workspace = Workspace::new();
        let first = workspace.open_path(PathBuf::from("first.txt"), "first".into());
        let second = workspace.open_path(PathBuf::from("second.txt"), "second".into());

        workspace.close(second);

        assert_eq!(workspace.active(), Some(first));
        assert_eq!(workspace.file_id_for_path(Path::new("second.txt")), None);
    }

    #[test]
    fn diff_tabs_are_reused_and_are_not_persisted_as_editable_files() {
        let mut workspace = Workspace::new();
        let file_path = PathBuf::from("repo/src/main.rs");
        let file = workspace.open_path(file_path.clone(), "fn main() {}".into());
        let make_diff = || {
            DiffDocument::from_unified(
                PathBuf::from("repo"),
                PathBuf::from("src/main.rs"),
                DiffTarget::IndexToWorktree,
                UnifiedDiff::default(),
            )
        };

        let first = workspace.open_diff(make_diff());
        let reopened = workspace.open_diff(make_diff());

        assert_eq!(first, reopened);
        assert_eq!(workspace.active(), Some(first));
        assert!(workspace.is_diff(first));
        assert!(workspace.active_buffer().is_none());
        assert_eq!(workspace.open_files(), &[file, first]);
        assert_eq!(workspace.open_paths(), vec![file_path]);
    }

    #[test]
    fn keeps_an_independent_scroll_position_for_each_document() {
        let mut workspace = Workspace::new();
        let first = workspace.open_path(PathBuf::from("first.txt"), "first".into());
        workspace.set_active_scroll(24.0, 120.0);
        let second = workspace.open_path(PathBuf::from("second.txt"), "second".into());
        workspace.set_active_scroll(8.0, 40.0);

        workspace.set_active(first);
        assert_eq!(workspace.active_scroll(), (24.0, 120.0));
        workspace.set_active(second);
        assert_eq!(workspace.active_scroll(), (8.0, 40.0));
    }

    #[test]
    fn dirty_state_tracks_content_against_the_opened_file() {
        let mut workspace = Workspace::new();
        let file = workspace.open_path(PathBuf::from("notes.txt"), "hello".into());

        assert!(!workspace.is_dirty(file));
        workspace.active_buffer_mut().unwrap().move_end();
        assert!(!workspace.is_dirty(file));

        workspace.active_buffer_mut().unwrap().insert("!");
        assert!(workspace.is_dirty(file));

        let (snapshot_id, path, contents) = workspace.active_save_snapshot().unwrap();
        assert_eq!(snapshot_id, file);
        assert_eq!(path, PathBuf::from("notes.txt"));
        assert_eq!(contents, "hello!");
        assert!(workspace.mark_saved(file));
        assert!(!workspace.is_dirty(file));

        workspace.active_buffer_mut().unwrap().backspace();
        assert!(workspace.is_dirty(file));
    }

    #[test]
    fn directory_rename_retargets_open_files_without_reopening_them() {
        let mut workspace = Workspace::new();
        let old_root = PathBuf::from("project/src");
        let new_root = PathBuf::from("project/source");
        let old_file = old_root.join("nested/main.rs");
        let new_file = new_root.join("nested/main.rs");
        let id = workspace.open_path(old_file.clone(), "fn main() {}".into());

        assert_eq!(workspace.move_open_paths_under(&old_root, &new_root), 1);
        assert_eq!(workspace.file_id_for_path(&old_file), None);
        assert_eq!(workspace.file_id_for_path(&new_file), Some(id));
        assert_eq!(workspace.active_path(), Some(new_file.as_path()));
    }

    #[test]
    fn directory_delete_closes_only_open_files_below_that_directory() {
        let mut workspace = Workspace::new();
        let removed = workspace.open_path(PathBuf::from("project/src/main.rs"), String::new());
        let kept_path = PathBuf::from("project/tests/main.rs");
        let kept = workspace.open_path(kept_path.clone(), String::new());

        assert_eq!(workspace.close_paths_under(Path::new("project/src")), 1);
        assert_eq!(
            workspace.file_id_for_path(Path::new("project/src/main.rs")),
            None
        );
        assert_eq!(workspace.file_id_for_path(&kept_path), Some(kept));
        assert_eq!(workspace.active(), Some(kept));
        assert_ne!(workspace.active(), Some(removed));
    }

    #[test]
    fn dirty_document_is_never_replaced_during_disk_reconciliation() {
        let root = std::env::temp_dir().join(format!(
            "loom-reconcile-{}-{}",
            std::process::id(),
            crate::model::document::content_hash(module_path!())
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("note.txt");
        std::fs::write(&path, "disk one").unwrap();
        let mut workspace = Workspace::new();
        let id = workspace.open_path(path.clone(), "disk one".into());
        workspace.active_buffer_mut().unwrap().move_end();
        workspace.active_buffer_mut().unwrap().insert(" + editor");
        std::fs::write(&path, "disk two with size").unwrap();

        assert_eq!(
            workspace.reconcile_document(id),
            ReconcileResult::Conflict(path.clone())
        );
        assert_eq!(
            workspace.active_buffer().unwrap().text(),
            "disk one + editor"
        );
        assert!(workspace.has_disk_conflict(id));
        let _ = std::fs::remove_dir_all(root);
    }
}
