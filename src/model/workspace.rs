//! Open disk-backed documents and the active editor buffer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::model::buffer::TextBuffer;
use crate::model::document::{DiskState, FileId, FileMeta};

#[derive(Clone)]
struct OpenDocument {
    meta: FileMeta,
    buffer: TextBuffer,
    saved_text: String,
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

    pub fn file_id_for_path(&self, path: &Path) -> Option<FileId> {
        self.paths.get(path).copied()
    }

    pub fn active_buffer(&self) -> Option<&TextBuffer> {
        self.active
            .and_then(|id| self.documents.get(&id))
            .map(|document| &document.buffer)
    }

    pub fn active_buffer_mut(&mut self) -> Option<&mut TextBuffer> {
        self.active
            .and_then(|id| self.documents.get_mut(&id))
            .map(|document| &mut document.buffer)
    }

    pub fn is_dirty(&self, id: FileId) -> bool {
        self.documents
            .get(&id)
            .is_some_and(|document| document.buffer.text() != document.saved_text)
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
            .is_some_and(|document| document.disk_conflict)
    }

    pub fn is_missing_on_disk(&self, id: FileId) -> bool {
        self.documents
            .get(&id)
            .is_some_and(|document| document.missing_on_disk)
    }

    pub fn active_save_snapshot(&self) -> Option<(FileId, PathBuf, String)> {
        let id = self.active?;
        let document = self.documents.get(&id)?;
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

    pub fn set_active(&mut self, id: FileId) {
        if self.documents.contains_key(&id) {
            self.active = Some(id);
        }
    }

    pub fn close(&mut self, id: FileId) {
        if let Some(pos) = self.open.iter().position(|&file| file == id) {
            self.open.remove(pos);
            if let Some(document) = self.documents.remove(&id) {
                self.paths.remove(&document.meta.path);
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
