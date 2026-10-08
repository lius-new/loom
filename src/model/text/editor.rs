//! A document seen through one view's selection.
//!
//! `EditorMut` is the only way to change a document's text. It applies the
//! edits, sets the editing view's selection, moves every other view's
//! selection across the change and records the change in the history.

use std::ops::{Deref, Range};

use super::buffer::TextBuffer;
use super::change::{Assoc, ChangeSet, Edit, EditError};
use super::history::EditKind;
use super::selection::Selection;

/// Read-only view of a buffer through one selection.
#[derive(Clone, Copy)]
pub struct Editor<'a> {
    buffer: &'a TextBuffer,
    selection: Selection,
}

impl Deref for Editor<'_> {
    type Target = TextBuffer;
    fn deref(&self) -> &TextBuffer {
        self.buffer
    }
}

impl<'a> Editor<'a> {
    pub fn new(buffer: &'a TextBuffer, selection: Selection) -> Self {
        Self { buffer, selection }
    }

    pub fn buffer(&self) -> &'a TextBuffer {
        self.buffer
    }

    pub fn cursor(&self) -> usize {
        self.selection.cursor
    }

    pub fn selection_state(&self) -> Selection {
        self.selection
    }

    pub fn selection(&self) -> Option<Range<usize>> {
        self.selection.range()
    }

    pub fn selected_text(&self) -> Option<&'a str> {
        let buffer = self.buffer;
        self.selection().map(|range| &buffer.text()[range])
    }

    pub fn line_col(&self) -> (usize, usize) {
        self.buffer.line_col(self.selection.cursor)
    }
}

/// Editable view of a buffer through one selection. `peers` are the
/// selections of the document's other views.
pub struct EditorMut<'a> {
    buffer: &'a mut TextBuffer,
    selection: &'a mut Selection,
    peers: Vec<&'a mut Selection>,
}

impl Deref for EditorMut<'_> {
    type Target = TextBuffer;
    fn deref(&self) -> &TextBuffer {
        self.buffer
    }
}

impl<'a> EditorMut<'a> {
    pub fn new(
        buffer: &'a mut TextBuffer,
        selection: &'a mut Selection,
        peers: Vec<&'a mut Selection>,
    ) -> Self {
        Self {
            buffer,
            selection,
            peers,
        }
    }

    pub fn as_ref(&self) -> Editor<'_> {
        Editor::new(self.buffer, *self.selection)
    }

    pub fn buffer(&self) -> &TextBuffer {
        self.buffer
    }

    pub fn cursor(&self) -> usize {
        self.selection.cursor
    }

    pub fn selection_state(&self) -> Selection {
        *self.selection
    }

    pub fn selection(&self) -> Option<Range<usize>> {
        self.selection.range()
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.selection().map(|range| &self.buffer.text()[range])
    }

    pub fn line_col(&self) -> (usize, usize) {
        self.buffer.line_col(self.selection.cursor)
    }

    // ---- Selection -----------------------------------------------------------

    /// Replace this view's selection. Ends snap to grapheme boundaries, and
    /// the next edit starts a new undo step.
    pub fn set_selection(&mut self, selection: Selection) {
        self.buffer.break_undo_group();
        *self.selection = self.buffer.clamp(selection);
    }

    pub fn set_cursor(&mut self, offset: usize) {
        self.select_to(offset, false);
    }

    /// Move the cursor to `offset`, keeping (or starting) the anchor when
    /// `extend` is set and dropping it otherwise.
    pub fn select_to(&mut self, offset: usize, extend: bool) {
        let anchor = extend.then(|| self.selection.anchor.unwrap_or(self.selection.cursor));
        self.set_selection(Selection {
            cursor: offset,
            anchor,
            preferred_column: None,
        });
    }

    pub fn select_range(&mut self, range: Range<usize>) {
        self.set_selection(Selection::spanning(range.start, range.end));
    }

    pub fn select_all(&mut self) {
        self.select_range(0..self.buffer.len());
    }

    pub fn clear_selection(&mut self) {
        self.buffer.break_undo_group();
        self.selection.anchor = None;
    }

    /// Keep a display column for the next vertical move.
    pub fn set_preferred_column(&mut self, column: Option<usize>) {
        self.selection.preferred_column = column;
    }

    // ---- Editing -------------------------------------------------------------

    /// Apply `edits` (all in the current text's offsets) as one change and
    /// give this view the selection `select` computes from the result. Other
    /// views follow the change. Invalid edits change nothing.
    pub fn edit(
        &mut self,
        kind: EditKind,
        edits: Vec<Edit>,
        select: impl FnOnce(&ChangeSet, &TextBuffer) -> Selection,
    ) -> Result<ChangeSet, EditError> {
        let before = *self.selection;
        let changes = self.buffer.apply_edits(edits)?;
        if changes.is_empty() {
            return Ok(changes);
        }
        let after = self.buffer.clamp(select(&changes, self.buffer));
        *self.selection = after;
        self.move_peers(&changes);
        self.buffer
            .history_mut()
            .record(changes.clone(), kind, before, after);
        Ok(changes)
    }

    /// Replace `range` with `text` and put the caret after the inserted text.
    pub fn replace(
        &mut self,
        range: Range<usize>,
        text: &str,
        kind: EditKind,
    ) -> Result<ChangeSet, EditError> {
        let start = range.start;
        self.edit(kind, vec![Edit::replace(range, text)], |_, buffer| {
            let end = start + text.len();
            // Combining input can join the preceding grapheme; keep the caret
            // after the whole cluster.
            Selection::caret(if buffer.is_grapheme_boundary(end) {
                end
            } else {
                buffer.next_grapheme(end)
            })
        })
    }

    /// Replace the selection, or insert at the caret, and put the caret after
    /// the inserted text.
    pub fn replace_selection(
        &mut self,
        text: &str,
        kind: EditKind,
    ) -> Result<ChangeSet, EditError> {
        let cursor = self.selection.cursor;
        let range = self.selection().unwrap_or(cursor..cursor);
        self.replace(range, text, kind)
    }

    fn move_peers(&mut self, changes: &ChangeSet) {
        for peer in &mut self.peers {
            peer.transform(changes);
            **peer = self.buffer.clamp(**peer);
        }
    }

    // ---- History -------------------------------------------------------------

    /// Join every edit until the matching `end_group` into one undo step.
    /// Groups nest; the outermost one decides the step.
    pub fn begin_group(&mut self) {
        self.buffer.history_mut().begin_group();
    }

    pub fn end_group(&mut self) {
        self.buffer.history_mut().end_group();
    }

    /// Start a new undo step with the next edit (outside explicit groups).
    pub fn break_undo_group(&mut self) {
        self.buffer.break_undo_group();
    }

    /// Undo the last step. This view gets the selection from before the step;
    /// other views move across the reverse change. Returns the range of the
    /// restored text.
    pub fn undo(&mut self) -> Option<Range<usize>> {
        let entry = self.buffer.history_mut().pop_undo()?;
        let reverse = entry.changes.iter().rev().map(ChangeSet::inverse).collect();
        let extent = self.replay(reverse)?;
        *self.selection = self.buffer.clamp(Selection {
            preferred_column: None,
            ..entry.before
        });
        self.buffer.history_mut().push_redo(entry);
        Some(extent)
    }

    /// Redo the last undone step. This view gets the selection from after it.
    pub fn redo(&mut self) -> Option<Range<usize>> {
        let entry = self.buffer.history_mut().pop_redo()?;
        let forward = entry.changes.iter().map(ChangeSet::forward).collect();
        let extent = self.replay(forward)?;
        *self.selection = self.buffer.clamp(Selection {
            preferred_column: None,
            ..entry.after
        });
        self.buffer.history_mut().push_undo(entry);
        Some(extent)
    }

    /// Apply recorded edit batches in order, moving the peers, and return the
    /// range they cover in the final text.
    fn replay(&mut self, batches: Vec<Vec<Edit>>) -> Option<Range<usize>> {
        let mut extent: Option<Range<usize>> = None;
        for edits in batches {
            let Ok(changes) = self.buffer.apply_edits(edits) else {
                // The history no longer describes this text; drop it rather
                // than apply edits at the wrong place.
                *self.buffer.history_mut() = Default::default();
                return None;
            };
            self.move_peers(&changes);
            let mapped = extent.map(|range| {
                changes.map(range.start, Assoc::Before)..changes.map(range.end, Assoc::After)
            });
            extent = match (mapped, changes.new_extent()) {
                (Some(a), Some(b)) => Some(a.start.min(b.start)..a.end.max(b.end)),
                (a, b) => a.or(b),
            };
        }
        Some(extent.unwrap_or(self.selection.cursor..self.selection.cursor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caret_after(changes: &ChangeSet, _: &TextBuffer) -> Selection {
        Selection::caret(changes.new_extent().map_or(0, |range| range.end))
    }

    #[test]
    fn edits_move_other_views_and_undo_reverses_exactly() {
        let mut buffer = TextBuffer::new("abc abc");
        let mut mine = Selection::caret(3);
        let mut theirs = Selection::spanning(4, 7);
        let mut ed = EditorMut::new(&mut buffer, &mut mine, vec![&mut theirs]);
        ed.replace_selection(" abc", EditKind::Other).unwrap();
        drop(ed);
        assert_eq!(buffer.text(), "abc abc abc");
        assert_eq!((mine.cursor, theirs.range()), (7, Some(8..11)));

        // Undo from the other view: the removed " abc" is identical to its
        // neighbour, yet the reverse change is exact.
        let mut ed = EditorMut::new(&mut buffer, &mut theirs, vec![&mut mine]);
        assert_eq!(ed.undo(), Some(3..3));
        drop(ed);
        assert_eq!(buffer.text(), "abc abc");
        assert_eq!((theirs.cursor, mine.cursor), (3, 3));
    }

    #[test]
    fn explicit_groups_join_edits_and_nest() {
        let mut buffer = TextBuffer::new("");
        let mut selection = Selection::default();
        let mut ed = EditorMut::new(&mut buffer, &mut selection, Vec::new());
        ed.begin_group();
        ed.replace_selection("a", EditKind::Other).unwrap();
        ed.begin_group();
        ed.set_cursor(0);
        ed.replace_selection("b", EditKind::Other).unwrap();
        ed.end_group();
        ed.break_undo_group();
        ed.replace_selection("c", EditKind::Typing).unwrap();
        ed.end_group();
        ed.replace_selection("d", EditKind::Typing).unwrap();
        assert_eq!(ed.text(), "bcda");
        assert_eq!(ed.undo(), Some(2..2));
        assert_eq!(ed.text(), "bca");
        assert_eq!(ed.undo(), Some(0..0));
        assert_eq!(ed.text(), "");
        assert_eq!(ed.redo(), Some(0..3));
        assert_eq!((ed.text(), ed.cursor()), ("bca", 2));
    }

    #[test]
    fn batch_edits_validate_first_and_produce_one_undo_step() {
        let mut buffer = TextBuffer::new("a\nb\nc");
        let mut selection = Selection::caret(4);
        let mut ed = EditorMut::new(&mut buffer, &mut selection, Vec::new());
        let bad = ed.edit(
            EditKind::Other,
            vec![Edit::insert(0, "> "), Edit::insert(0, "> ")],
            caret_after,
        );
        assert!(bad.is_err());
        assert!(!ed.can_undo());
        ed.edit(
            EditKind::Other,
            vec![
                Edit::insert(0, "> "),
                Edit::insert(2, "> "),
                Edit::insert(4, "> "),
            ],
            |changes, _| Selection::caret(changes.map(4, Assoc::After)),
        )
        .unwrap();
        assert_eq!((ed.text(), ed.cursor()), ("> a\n> b\n> c", 10));
        assert_eq!(ed.undo(), Some(0..4));
        assert_eq!((ed.text(), ed.cursor()), ("a\nb\nc", 4));
    }

    #[test]
    fn typing_runs_join_until_the_selection_moves() {
        let mut buffer = TextBuffer::new("");
        let mut selection = Selection::default();
        let mut ed = EditorMut::new(&mut buffer, &mut selection, Vec::new());
        for typed in ["a", "b"] {
            ed.replace_selection(typed, EditKind::Typing).unwrap();
        }
        ed.set_cursor(1);
        ed.replace_selection("x", EditKind::Typing).unwrap();
        ed.undo();
        assert_eq!(ed.text(), "ab");
        ed.undo();
        assert_eq!(ed.text(), "");
    }
}
