//! A view's selection: an anchor and an active end.

use std::ops::Range;

use super::change::{Assoc, ChangeSet};

/// Caret and optional anchor of one view, as byte offsets into the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// The active end, where the caret is drawn.
    pub cursor: usize,
    pub anchor: Option<usize>,
    /// Display column kept across vertical moves through shorter lines.
    pub preferred_column: Option<usize>,
}

impl Selection {
    pub fn caret(cursor: usize) -> Self {
        Self {
            cursor,
            ..Self::default()
        }
    }

    /// A selection from `anchor` to `cursor`.
    pub fn spanning(anchor: usize, cursor: usize) -> Self {
        Self {
            cursor,
            anchor: Some(anchor),
            preferred_column: None,
        }
    }

    /// The selected range, empty selections excluded.
    pub fn range(&self) -> Option<Range<usize>> {
        self.anchor
            .filter(|&anchor| anchor != self.cursor)
            .map(|anchor| anchor.min(self.cursor)..anchor.max(self.cursor))
    }

    /// Follow changes made through another view: each end maps on its own
    /// and stays before text inserted at its position.
    pub fn transform(&mut self, changes: &ChangeSet) {
        self.cursor = changes.map(self.cursor, Assoc::Before);
        self.anchor = self.anchor.map(|anchor| changes.map(anchor, Assoc::Before));
        self.preferred_column = None;
    }
}
