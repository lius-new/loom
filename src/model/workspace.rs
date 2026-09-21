//! The set of open files and the active buffer.

use std::collections::HashMap;

use crate::model::buffer::TextBuffer;
use crate::model::document::{meta, FileId};

#[derive(Clone)]
pub struct Workspace {
    open: Vec<FileId>,
    active: Option<FileId>,
    buffers: HashMap<FileId, TextBuffer>,
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            open: Vec::new(),
            active: None,
            buffers: HashMap::new(),
        }
    }

    pub fn open_files(&self) -> &[FileId] {
        &self.open
    }

    /// The active file, if any. `None` means no file is open.
    pub fn active(&self) -> Option<FileId> {
        self.active
    }

    pub fn is_open(&self, id: FileId) -> bool {
        self.open.contains(&id)
    }

    pub fn active_buffer(&self) -> Option<&TextBuffer> {
        self.active.and_then(|id| self.buffers.get(&id))
    }

    pub fn active_buffer_mut(&mut self) -> Option<&mut TextBuffer> {
        self.active.and_then(|id| self.buffers.get_mut(&id))
    }

    /// Open a file: create its buffer from the catalog if needed, then activate it.
    pub fn open(&mut self, id: FileId) {
        if !self.buffers.contains_key(&id) {
            self.buffers.insert(id, TextBuffer::new(meta(id).code));
            self.open.push(id);
        }
        self.active = Some(id);
    }

    /// Switch to an already-open file. No-op if `id` isn't open.
    pub fn set_active(&mut self, id: FileId) {
        if self.buffers.contains_key(&id) {
            self.active = Some(id);
        }
    }

    pub fn close(&mut self, id: FileId) {
        if let Some(pos) = self.open.iter().position(|&f| f == id) {
            self.open.remove(pos);
            self.buffers.remove(&id);
            if self.active == Some(id) {
                self.active = if self.open.is_empty() {
                    None
                } else {
                    Some(self.open[pos.min(self.open.len() - 1)])
                };
            }
        }
    }

    pub fn next(&mut self) {
        if let Some(active) = self.active {
            if let Some(pos) = self.open.iter().position(|&f| f == active) {
                self.active = Some(self.open[(pos + 1) % self.open.len()]);
            }
        }
    }

    pub fn prev(&mut self) {
        if let Some(active) = self.active {
            if let Some(pos) = self.open.iter().position(|&f| f == active) {
                self.active = Some(self.open[(pos + self.open.len() - 1) % self.open.len()]);
            }
        }
    }
}
