//! Undo history: each entry holds the exact change sets of one undo step.

use std::time::{Duration, Instant};

use super::change::ChangeSet;
use super::selection::Selection;
type PairSnapshot = std::sync::Arc<Vec<super::pairs::Pair>>;

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
    pub metadata: Option<(u32, u32)>,
    pub ending: Option<(&'static str, &'static str)>,
    pub pairs: Option<(PairSnapshot, PairSnapshot)>,
    pub selections: Option<(Vec<Selection>, Vec<Selection>)>,
    /// Selection of the editing view before the step.
    pub before: Selection,
    /// Selection of the editing view after the step's last edit.
    pub after: Selection,
    /// The application-level change this step belongs to, if it was made as part
    /// of one spanning several documents.
    pub transaction: Option<u64>,
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
    /// Edits made while this is set are one step tagged with it.
    transaction: Option<u64>,
}

impl History {
    pub fn attach_selections(&mut self, before: Vec<Selection>, after: Vec<Selection>) {
        if let Some(entry) = self.undo.last_mut() {
            match &mut entry.selections {
                Some((_, last)) => *last = after,
                None => entry.selections = Some((before, after)),
            }
        }
    }
    pub fn update_selections(&mut self, primary: Selection, after: Vec<Selection>) {
        if let Some(entry) = self.undo.last_mut() {
            entry.after = primary;
            if let Some((_, last)) = &mut entry.selections {
                *last = after;
            }
        }
    }
    pub fn attach_ending(&mut self, before: &'static str, after: &'static str) {
        if let Some(entry) = self.undo.last_mut() {
            match &mut entry.ending {
                Some((_, last)) => *last = after,
                None => entry.ending = Some((before, after)),
            }
        }
    }

    pub fn attach_pairs(
        &mut self,
        before: std::sync::Arc<Vec<super::pairs::Pair>>,
        after: std::sync::Arc<Vec<super::pairs::Pair>>,
    ) {
        if let Some(entry) = self.undo.last_mut() {
            match &mut entry.pairs {
                Some((_, last)) => *last = after,
                None => entry.pairs = Some((before, after)),
            }
        }
    }

    pub fn attach_metadata(&mut self, before: u32, after: u32) {
        if let Some(entry) = self.undo.last_mut() {
            entry.metadata = Some((before, after));
        }
    }

    pub fn record_metadata(&mut self, before: u32, after: u32) {
        self.close_groups();
        self.redo.clear();
        if self.undo.len() >= MAX_ENTRIES {
            self.undo.remove(0);
        }
        self.undo.push(Entry {
            changes: Vec::new(),
            metadata: Some((before, after)),
            pairs: None,
            selections: None,
            ending: None,
            before: Selection::default(),
            after: Selection::default(),
            transaction: None,
        });
    }
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
        if let Some(id) = self.transaction {
            // A transaction is one step of its own, however many edits it makes.
            self.last_typing = None;
            match self.undo.last_mut() {
                Some(entry) if entry.transaction == Some(id) => {
                    entry.changes.push(changes);
                    entry.after = after;
                }
                _ => {
                    if self.undo.len() >= MAX_ENTRIES {
                        self.undo.remove(0);
                    }
                    self.undo.push(Entry {
                        metadata: None,
                        pairs: None,
                        selections: None,
                        ending: None,
                        changes: vec![changes],
                        before,
                        after,
                        transaction: Some(id),
                    });
                }
            }
            return;
        }
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
            Some(entry) if joins && entry.transaction.is_none() => {
                entry.changes.push(changes);
                entry.after = after;
            }
            _ => {
                if self.undo.len() >= MAX_ENTRIES {
                    self.undo.remove(0);
                }
                self.undo.push(Entry {
                    metadata: None,
                    pairs: None,
                    selections: None,
                    ending: None,
                    changes: vec![changes],
                    before,
                    after,
                    transaction: None,
                });
            }
        }
    }

    pub fn set_transaction(&mut self, id: Option<u64>) {
        self.transaction = id;
        self.last_typing = None;
    }

    pub fn top_undo_transaction(&self) -> Option<u64> {
        self.undo.last().and_then(|entry| entry.transaction)
    }

    pub fn top_redo_transaction(&self) -> Option<u64> {
        self.redo.last().and_then(|entry| entry.transaction)
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

    #[cfg(any(test, feature = "diagnostics"))]
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
