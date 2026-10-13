//! Explorer data shared by the model, application commands and views.
use std::path::PathBuf;

/// One entry in a loaded directory listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExplorerCreateKind {
    File,
    Folder,
}

#[derive(Clone)]
pub struct ExplorerCreateRequest {
    pub kind: ExplorerCreateKind,
    pub parent: PathBuf,
}

#[derive(Clone)]
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
