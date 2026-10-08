//! Undo history: each entry holds the exact change sets of one undo step.

use std::time::{Duration, Instant};

use super::change::ChangeSet;
use super::selection::Selection;

/// Entries kept before the oldest is dropped.
const MAX_ENTRIES: usize = 500;
/// Typing of the same kind within this pause joins the previous undo step.
const TYPING_PAUSE: Duration = Duration::from_secs(1);

/// What an edit was, for joining consecutive typing into one undo step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Backspace,
    Delete,
    /// Never joins a neighbouring edit outside an explicit group.
    Other,
}

impl EditKind {
    fn joins_typing(self) -> bool {
        self != EditKind::Other
    }
}

#[derive(Clone, Debug)]
pub(super) struct Entry {
    pub changes: Vec<ChangeSet>,
    /// Selection of the editing view before the step.
    pub before: Selection,
    /// Selection of the editing view after the step's last edit.
    pub after: Selection,
}

#[derive(Clone, Debug, Default)]
pub(super) struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    /// Depth of nested explicit groups; edits inside join one entry.
    group_depth: usize,
    /// Whether the open group already pushed its entry.
    group_started: bool,
    /// Kind, time and resulting caret of the last edit outside a group.
    last_typing: Option<(EditKind, Instant, usize)>,
}

impl History {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn in_group(&self) -> bool {
        self.group_depth > 0
    }

    pub fn begin_group(&mut self) {
        if self.group_depth == 0 {
            self.group_started = false;
            self.last_typing = None;
        }
        self.group_depth += 1;
    }

    pub fn end_group(&mut self) {
        self.group_depth = self.group_depth.saturating_sub(1);
        if self.group_depth == 0 {
            self.group_started = false;
            self.last_typing = None;
        }
    }

    /// Stop the next edit from joining the current typing run. Explicit
    /// groups decide their own extent, so this does not split them.
    pub fn break_typing(&mut self) {
        self.last_typing = None;
    }

    /// Close every open group, e.g. before undoing.
    pub fn close_groups(&mut self) {
        self.group_depth = 0;
        self.group_started = false;
        self.last_typing = None;
    }

    pub fn record(
        &mut self,
        changes: ChangeSet,
        kind: EditKind,
        before: Selection,
        after: Selection,
    ) {
        self.redo.clear();
        let joins = if self.group_depth > 0 {
            std::mem::replace(&mut self.group_started, true)
        } else {
            let now = Instant::now();
            let joins = kind.joins_typing()
                && before.range().is_none()
                && self.last_typing.is_some_and(|(last, time, caret)| {
                    last == kind
                        && caret == before.cursor
                        && now.duration_since(time) < TYPING_PAUSE
                });
            self.last_typing = kind.joins_typing().then_some((kind, now, after.cursor));
            joins
        };
        match self.undo.last_mut() {
            Some(entry) if joins => {
                entry.changes.push(changes);
                entry.after = after;
            }
            _ => {
                if self.undo.len() >= MAX_ENTRIES {
                    self.undo.remove(0);
                }
                self.undo.push(Entry {
                    changes: vec![changes],
                    before,
                    after,
                });
            }
        }
    }

    pub fn pop_undo(&mut self) -> Option<Entry> {
        self.close_groups();
        self.undo.pop()
    }

    pub fn pop_redo(&mut self) -> Option<Entry> {
        self.close_groups();
        self.redo.pop()
    }

    pub fn push_undo(&mut self, entry: Entry) {
        self.undo.push(entry);
    }

    pub fn push_redo(&mut self, entry: Entry) {
        self.redo.push(entry);
    }

    #[cfg(test)]
    pub fn bytes(&self) -> usize {
        self.undo
            .iter()
            .chain(&self.redo)
            .flat_map(|entry| &entry.changes)
            .flat_map(|set| set.changes())
            .map(|change| change.removed.len() + change.inserted.len())
            .sum()
    }
}
