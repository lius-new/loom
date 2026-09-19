//! The set of open files and the active buffer.

use std::collections::HashMap;

use crate::model::buffer::TextBuffer;
use crate::model::document::{meta, FileId, FILE_ORDER};

#[derive(Clone)]
pub struct Workspace {
    open: Vec<FileId>,
    active: FileId,
    buffers: HashMap<FileId, TextBuffer>,
}

impl Workspace {
    pub fn new() -> Self {
        let mut buffers = HashMap::new();
        for id in FILE_ORDER {
            buffers.insert(id, TextBuffer::new(meta(id).code));
        }
        Self {
            open: FILE_ORDER.to_vec(),
            active: FileId::Wallet,
            buffers,
        }
    }

    pub fn open_files(&self) -> &[FileId] {
        &self.open
    }

    pub fn active(&self) -> FileId {
        self.active
    }

    pub fn is_open(&self, id: FileId) -> bool {
        self.open.contains(&id)
    }

    pub fn active_buffer(&self) -> &TextBuffer {
        self.buffers.get(&self.active).expect("active buffer exists")
    }

    pub fn active_buffer_mut(&mut self) -> &mut TextBuffer {
        self.buffers
            .get_mut(&self.active)
            .expect("active buffer exists")
    }

    pub fn set_active(&mut self, id: FileId) {
        if self.buffers.contains_key(&id) {
            self.active = id;
        }
    }

    pub fn close(&mut self, id: FileId) {
        if self.open.len() <= 1 {
            return; // keep at least one buffer
        }
        if let Some(pos) = self.open.iter().position(|&f| f == id) {
            self.open.remove(pos);
            self.buffers.remove(&id);
            if self.active == id {
                self.active = self.open[pos.min(self.open.len() - 1)];
            }
        }
    }

    pub fn next(&mut self) {
        if let Some(pos) = self.open.iter().position(|&f| f == self.active) {
            self.active = self.open[(pos + 1) % self.open.len()];
        }
    }

    pub fn prev(&mut self) {
        if let Some(pos) = self.open.iter().position(|&f| f == self.active) {
            self.active = self.open[(pos + self.open.len() - 1) % self.open.len()];
        }
    }
}
