//! Explorer data shared by the model, application commands and views.
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One entry in a loaded directory listing. The full path is not stored per
/// entry: every entry of a listing shares its parent, and a path is built only
/// for the rows that need one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    parent: Arc<Path>,
}

impl DirEntry {
    pub fn new(parent: &Arc<Path>, name: String, is_dir: bool) -> Self {
        Self {
            name,
            is_dir,
            parent: Arc::clone(parent),
        }
    }

    pub fn path(&self) -> PathBuf {
        self.parent.join(&self.name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExplorerCreateKind {
    File,
    Folder,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplorerCreateRequest {
    pub kind: ExplorerCreateKind,
    pub parent: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplorerRenameRequest {
    pub path: PathBuf,
    pub kind: ExplorerTargetKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExplorerTargetKind {
    File,
    Directory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplorerContextTarget {
    pub path: PathBuf,
    pub kind: ExplorerTargetKind,
}
