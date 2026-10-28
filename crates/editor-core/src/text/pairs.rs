//! Automatically inserted delimiters belong to a buffer and its history.
use super::change::{Assoc, ChangeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pair {
    pub open: usize,
    pub close: usize,
}

impl Pair {
    pub fn transform(self, changes: &ChangeSet) -> Option<Self> {
        if changes
            .changes()
            .iter()
            .any(|c| c.old.contains(&self.open) || c.old.contains(&self.close))
        {
            return None;
        }
        Some(Self {
            open: changes.map(self.open, Assoc::After),
            close: changes.map(self.close, Assoc::After),
        })
    }
}
