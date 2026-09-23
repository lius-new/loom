//! Open disk-backed documents and the active editor buffer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::model::buffer::TextBuffer;
use crate::model::document::{FileId, FileMeta};

#[derive(Clone)]
struct OpenDocument {
    meta: FileMeta,
    buffer: TextBuffer,
    saved_text: String,
    scroll_x: f32,
    scroll_y: f32,
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
        self.documents.insert(
            id,
            OpenDocument {
                meta: FileMeta::from_path(path.clone()),
                buffer: TextBuffer::new(contents),
                saved_text,
                scroll_x: 0.0,
                scroll_y: 0.0,
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
}
