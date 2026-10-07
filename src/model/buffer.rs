//! Unicode-aware text storage with per-document undo history, and the
//! editor views that move a selection through it.
//!
//! A `TextBuffer` is shared by every view of one document. Each view owns a
//! `Selection`; `EditorMut` pairs the two for one view and keeps the other
//! views' selections valid as the text changes.
use std::{
    ops::{Deref, Range},
    sync::Arc,
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Movement {
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    Home,
    End,
    Start,
    Finish,
    PageUp(usize),
    PageDown(usize),
}

/// Caret and optional anchor of one view, as byte offsets into the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
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

    pub fn range(&self) -> Option<Range<usize>> {
        self.anchor
            .filter(|&a| a != self.cursor)
            .map(|a| a.min(self.cursor)..a.max(self.cursor))
    }

    /// Follow a text replacement made through another view. Offsets after the
    /// edit shift by its length change; offsets inside it collapse to its start.
    pub fn transform(&mut self, edit: TextEdit) {
        self.cursor = edit.map(self.cursor);
        self.anchor = self.anchor.map(|anchor| edit.map(anchor));
        self.preferred_column = None;
    }
}

/// One replacement of `range` (in the old text) by `inserted_len` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub range: Range<usize>,
    pub inserted_len: usize,
}

impl TextEdit {
    fn map(&self, offset: usize) -> usize {
        if offset <= self.range.start {
            offset
        } else if offset >= self.range.end {
            offset - self.range.len() + self.inserted_len
        } else {
            self.range.start
        }
    }

    /// The smallest single edit turning `old` into `new`.
    fn between(old: &str, new: &str) -> Option<Self> {
        if old == new {
            return None;
        }
        let mut prefix = old
            .bytes()
            .zip(new.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
            prefix -= 1;
        }
        let max_suffix = old.len().min(new.len()) - prefix;
        let mut suffix = old
            .bytes()
            .rev()
            .zip(new.bytes().rev())
            .take(max_suffix)
            .take_while(|(a, b)| a == b)
            .count();
        while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix)
        {
            suffix -= 1;
        }
        Some(Self {
            range: prefix..old.len() - suffix,
            inserted_len: new.len() - suffix - prefix,
        })
    }
}

#[derive(Clone, Debug)]
struct Snapshot {
    text: Arc<str>,
    selection: Selection,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditKind {
    Typing,
    Backspace,
    Delete,
    Atomic,
}
#[derive(Clone, Debug)]
pub struct TextBuffer {
    text: String,
    undo: Arc<Vec<Snapshot>>,
    redo: Arc<Vec<Snapshot>>,
    /// Kind, time and resulting caret of the last edit; a following edit of
    /// the same kind at that caret joins its undo group.
    last_edit: Option<(EditKind, Instant, usize)>,
}
impl TextBuffer {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            undo: Arc::new(Vec::new()),
            redo: Arc::new(Vec::new()),
            last_edit: None,
        }
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn break_undo_group(&mut self) {
        self.last_edit = None;
    }
    /// The nearest grapheme boundary at or before `pos`.
    pub fn boundary(&self, pos: usize) -> usize {
        let pos = pos.min(self.text.len());
        if pos == self.text.len() {
            return pos;
        }
        self.text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .take_while(|&i| i <= pos)
            .last()
            .unwrap_or(0)
    }
    /// Clamp a selection that may predate a text change onto valid boundaries.
    pub fn clamp(&self, selection: Selection) -> Selection {
        Selection {
            cursor: self.boundary(selection.cursor),
            anchor: selection.anchor.map(|anchor| self.boundary(anchor)),
            preferred_column: selection.preferred_column,
        }
    }
    pub fn word_range(&self, pos: usize) -> Range<usize> {
        let pos = self.boundary(pos);
        self.text
            .split_word_bound_indices()
            .find(|(i, s)| *i <= pos && pos < *i + s.len())
            .map(|(i, s)| i..i + s.len())
            .unwrap_or(pos..pos)
    }
    pub fn line_range(&self, line: usize) -> Range<usize> {
        let b = self.line_bounds();
        let line = line.min(b.len() - 1);
        b[line].0..b.get(line + 1).map_or(self.text.len(), |v| v.0)
    }
    pub fn newline(&self) -> &'static str {
        if self.text.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        }
    }
    /// Zero-based line and character column of a byte offset.
    pub fn line_col_at(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.text.len());
        let line = self.text[..offset].bytes().filter(|b| *b == b'\n').count();
        let start = self.text[..offset].rfind('\n').map_or(0, |i| i + 1);
        (line, self.text[start..offset].chars().count())
    }
    pub fn line_count(&self) -> usize {
        self.line_bounds().len()
    }
    pub fn line_bounds(&self) -> Vec<(usize, usize)> {
        let mut lines = Vec::new();
        let mut start = 0;
        for (i, c) in self.text.char_indices() {
            if c == '\n' {
                let end = if i > start && self.text.as_bytes()[i - 1] == b'\r' {
                    i - 1
                } else {
                    i
                };
                lines.push((start, end));
                start = i + 1;
            }
        }
        lines.push((start, self.text.len()));
        lines
    }
    pub fn line(&self, idx: usize) -> &str {
        let b = self.line_bounds();
        let (a, z) = b[idx.min(b.len() - 1)];
        &self.text[a..z]
    }
    /// Replace the whole text with a version loaded from disk. History is
    /// reset because it describes a different file revision.
    pub fn reload(&mut self, contents: String) {
        *self = Self::new(contents);
    }

    fn previous(&self, p: usize) -> usize {
        self.text[..p]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i)
    }
    fn next(&self, p: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|&i| i > p)
            .unwrap_or(self.text.len())
    }
    fn word_left(&self, from: usize) -> usize {
        self.text[..from]
            .split_word_bound_indices()
            .rev()
            .find(|(_, s)| !s.chars().all(char::is_whitespace))
            .map_or(0, |(i, _)| i)
    }
    fn word_right(&self, from: usize) -> usize {
        let rest = &self.text[from..];
        let mut end = 0;
        let mut found = false;
        let leading_space = rest.chars().next().is_some_and(char::is_whitespace);
        for (i, s) in rest.split_word_bound_indices() {
            let space = s.chars().all(char::is_whitespace);
            if (found || leading_space) && !space {
                break;
            }
            end = i + s.len();
            if !space {
                found = true;
            }
        }
        from + end
    }
    /// Push an undo snapshot unless this edit continues the current group.
    fn record(&mut self, kind: EditKind, before: Selection) {
        let now = Instant::now();
        let merge = kind != EditKind::Atomic
            && before.range().is_none()
            && self.last_edit.is_some_and(|(k, t, caret)| {
                k == kind
                    && caret == before.cursor
                    && now.duration_since(t) < Duration::from_secs(1)
            });
        if !merge {
            let s = Snapshot {
                text: Arc::from(self.text.as_str()),
                selection: before,
            };
            let h = Arc::make_mut(&mut self.undo);
            if h.len() >= 200 {
                h.remove(0);
            }
            h.push(s);
        }
        Arc::make_mut(&mut self.redo).clear();
        self.last_edit = Some((kind, now, 0));
    }
    fn finish_record(&mut self, caret: usize) {
        if let Some((_, _, last)) = &mut self.last_edit {
            *last = caret;
        }
    }
}
/// Read-only view of a buffer through one selection.
#[derive(Clone, Copy)]
pub struct Editor<'a> {
    buffer: &'a TextBuffer,
    selection: Selection,
}

/// Editable view of a buffer through one selection. Every text change is
/// replayed onto `peers`, the selections of the document's other views.
pub struct EditorMut<'a> {
    buffer: &'a mut TextBuffer,
    selection: &'a mut Selection,
    peers: Vec<&'a mut Selection>,
}

impl Deref for Editor<'_> {
    type Target = TextBuffer;
    fn deref(&self) -> &TextBuffer {
        self.buffer
    }
}
impl Deref for EditorMut<'_> {
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
        self.selection().map(|r| &buffer.text()[r])
    }
    pub fn line_col(&self) -> (usize, usize) {
        self.buffer.line_col_at(self.selection.cursor)
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
    pub fn cursor(&self) -> usize {
        self.selection.cursor
    }
    pub fn selection(&self) -> Option<Range<usize>> {
        self.selection.range()
    }
    pub fn selected_text(&self) -> Option<&str> {
        self.selection().map(|r| &self.buffer.text()[r])
    }
    pub fn line_col(&self) -> (usize, usize) {
        self.buffer.line_col_at(self.selection.cursor)
    }
    pub fn break_undo_group(&mut self) {
        self.buffer.break_undo_group();
    }

    pub fn set_cursor(&mut self, pos: usize) {
        self.select_to(pos, false);
    }
    pub fn select_to(&mut self, pos: usize, extend: bool) {
        self.buffer.break_undo_group();
        if extend {
            self.selection.anchor.get_or_insert(self.selection.cursor);
        } else {
            self.selection.anchor = None;
        }
        self.selection.cursor = self.buffer.boundary(pos);
        self.selection.preferred_column = None;
    }
    pub fn select_all(&mut self) {
        self.select_to(0, false);
        let end = self.buffer.text().len();
        self.select_to(end, true);
    }
    pub fn clear_selection(&mut self) {
        self.selection.anchor = None;
        self.buffer.break_undo_group();
    }
    pub fn select_range(&mut self, range: Range<usize>) {
        self.set_cursor(range.start);
        self.select_to(range.end, true);
    }

    fn propagate(&mut self, edit: TextEdit) {
        for peer in &mut self.peers {
            peer.transform(edit.clone());
            **peer = self.buffer.clamp(**peer);
        }
    }
    /// Restore a history snapshot into this view and move the peers across the change.
    fn restore(&mut self, s: Snapshot) {
        let edit = TextEdit::between(&self.buffer.text, &s.text);
        self.buffer.text = s.text.to_string();
        *self.selection = s.selection;
        self.selection.preferred_column = None;
        self.buffer.break_undo_group();
        if let Some(edit) = edit {
            self.propagate(edit);
        }
    }
    pub fn undo(&mut self) {
        if let Some(s) = Arc::make_mut(&mut self.buffer.undo).pop() {
            let current = Snapshot {
                text: Arc::from(self.buffer.text.as_str()),
                selection: *self.selection,
            };
            Arc::make_mut(&mut self.buffer.redo).push(current);
            self.restore(s);
        }
    }
    pub fn redo(&mut self) {
        if let Some(s) = Arc::make_mut(&mut self.buffer.redo).pop() {
            let current = Snapshot {
                text: Arc::from(self.buffer.text.as_str()),
                selection: *self.selection,
            };
            Arc::make_mut(&mut self.buffer.undo).push(current);
            self.restore(s);
        }
    }

    fn replace(&mut self, range: Range<usize>, value: &str, kind: EditKind) {
        if range.is_empty() && value.is_empty() {
            return;
        }
        self.buffer.record(kind, *self.selection);
        let edit = TextEdit {
            range: range.clone(),
            inserted_len: value.len(),
        };
        let mut cursor = range.start + value.len();
        self.buffer.text.replace_range(range, value);
        // Combining input can join the preceding grapheme; keep the caret at its end.
        if cursor < self.buffer.text.len() && self.buffer.boundary(cursor) != cursor {
            cursor = self.buffer.next(cursor);
        }
        *self.selection = Selection::caret(cursor);
        self.buffer.finish_record(cursor);
        self.propagate(edit);
    }
    fn selection_or_caret(&self) -> Range<usize> {
        self.selection()
            .unwrap_or(self.selection.cursor..self.selection.cursor)
    }
    pub fn insert(&mut self, s: &str) {
        let kind = if s.contains(['\n', '\r', '\t']) {
            EditKind::Atomic
        } else {
            EditKind::Typing
        };
        self.replace(self.selection_or_caret(), s, kind);
    }
    pub fn paste(&mut self, s: &str) {
        let newline = self.buffer.newline();
        let normalized = s
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', newline);
        self.replace(self.selection_or_caret(), &normalized, EditKind::Atomic);
        self.buffer.break_undo_group();
    }
    pub fn backspace(&mut self) {
        self.erase(false, false);
    }
    pub fn delete_forward(&mut self) {
        self.erase(true, false);
    }
    pub fn erase(&mut self, forward: bool, word: bool) {
        let cursor = self.selection.cursor;
        let range = self.selection().unwrap_or_else(|| {
            if forward {
                cursor..if word {
                    self.buffer.word_right(cursor)
                } else {
                    self.buffer.next(cursor)
                }
            } else {
                (if word {
                    self.buffer.word_left(cursor)
                } else {
                    self.buffer.previous(cursor)
                })..cursor
            }
        });
        self.replace(
            range,
            "",
            if word {
                EditKind::Atomic
            } else if forward {
                EditKind::Delete
            } else {
                EditKind::Backspace
            },
        );
    }
    pub fn enter(&mut self) {
        let text = self.buffer.text();
        let start = self.selection().map_or(self.selection.cursor, |r| r.start);
        let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
        let indent: String = text[line_start..start]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let newline = self.buffer.newline();
        self.insert(&format!("{newline}{indent}"));
    }
    pub fn indent(&mut self, outdent: bool) {
        if !outdent && self.selection().is_none() {
            self.insert("  ");
            self.buffer.break_undo_group();
            return;
        }
        let range = self.selection_or_caret();
        let bounds = self.buffer.line_bounds();
        let text = self.buffer.text();
        let edits: Vec<(usize, usize)> = bounds
            .iter()
            .filter(|(a, b)| {
                *b >= range.start && (*a < range.end || range.is_empty() && *a <= range.start)
            })
            .filter_map(|&(a, _)| {
                let remove = if text[a..].starts_with('\t') {
                    1
                } else {
                    text[a..].chars().take(2).take_while(|c| *c == ' ').count()
                };
                if outdent && remove == 0 {
                    None
                } else {
                    Some((a, remove))
                }
            })
            .collect();
        if edits.is_empty() {
            return;
        }
        self.buffer.record(EditKind::Atomic, *self.selection);
        for (a, remove) in edits.into_iter().rev() {
            let (deleted, inserted) = if outdent { (remove, 0) } else { (0, 2) };
            self.buffer
                .text
                .replace_range(a..a + deleted, if outdent { "" } else { "  " });
            let adjust = |p: usize| {
                if p < a {
                    p
                } else {
                    a + (p - a).saturating_sub(deleted) + inserted
                }
            };
            self.selection.cursor = adjust(self.selection.cursor);
            self.selection.anchor = self.selection.anchor.map(adjust);
            self.propagate(TextEdit {
                range: a..a + deleted,
                inserted_len: inserted,
            });
        }
        self.selection.preferred_column = None;
        self.buffer.break_undo_group();
    }
    pub fn navigate(&mut self, movement: Movement, extend: bool) {
        self.buffer.break_undo_group();
        if !extend
            && matches!(movement, Movement::Left | Movement::Right)
            && let Some(r) = self.selection()
        {
            self.set_cursor(if movement == Movement::Left {
                r.start
            } else {
                r.end
            });
            return;
        }
        let cursor = self.selection.cursor;
        if extend {
            self.selection.anchor.get_or_insert(cursor);
        } else {
            self.selection.anchor = None;
        }
        let buffer = &*self.buffer;
        let text = buffer.text();
        let (line, _) = buffer.line_col_at(cursor);
        let bounds = buffer.line_bounds();
        let vertical = matches!(
            movement,
            Movement::Up | Movement::Down | Movement::PageUp(_) | Movement::PageDown(_)
        );
        let target = match movement {
            Movement::Left => buffer.previous(cursor),
            Movement::Right => buffer.next(cursor),
            Movement::WordLeft => buffer.word_left(cursor),
            Movement::WordRight => buffer.word_right(cursor),
            Movement::Home => bounds[line].0,
            Movement::End => bounds[line].1,
            Movement::Start => 0,
            Movement::Finish => text.len(),
            _ => {
                let col = self
                    .selection
                    .preferred_column
                    .unwrap_or_else(|| display_columns(&text[bounds[line].0..cursor]));
                self.selection.preferred_column = Some(col);
                let target = match movement {
                    Movement::Up => line.saturating_sub(1),
                    Movement::Down => (line + 1).min(bounds.len() - 1),
                    Movement::PageUp(n) => line.saturating_sub(n),
                    Movement::PageDown(n) => (line + n).min(bounds.len() - 1),
                    _ => line,
                };
                let (a, b) = bounds[target];
                let mut x = 0;
                let mut offset = a;
                for (i, g) in text[a..b].grapheme_indices(true) {
                    let width = if g == "\t" {
                        4 - x % 4
                    } else {
                        UnicodeWidthStr::width(g)
                    };
                    if x + width > col {
                        break;
                    }
                    x += width;
                    offset = a + i + g.len();
                }
                offset
            }
        };
        self.selection.cursor = target;
        if !vertical {
            self.selection.preferred_column = None;
        }
    }
    pub fn move_left(&mut self) {
        self.navigate(Movement::Left, false)
    }
    pub fn move_right(&mut self) {
        self.navigate(Movement::Right, false)
    }
    pub fn move_up(&mut self) {
        self.navigate(Movement::Up, false)
    }
    pub fn move_down(&mut self) {
        self.navigate(Movement::Down, false)
    }
    pub fn move_home(&mut self) {
        self.navigate(Movement::Home, false)
    }
    pub fn move_end(&mut self) {
        self.navigate(Movement::End, false)
    }
}
pub fn display_columns(s: &str) -> usize {
    s.graphemes(true).fold(0, |col, g| {
        col + if g == "\t" {
            4 - col % 4
        } else {
            UnicodeWidthStr::width(g)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A buffer with one view, the shape most tests need.
    struct Single {
        buffer: TextBuffer,
        selection: Selection,
    }
    impl Single {
        fn new(text: &str) -> Self {
            Self {
                buffer: TextBuffer::new(text),
                selection: Selection::default(),
            }
        }
        fn ed(&mut self) -> EditorMut<'_> {
            EditorMut::new(&mut self.buffer, &mut self.selection, Vec::new())
        }
    }

    #[test]
    fn unicode_navigation_and_delete_are_grapheme_safe() {
        let mut s = Single::new("中e\u{301}👩‍💻\r\n尾");
        let mut b = s.ed();
        for expected in [3, 6, 17, 19, 22] {
            b.move_right();
            assert_eq!(b.cursor(), expected);
        }
        b.backspace();
        b.backspace();
        assert_eq!(b.text(), "中e\u{301}👩‍💻");
        b.backspace();
        assert_eq!(b.text(), "中e\u{301}");
        b.set_cursor(4);
        assert_eq!(b.cursor(), 3);
    }
    #[test]
    fn selection_replace_and_history_restore_both_endpoints() {
        let mut s = Single::new("hello world");
        let mut b = s.ed();
        b.select_range(0..5);
        b.insert("你好");
        assert_eq!(b.text(), "你好 world");
        b.undo();
        assert_eq!(b.selected_text(), Some("hello"));
        b.redo();
        assert_eq!(b.text(), "你好 world");
        b.undo();
        b.insert("other");
        assert!(!b.can_redo());
    }
    #[test]
    fn typing_groups_but_navigation_and_paste_break_groups() {
        let mut s = Single::new("");
        let mut b = s.ed();
        b.insert("a");
        b.insert("b");
        b.insert("c");
        b.undo();
        assert_eq!(b.text(), "");
        b.redo();
        b.move_left();
        b.insert("x");
        b.paste("yz");
        b.undo();
        assert_eq!(b.text(), "abxc");
        b.undo();
        assert_eq!(b.text(), "abc");
    }
    #[test]
    fn vertical_navigation_keeps_visual_column_and_selection_anchor() {
        let mut s = Single::new("abcdef\nx\nabcdef");
        let mut b = s.ed();
        b.set_cursor(5);
        b.navigate(Movement::Down, true);
        b.navigate(Movement::Down, true);
        assert_eq!(b.line_col(), (2, 5));
        assert_eq!(b.selection(), Some(5..14));
        b.move_left();
        assert_eq!(b.cursor(), 5);
    }
    #[test]
    fn crlf_enter_paste_and_indent_preserve_document_conventions() {
        let mut s = Single::new("  one\r\n  two");
        let mut b = s.ed();
        b.set_cursor(5);
        b.enter();
        assert_eq!(b.text(), "  one\r\n  \r\n  two");
        b.undo();
        b.select_all();
        b.indent(false);
        assert_eq!(b.text(), "    one\r\n    two");
        b.indent(true);
        assert_eq!(b.text(), "  one\r\n  two");
        b.select_all();
        b.paste("a\nb\r\nc");
        assert_eq!(b.text(), "a\r\nb\r\nc");
    }
    #[test]
    fn indent_excludes_line_at_selection_end() {
        let mut s = Single::new("a\nb\nc");
        let mut b = s.ed();
        b.select_range(0..2);
        b.indent(false);
        assert_eq!(b.text(), "  a\nb\nc");
        b.undo();
        assert_eq!(b.selection(), Some(0..2));
    }
    #[test]
    fn words_deletion_and_document_boundaries() {
        let mut s = Single::new("hello  world!");
        let mut b = s.ed();
        b.navigate(Movement::WordRight, false);
        assert_eq!(b.cursor(), 7);
        b.erase(true, true);
        assert_eq!(b.text(), "hello  !");
        b.erase(false, true);
        assert_eq!(b.text(), "!");
        b.navigate(Movement::Start, true);
        b.backspace();
        b.navigate(Movement::Finish, false);
        b.delete_forward();
        assert_eq!(b.text(), "!");
    }

    #[test]
    fn transform_shifts_offsets_after_and_collapses_offsets_inside_an_edit() {
        let edit = TextEdit {
            range: 4..8,
            inserted_len: 1,
        };
        let mut before = Selection::caret(4);
        before.transform(edit.clone());
        assert_eq!(before.cursor, 4);
        let mut after = Selection {
            cursor: 10,
            anchor: Some(8),
            preferred_column: Some(3),
        };
        after.transform(edit.clone());
        assert_eq!((after.cursor, after.anchor), (7, Some(5)));
        assert_eq!(after.preferred_column, None);
        let mut spanning = Selection {
            cursor: 6,
            anchor: Some(2),
            preferred_column: None,
        };
        spanning.transform(edit);
        assert_eq!(spanning.range(), Some(2..4));
    }

    #[test]
    fn edits_in_one_view_move_the_other_views() {
        let mut buffer = TextBuffer::new("one two three");
        let mut mine = Selection::caret(4);
        let mut theirs = Selection {
            cursor: 13,
            anchor: Some(8),
            preferred_column: None,
        };
        let mut ed = EditorMut::new(&mut buffer, &mut mine, vec![&mut theirs]);
        ed.insert("big ");
        assert_eq!(ed.text(), "one big two three");
        drop(ed);
        assert_eq!(theirs.range(), Some(12..17));
        assert_eq!(mine.cursor, 8);

        let mut ed = EditorMut::new(&mut buffer, &mut mine, vec![&mut theirs]);
        ed.select_all();
        ed.indent(false);
        drop(ed);
        assert_eq!(buffer.text(), "  one big two three");
        assert_eq!(theirs.range(), Some(14..19));
    }

    #[test]
    fn undo_restores_the_undoing_view_and_moves_peers_across_the_change() {
        let mut buffer = TextBuffer::new("abc xyz");
        let mut mine = Selection::caret(3);
        let mut theirs = Selection::caret(7);
        EditorMut::new(&mut buffer, &mut mine, vec![&mut theirs]).insert("123");
        assert_eq!(theirs.cursor, 10);
        EditorMut::new(&mut buffer, &mut mine, vec![&mut theirs]).undo();
        assert_eq!(buffer.text(), "abc xyz");
        assert_eq!(mine.cursor, 3);
        assert_eq!(theirs.cursor, 7);
    }

    #[test]
    fn typing_in_another_view_starts_a_new_undo_group() {
        let mut buffer = TextBuffer::new("ab");
        let mut left = Selection::caret(0);
        let mut right = Selection::caret(2);
        EditorMut::new(&mut buffer, &mut left, vec![&mut right]).insert("x");
        EditorMut::new(&mut buffer, &mut right, vec![&mut left]).insert("y");
        assert_eq!(buffer.text(), "xaby");
        EditorMut::new(&mut buffer, &mut right, vec![&mut left]).undo();
        assert_eq!(buffer.text(), "xab");
    }

    #[test]
    fn minimal_edit_respects_char_boundaries() {
        let edit = TextEdit::between("a中b", "a文b").unwrap();
        assert_eq!(edit.range, 1..4);
        assert_eq!(edit.inserted_len, 3);
        assert_eq!(TextEdit::between("same", "same"), None);
        let edit = TextEdit::between("aaa", "aaaa").unwrap();
        assert_eq!((edit.range, edit.inserted_len), (3..3, 1));
    }
}
