//! The text of one document, the queries editing behaviours build on, and
//! the one place where the text changes.

use std::ops::Range;
use std::sync::Arc;

use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

use super::change::{Change, ChangeSet, Edit, EditError, validate};
use super::history::History;
use super::lines::{LineIndex, display_width, grapheme_width};
use super::selection::Selection;

/// The line break ending the first line, if it has one.
fn first_line_ending(text: &str, lines: &LineIndex) -> Option<&'static str> {
    let next = lines.next_start(0)?;
    Some(if next >= 2 && text.as_bytes()[next - 2] == b'\r' {
        "\r\n"
    } else {
        "\n"
    })
}

/// Apply validated, sorted edits to `text` in one pass.
pub(super) fn apply(text: &mut String, edits: Vec<Edit>) -> ChangeSet {
    let mut changes = Vec::with_capacity(edits.len());
    if let [single] = edits.as_slice() {
        let range = single.range.clone();
        let removed = text[range.clone()].to_owned();
        text.replace_range(range.clone(), &single.text);
        changes.push(Change {
            new_start: range.start,
            old: range,
            removed,
            inserted: single.text.clone(),
        });
        return ChangeSet::new(changes);
    }
    let grown: usize = edits.iter().map(|edit| edit.text.len()).sum();
    let mut out = String::with_capacity(text.len() + grown);
    let mut copied = 0;
    for edit in edits {
        out.push_str(&text[copied..edit.range.start]);
        let new_start = out.len();
        out.push_str(&edit.text);
        changes.push(Change {
            removed: text[edit.range.clone()].to_owned(),
            old: edit.range.clone(),
            new_start,
            inserted: edit.text,
        });
        copied = edit.range.end;
    }
    out.push_str(&text[copied..]);
    *text = out;
    ChangeSet::new(changes)
}

/// The text of one document with its line index and undo history.
///
/// Cloning is cheap: the text, lines and history are shared until the next
/// change, which copies them only while an older clone is still alive.
#[derive(Clone, Debug)]
pub struct TextBuffer {
    text: Arc<String>,
    lines: Arc<LineIndex>,
    history: Arc<History>,
    version: u64,
    /// The line break of the first line, kept for when no line break is left.
    ending: &'static str,
}

impl TextBuffer {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let lines = LineIndex::new(&text);
        let ending = first_line_ending(&text, &lines).unwrap_or("\n");
        Self {
            lines: Arc::new(lines),
            text: Arc::new(text),
            history: Arc::default(),
            version: 0,
            ending,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Increases with every change to the text.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Replace the whole text with a version loaded from disk. History is
    /// reset because it describes a different file revision.
    pub fn reload(&mut self, contents: String) {
        let version = self.version + 1;
        *self = Self::new(contents);
        self.version = version;
    }

    pub fn install_prepared(&mut self, mut prepared: Self) {
        prepared.version = self.version + 1;
        *self = prepared;
    }

    /// Validate and apply `edits`, all given in the current text's offsets.
    /// Nothing changes when any edit is invalid.
    pub(super) fn apply_edits(&mut self, edits: Vec<Edit>) -> Result<ChangeSet, EditError> {
        let edits = validate(&self.text, edits)?;
        if edits.is_empty() {
            return Ok(ChangeSet::default());
        }
        let changes = apply(Arc::make_mut(&mut self.text), edits);
        Arc::make_mut(&mut self.lines).update(&self.text, &changes);
        self.version += 1;
        if let Some(ending) = first_line_ending(&self.text, &self.lines) {
            self.ending = ending;
        }
        Ok(changes)
    }

    pub(super) fn history(&self) -> &History {
        &self.history
    }

    pub(super) fn history_mut(&mut self) -> &mut History {
        Arc::make_mut(&mut self.history)
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Start a new undo step with the next edit, unless an explicit group
    /// is open.
    pub fn break_undo_group(&mut self) {
        if !self.history.in_group() {
            self.history_mut().break_typing();
        }
    }

    /// Bytes held by the undo and redo history.
    #[cfg(test)]
    pub fn history_bytes(&self) -> usize {
        self.history.bytes()
    }

    // ---- Characters ----------------------------------------------------------

    fn char_floor(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    pub fn is_grapheme_boundary(&self, offset: usize) -> bool {
        offset <= self.text.len()
            && self.text.is_char_boundary(offset)
            && GraphemeCursor::new(offset, self.text.len(), true)
                .is_boundary(&self.text, 0)
                .unwrap_or(true)
    }

    /// The nearest grapheme boundary at or before `offset`.
    pub fn grapheme_floor(&self, offset: usize) -> usize {
        let offset = self.char_floor(offset);
        if self.is_grapheme_boundary(offset) {
            return offset;
        }
        GraphemeCursor::new(offset, self.text.len(), true)
            .prev_boundary(&self.text, 0)
            .ok()
            .flatten()
            .unwrap_or(0)
    }

    /// The grapheme boundary after `offset`, or the end of the text.
    pub fn next_grapheme(&self, offset: usize) -> usize {
        let offset = self.grapheme_floor(offset);
        GraphemeCursor::new(offset, self.text.len(), true)
            .next_boundary(&self.text, 0)
            .ok()
            .flatten()
            .unwrap_or(self.text.len())
    }

    /// The grapheme boundary before `offset`, or the start of the text.
    pub fn prev_grapheme(&self, offset: usize) -> usize {
        let offset = self.char_floor(offset);
        GraphemeCursor::new(offset, self.text.len(), true)
            .prev_boundary(&self.text, 0)
            .ok()
            .flatten()
            .unwrap_or(0)
    }

    /// The grapheme starting at `offset`, if any.
    pub fn grapheme_at(&self, offset: usize) -> Option<&str> {
        let start = self.grapheme_floor(offset);
        (start < self.text.len()).then(|| &self.text[start..self.next_grapheme(start)])
    }

    /// Move both ends of a selection onto grapheme boundaries.
    pub fn clamp(&self, selection: Selection) -> Selection {
        Selection {
            cursor: self.grapheme_floor(selection.cursor),
            anchor: selection.anchor.map(|anchor| self.grapheme_floor(anchor)),
            preferred_column: selection.preferred_column,
        }
    }

    // ---- Lines ---------------------------------------------------------------

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// The line containing `offset`. A line break belongs to the line it ends.
    pub fn line_of(&self, offset: usize) -> usize {
        self.lines.line_of(offset.min(self.text.len()))
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.lines.start(self.last_line_at_most(line))
    }

    /// Where a line's content ends, before its line break.
    pub fn line_end(&self, line: usize) -> usize {
        self.lines
            .content_end(&self.text, self.last_line_at_most(line))
    }

    /// A line's content, without its line break.
    pub fn line_range(&self, line: usize) -> Range<usize> {
        self.line_start(line)..self.line_end(line)
    }

    /// A line including its line break.
    pub fn line_range_with_break(&self, line: usize) -> Range<usize> {
        let line = self.last_line_at_most(line);
        self.lines.start(line)..self.lines.next_start(line).unwrap_or(self.text.len())
    }

    /// A line's content, without its line break.
    pub fn line(&self, line: usize) -> &str {
        &self.text[self.line_range(line)]
    }

    pub fn lines(&self) -> impl Iterator<Item = &str> + '_ {
        (0..self.line_count()).map(|line| self.line(line))
    }

    fn last_line_at_most(&self, line: usize) -> usize {
        line.min(self.lines.len() - 1)
    }

    /// Zero-based line and character column of an offset.
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let offset = self.char_floor(offset);
        let line = self.line_of(offset);
        (
            line,
            self.text[self.lines.start(line)..offset].chars().count(),
        )
    }

    /// The offset of a character column, clamped to the line's content.
    pub fn offset_at_char_column(&self, line: usize, column: usize) -> usize {
        let range = self.line_range(line);
        self.text[range.clone()]
            .char_indices()
            .nth(column)
            .map_or(range.end, |(offset, _)| range.start + offset)
    }

    /// The line break the document uses, judged by its first line.
    pub fn line_ending(&self) -> &'static str {
        self.ending
    }

    // ---- Display columns -----------------------------------------------------

    /// Display column of `offset` within its line (tabs and wide characters
    /// expanded).
    pub fn display_column(&self, offset: usize) -> usize {
        let offset = self.char_floor(offset);
        display_width(&self.text[self.lines.start(self.line_of(offset))..offset])
    }

    /// The last grapheme boundary of `line` whose display column does not pass
    /// `column`, so a column inside a wide character or tab rounds down.
    pub fn offset_at_display_column(&self, line: usize, column: usize) -> usize {
        let range = self.line_range(line);
        let mut x = 0;
        let mut offset = range.start;
        for (index, grapheme) in self.text[range.clone()].grapheme_indices(true) {
            let width = grapheme_width(grapheme, x);
            if x + width > column {
                break;
            }
            x += width;
            offset = range.start + index + grapheme.len();
        }
        offset
    }

    /// The text of `line` covering display columns `left..right`. A wide
    /// character or tab that straddles either edge is included whole; a line
    /// that ends before `left` has no range.
    pub fn display_column_range(
        &self,
        line: usize,
        left: usize,
        right: usize,
    ) -> Option<Range<usize>> {
        let range = self.line_range(line);
        let mut x = 0;
        let mut start = None;
        let mut end = range.start;
        for (index, grapheme) in self.text[range.clone()].grapheme_indices(true) {
            if x >= right {
                break;
            }
            let width = grapheme_width(grapheme, x);
            if x + width.max(1) > left && start.is_none() {
                start = Some(range.start + index);
            }
            x += width;
            end = range.start + index + grapheme.len();
        }
        start.map(|start| start..end)
    }

    pub fn line_display_width(&self, line: usize) -> usize {
        self.lines.width(self.last_line_at_most(line))
    }

    pub fn max_line_display_width(&self) -> usize {
        self.lines.max_width()
    }

    // ---- Words (Unicode word boundaries) -----------------------------------

    /// The word-boundary segment containing `offset`.
    pub fn word_range(&self, offset: usize) -> Range<usize> {
        let offset = self.grapheme_floor(offset);
        let line = self.line_range_with_break(self.line_of(offset));
        self.text[line.clone()]
            .split_word_bound_indices()
            .map(|(index, segment)| line.start + index..line.start + index + segment.len())
            .find(|range| range.contains(&offset))
            .unwrap_or(offset..offset)
    }

    /// The start of the word before `from`, skipping whitespace.
    pub fn word_left(&self, from: usize) -> usize {
        self.text[..self.char_floor(from)]
            .split_word_bound_indices()
            .rev()
            .find(|(_, segment)| !segment.chars().all(char::is_whitespace))
            .map_or(0, |(index, _)| index)
    }

    /// The end of the word after `from`, skipping leading whitespace.
    pub fn word_right(&self, from: usize) -> usize {
        let from = self.char_floor(from);
        let rest = &self.text[from..];
        let mut end = 0;
        let mut found = false;
        let leading_space = rest.chars().next().is_some_and(char::is_whitespace);
        for (index, segment) in rest.split_word_bound_indices() {
            let space = segment.chars().all(char::is_whitespace);
            if (found || leading_space) && !space {
                break;
            }
            end = index + segment.len();
            if !space {
                found = true;
            }
        }
        from + end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grapheme_queries_step_over_clusters_and_crlf() {
        let buffer = TextBuffer::new("中e\u{301}👩‍💻\r\n尾");
        let mut offset = 0;
        let mut stops = Vec::new();
        while offset < buffer.len() {
            offset = buffer.next_grapheme(offset);
            stops.push(offset);
        }
        assert_eq!(stops, [3, 6, 17, 19, 22]);
        assert_eq!(buffer.prev_grapheme(19), 6 + 11);
        assert_eq!(buffer.prev_grapheme(17), 6);
        assert_eq!(buffer.grapheme_floor(4), 3);
        assert_eq!(buffer.grapheme_floor(18), 17, "between CR and LF");
        assert_eq!(buffer.grapheme_at(3), Some("e\u{301}"));
        assert_eq!(buffer.grapheme_at(22), None);
    }

    #[test]
    fn line_queries_separate_content_from_line_breaks() {
        let buffer = TextBuffer::new("a\tb\r\n中文\nend");
        assert_eq!(buffer.line_count(), 3);
        assert_eq!(buffer.line_range(0), 0..3);
        assert_eq!(buffer.line_range_with_break(0), 0..5);
        assert_eq!(buffer.line(1), "中文");
        assert_eq!(buffer.line_of(4), 0);
        assert_eq!(buffer.line_col(8), (1, 1));
        assert_eq!(buffer.offset_at_char_column(1, 9), 11);
        assert_eq!(buffer.line_ending(), "\r\n");
        assert_eq!(TextBuffer::new("a\nb\r\n").line_ending(), "\n");
        assert_eq!(buffer.display_column(2), 4);
        assert_eq!(buffer.offset_at_display_column(0, 3), 1, "inside the tab");
        assert_eq!(buffer.offset_at_display_column(1, 3), 8, "inside 文");
        assert_eq!(buffer.max_line_display_width(), 5);
        assert_eq!(buffer.line(7), "end", "lines clamp to the last one");
        assert_eq!(
            buffer.display_column_range(0, 1, 3),
            Some(1..2),
            "the tab spans 1..4"
        );
        assert_eq!(
            buffer.display_column_range(1, 1, 3),
            Some(5..11),
            "both wide characters"
        );
        assert_eq!(buffer.display_column_range(1, 5, 9), None);
        assert_eq!(buffer.display_column_range(2, 1, 99), Some(13..15));
    }

    #[test]
    fn invalid_edits_change_nothing_and_valid_batches_apply_once() {
        let mut buffer = TextBuffer::new("one two three");
        assert!(
            buffer
                .apply_edits(vec![Edit::insert(0, "x"), Edit::delete(2..20)])
                .is_err()
        );
        assert_eq!((buffer.text(), buffer.version()), ("one two three", 0));
        let changes = buffer
            .apply_edits(vec![Edit::replace(8..13, "3"), Edit::replace(0..3, "1")])
            .unwrap();
        assert_eq!(buffer.text(), "1 two 3");
        assert_eq!(buffer.version(), 1);
        assert_eq!(changes.changes().len(), 2);
        assert_eq!(buffer.line_count(), 1);
    }

    #[test]
    fn word_queries_stay_within_unicode_word_boundaries() {
        let buffer = TextBuffer::new("hello  world!\nnext");
        assert_eq!(buffer.word_range(2), 0..5);
        assert_eq!(buffer.word_range(6), 5..7);
        assert_eq!(buffer.word_range(15), 14..18);
        assert_eq!(buffer.word_right(0), 7, "a word and the spaces after it");
        assert_eq!(buffer.word_right(5), 7, "leading spaces only");
        assert_eq!(buffer.word_left(13), 12);
        assert_eq!(buffer.word_left(14), 12);
    }

    #[test]
    fn clones_share_text_until_one_of_them_changes() {
        let mut buffer = TextBuffer::new("shared");
        let snapshot = buffer.clone();
        assert!(Arc::ptr_eq(&buffer.text, &snapshot.text));
        buffer.apply_edits(vec![Edit::insert(0, ">")]).unwrap();
        assert_eq!((buffer.text(), snapshot.text()), (">shared", "shared"));
    }
}
