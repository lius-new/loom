//! Unicode-aware text editing, selection and per-document undo history.
use std::{
    ops::Range,
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
#[derive(Clone, Debug)]
struct Snapshot {
    text: Arc<str>,
    cursor: usize,
    anchor: Option<usize>,
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
    cursor: usize,
    anchor: Option<usize>,
    preferred_column: Option<usize>,
    undo: Arc<Vec<Snapshot>>,
    redo: Arc<Vec<Snapshot>>,
    last_edit: Option<(EditKind, Instant)>,
}
impl TextBuffer {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            cursor: 0,
            anchor: None,
            preferred_column: None,
            undo: Arc::new(Vec::new()),
            redo: Arc::new(Vec::new()),
            last_edit: None,
        }
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    fn boundary(&self, pos: usize) -> usize {
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
    pub fn set_cursor(&mut self, pos: usize) {
        self.select_to(pos, false);
    }
    pub fn select_to(&mut self, pos: usize, extend: bool) {
        self.break_undo_group();
        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = self.boundary(pos);
        self.preferred_column = None;
    }
    pub fn selection(&self) -> Option<Range<usize>> {
        self.anchor
            .filter(|&a| a != self.cursor)
            .map(|a| a.min(self.cursor)..a.max(self.cursor))
    }
    pub fn selected_text(&self) -> Option<&str> {
        self.selection().map(|r| &self.text[r])
    }
    pub fn select_all(&mut self) {
        self.select_to(0, false);
        self.select_to(self.text.len(), true);
    }
    pub fn clear_selection(&mut self) {
        self.anchor = None;
        self.break_undo_group();
    }
    pub fn select_range(&mut self, range: Range<usize>) {
        self.set_cursor(range.start);
        self.select_to(range.end, true);
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
    pub fn break_undo_group(&mut self) {
        self.last_edit = None;
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: Arc::from(self.text.as_str()),
            cursor: self.cursor,
            anchor: self.anchor,
        }
    }
    fn restore(&mut self, s: Snapshot) {
        self.text = s.text.to_string();
        self.cursor = s.cursor;
        self.anchor = s.anchor;
        self.preferred_column = None;
        self.break_undo_group();
    }
    fn record(&mut self, kind: EditKind) {
        let now = Instant::now();
        let merge = kind != EditKind::Atomic
            && self.selection().is_none()
            && self
                .last_edit
                .is_some_and(|(k, t)| k == kind && now.duration_since(t) < Duration::from_secs(1));
        if !merge {
            let s = self.snapshot();
            let h = Arc::make_mut(&mut self.undo);
            if h.len() >= 200 {
                h.remove(0);
            }
            h.push(s);
        }
        Arc::make_mut(&mut self.redo).clear();
        self.last_edit = Some((kind, now));
        self.preferred_column = None;
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo(&mut self) {
        if let Some(s) = Arc::make_mut(&mut self.undo).pop() {
            let current = self.snapshot();
            Arc::make_mut(&mut self.redo).push(current);
            self.restore(s);
        }
    }
    pub fn redo(&mut self) {
        if let Some(s) = Arc::make_mut(&mut self.redo).pop() {
            let current = self.snapshot();
            Arc::make_mut(&mut self.undo).push(current);
            self.restore(s);
        }
    }
    fn replace(&mut self, range: Range<usize>, value: &str, kind: EditKind) {
        if range.is_empty() && value.is_empty() {
            return;
        }
        self.record(kind);
        self.cursor = range.start + value.len();
        self.text.replace_range(range, value);
        self.anchor = None;
        // Combining input can join the preceding grapheme; keep the caret at its end.
        if self.cursor < self.text.len() && self.boundary(self.cursor) != self.cursor {
            self.cursor = self.next(self.cursor);
        }
    }
    pub fn insert(&mut self, s: &str) {
        let kind = if s.contains(['\n', '\r', '\t']) {
            EditKind::Atomic
        } else {
            EditKind::Typing
        };
        self.replace(
            self.selection().unwrap_or(self.cursor..self.cursor),
            s,
            kind,
        );
    }
    pub fn paste(&mut self, s: &str) {
        let newline = self.newline();
        let normalized = s
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', newline);
        self.replace(
            self.selection().unwrap_or(self.cursor..self.cursor),
            &normalized,
            EditKind::Atomic,
        );
        self.break_undo_group();
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
    fn word_left(&self) -> usize {
        self.text[..self.cursor]
            .split_word_bound_indices()
            .rev()
            .find(|(_, s)| !s.chars().all(char::is_whitespace))
            .map_or(0, |(i, _)| i)
    }
    fn word_right(&self) -> usize {
        let rest = &self.text[self.cursor..];
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
        self.cursor + end
    }
    pub fn backspace(&mut self) {
        self.erase(false, false);
    }
    pub fn delete_forward(&mut self) {
        self.erase(true, false);
    }
    pub fn erase(&mut self, forward: bool, word: bool) {
        let range = self.selection().unwrap_or_else(|| {
            if forward {
                self.cursor..if word {
                    self.word_right()
                } else {
                    self.next(self.cursor)
                }
            } else {
                (if word {
                    self.word_left()
                } else {
                    self.previous(self.cursor)
                })..self.cursor
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
    pub fn newline(&self) -> &'static str {
        if self.text.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        }
    }
    pub fn enter(&mut self) {
        let start = self.selection().map_or(self.cursor, |r| r.start);
        let line_start = self.text[..start].rfind('\n').map_or(0, |i| i + 1);
        let indent: String = self.text[line_start..start]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        self.insert(&format!("{}{}", self.newline(), indent));
    }
    pub fn indent(&mut self, outdent: bool) {
        if !outdent && self.selection().is_none() {
            self.insert("  ");
            self.break_undo_group();
            return;
        }
        let range = self.selection().unwrap_or(self.cursor..self.cursor);
        let bounds = self.line_bounds();
        let starts: Vec<usize> = bounds
            .iter()
            .filter(|(a, b)| {
                *b >= range.start && (*a < range.end || range.is_empty() && *a <= range.start)
            })
            .map(|(a, _)| *a)
            .collect();
        let edits: Vec<(usize, usize)> = starts
            .into_iter()
            .filter_map(|a| {
                let remove = if self.text[a..].starts_with('\t') {
                    1
                } else {
                    self.text[a..]
                        .chars()
                        .take(2)
                        .take_while(|c| *c == ' ')
                        .count()
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
        self.record(EditKind::Atomic);
        for (a, remove) in edits.into_iter().rev() {
            let (deleted, inserted) = if outdent { (remove, 0) } else { (0, 2) };
            self.text
                .replace_range(a..a + deleted, if outdent { "" } else { "  " });
            let adjust = |p: usize| {
                if p < a {
                    p
                } else {
                    a + (p - a).saturating_sub(deleted) + inserted
                }
            };
            self.cursor = adjust(self.cursor);
            self.anchor = self.anchor.map(adjust);
        }
        self.break_undo_group();
    }
    pub fn navigate(&mut self, movement: Movement, extend: bool) {
        self.break_undo_group();
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
        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        let (line, _) = self.line_col();
        let bounds = self.line_bounds();
        let vertical = matches!(
            movement,
            Movement::Up | Movement::Down | Movement::PageUp(_) | Movement::PageDown(_)
        );
        let target = match movement {
            Movement::Left => self.previous(self.cursor),
            Movement::Right => self.next(self.cursor),
            Movement::WordLeft => self.word_left(),
            Movement::WordRight => self.word_right(),
            Movement::Home => bounds[line].0,
            Movement::End => bounds[line].1,
            Movement::Start => 0,
            Movement::Finish => self.text.len(),
            _ => {
                let col = self
                    .preferred_column
                    .unwrap_or_else(|| display_columns(&self.text[bounds[line].0..self.cursor]));
                self.preferred_column = Some(col);
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
                for (i, g) in self.text[a..b].grapheme_indices(true) {
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
        self.cursor = target;
        if !vertical {
            self.preferred_column = None;
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
    pub fn line_col(&self) -> (usize, usize) {
        let line = self.text[..self.cursor]
            .bytes()
            .filter(|b| *b == b'\n')
            .count();
        let start = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
        (line, self.text[start..self.cursor].chars().count())
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
    #[test]
    fn unicode_navigation_and_delete_are_grapheme_safe() {
        let mut b = TextBuffer::new("中e\u{301}👩‍💻\r\n尾");
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
        let mut b = TextBuffer::new("hello world");
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
        let mut b = TextBuffer::new("");
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
        let mut b = TextBuffer::new("abcdef\nx\nabcdef");
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
        let mut b = TextBuffer::new("  one\r\n  two");
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
        let mut b = TextBuffer::new("a\nb\nc");
        b.select_range(0..2);
        b.indent(false);
        assert_eq!(b.text(), "  a\nb\nc");
        b.undo();
        assert_eq!(b.selection(), Some(0..2));
    }
    #[test]
    fn words_deletion_and_document_boundaries() {
        let mut b = TextBuffer::new("hello  world!");
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
}
