//! Opaque runtime identities, independent of file metadata and pane geometry.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FileId(u64);

impl FileId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaneId(u64);

impl PaneId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}
