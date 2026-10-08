//! Default editing behaviour: what typing, deleting, moving and indenting
//! mean in an ordinary editor, expressed as edits on the editing core.

use crate::model::text::{Assoc, Edit, EditKind, EditorMut, Selection};

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

/// Spaces one indentation step inserts.
const INDENT: &str = "  ";

/// Insert typed or committed text over the selection. Plain characters join
/// the current typing run; line breaks and tabs are undo steps of their own.
pub fn insert(editor: &mut EditorMut<'_>, text: &str) {
    let kind = if text.contains(['\n', '\r', '\t']) {
        EditKind::Other
    } else {
        EditKind::Typing
    };
    // Selection-relative edits on a validated buffer cannot fail.
    let _ = editor.replace_selection(text, kind);
}

/// Paste text, converting its line breaks to the document's convention.
pub fn paste(editor: &mut EditorMut<'_>, text: &str) {
    let newline = editor.line_ending();
    let normalized = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', newline);
    let _ = editor.replace_selection(&normalized, EditKind::Other);
    editor.break_undo_group();
}

pub fn backspace(editor: &mut EditorMut<'_>) {
    erase(editor, false, false);
}

pub fn delete_forward(editor: &mut EditorMut<'_>) {
    erase(editor, true, false);
}

/// Delete the selection, or one character or word before or after the caret.
pub fn erase(editor: &mut EditorMut<'_>, forward: bool, word: bool) {
    let cursor = editor.cursor();
    let range = editor.selection().unwrap_or_else(|| match (forward, word) {
        (true, true) => cursor..editor.word_right(cursor),
        (true, false) => cursor..editor.next_grapheme(cursor),
        (false, true) => editor.word_left(cursor)..cursor,
        (false, false) => editor.prev_grapheme(cursor)..cursor,
    });
    let kind = match (forward, word) {
        (_, true) => EditKind::Other,
        (true, false) => EditKind::Delete,
        (false, false) => EditKind::Backspace,
    };
    let _ = editor.replace(range, "", kind);
}

/// Break the line, carrying over the current line's indentation.
pub fn enter(editor: &mut EditorMut<'_>) {
    let start = editor
        .selection()
        .map_or(editor.cursor(), |range| range.start);
    let line_start = editor.line_start(editor.line_of(start));
    let indent: String = editor.text()[line_start..start]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let newline = editor.line_ending();
    insert(editor, &format!("{newline}{indent}"));
}

/// Indent or outdent every line the selection touches, as one undo step.
/// Without a selection, Tab inserts spaces at the caret instead.
pub fn indent(editor: &mut EditorMut<'_>, outdent: bool) {
    if !outdent && editor.selection().is_none() {
        insert(editor, INDENT);
        editor.break_undo_group();
        return;
    }
    let cursor = editor.cursor();
    let range = editor.selection().unwrap_or(cursor..cursor);
    let first = editor.line_of(range.start);
    // A selection ending at a line start does not include that line.
    let last =
        if range.end > range.start && editor.line_start(editor.line_of(range.end)) == range.end {
            editor.line_of(range.end) - 1
        } else {
            editor.line_of(range.end)
        };
    let edits: Vec<Edit> = (first..=last)
        .filter_map(|line| {
            let start = editor.line_start(line);
            if !outdent {
                return Some(Edit::insert(start, INDENT));
            }
            let rest = &editor.text()[start..editor.line_end(line)];
            let remove = if rest.starts_with('\t') {
                1
            } else {
                rest.chars().take(2).take_while(|c| *c == ' ').count()
            };
            (remove > 0).then(|| Edit::delete(start..start + remove))
        })
        .collect();
    let selection = editor.selection_state();
    let _ = editor.edit(EditKind::Other, edits, |changes, _| Selection {
        cursor: changes.map(selection.cursor, Assoc::After),
        anchor: selection
            .anchor
            .map(|anchor| changes.map(anchor, Assoc::After)),
        preferred_column: None,
    });
    editor.break_undo_group();
}

/// Move the caret, extending the selection when `extend` is set.
pub fn navigate(editor: &mut EditorMut<'_>, movement: Movement, extend: bool) {
    if !extend
        && matches!(movement, Movement::Left | Movement::Right)
        && let Some(range) = editor.selection()
    {
        editor.set_cursor(if movement == Movement::Left {
            range.start
        } else {
            range.end
        });
        return;
    }
    let state = editor.selection_state();
    let cursor = state.cursor;
    let line = editor.line_of(cursor);
    let last_line = editor.line_count() - 1;
    let vertical_target = match movement {
        Movement::Up => Some(line.saturating_sub(1)),
        Movement::Down => Some((line + 1).min(last_line)),
        Movement::PageUp(lines) => Some(line.saturating_sub(lines)),
        Movement::PageDown(lines) => Some((line + lines).min(last_line)),
        _ => None,
    };
    if let Some(target) = vertical_target {
        let column = state
            .preferred_column
            .unwrap_or_else(|| editor.display_column(cursor));
        let offset = editor.offset_at_display_column(target, column);
        editor.select_to(offset, extend);
        editor.set_preferred_column(Some(column));
        return;
    }
    let target = match movement {
        Movement::Left => editor.prev_grapheme(cursor),
        Movement::Right => editor.next_grapheme(cursor),
        Movement::WordLeft => editor.word_left(cursor),
        Movement::WordRight => editor.word_right(cursor),
        Movement::Home => editor.line_start(line),
        Movement::End => editor.line_end(line),
        Movement::Start => 0,
        Movement::Finish => editor.len(),
        _ => unreachable!("vertical movements return above"),
    };
    editor.select_to(target, extend);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::text::TextBuffer;

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
            navigate(&mut b, Movement::Right, false);
            assert_eq!(b.cursor(), expected);
        }
        backspace(&mut b);
        backspace(&mut b);
        assert_eq!(b.text(), "中e\u{301}👩‍💻");
        backspace(&mut b);
        assert_eq!(b.text(), "中e\u{301}");
        b.set_cursor(4);
        assert_eq!(b.cursor(), 3);
    }

    #[test]
    fn selection_replace_and_history_restore_both_endpoints() {
        let mut s = Single::new("hello world");
        let mut b = s.ed();
        b.select_range(0..5);
        insert(&mut b, "你好");
        assert_eq!(b.text(), "你好 world");
        b.undo();
        assert_eq!(b.selected_text(), Some("hello"));
        b.redo();
        assert_eq!(b.text(), "你好 world");
        b.undo();
        insert(&mut b, "other");
        assert!(!b.can_redo());
    }

    #[test]
    fn typing_groups_but_navigation_and_paste_break_groups() {
        let mut s = Single::new("");
        let mut b = s.ed();
        insert(&mut b, "a");
        insert(&mut b, "b");
        insert(&mut b, "c");
        b.undo();
        assert_eq!(b.text(), "");
        b.redo();
        navigate(&mut b, Movement::Left, false);
        insert(&mut b, "x");
        paste(&mut b, "yz");
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
        navigate(&mut b, Movement::Down, true);
        navigate(&mut b, Movement::Down, true);
        assert_eq!(b.line_col(), (2, 5));
        assert_eq!(b.selection(), Some(5..14));
        navigate(&mut b, Movement::Left, false);
        assert_eq!(b.cursor(), 5);
    }

    #[test]
    fn crlf_enter_paste_and_indent_preserve_document_conventions() {
        let mut s = Single::new("  one\r\n  two");
        let mut b = s.ed();
        b.set_cursor(5);
        enter(&mut b);
        assert_eq!(b.text(), "  one\r\n  \r\n  two");
        b.undo();
        b.select_all();
        indent(&mut b, false);
        assert_eq!(b.text(), "    one\r\n    two");
        indent(&mut b, true);
        assert_eq!(b.text(), "  one\r\n  two");
        b.select_all();
        paste(&mut b, "a\nb\r\nc");
        assert_eq!(b.text(), "a\r\nb\r\nc");
    }

    #[test]
    fn indent_excludes_line_at_selection_end() {
        let mut s = Single::new("a\nb\nc");
        let mut b = s.ed();
        b.select_range(0..2);
        indent(&mut b, false);
        assert_eq!(b.text(), "  a\nb\nc");
        assert_eq!(b.selection(), Some(2..4));
        b.undo();
        assert_eq!(b.selection(), Some(0..2));
    }

    #[test]
    fn words_deletion_and_document_boundaries() {
        let mut s = Single::new("hello  world!");
        let mut b = s.ed();
        navigate(&mut b, Movement::WordRight, false);
        assert_eq!(b.cursor(), 7);
        erase(&mut b, true, true);
        assert_eq!(b.text(), "hello  !");
        erase(&mut b, false, true);
        assert_eq!(b.text(), "!");
        navigate(&mut b, Movement::Start, true);
        backspace(&mut b);
        navigate(&mut b, Movement::Finish, false);
        delete_forward(&mut b);
        assert_eq!(b.text(), "!");
    }

    #[test]
    fn edits_in_one_view_move_the_other_views() {
        let mut buffer = TextBuffer::new("one two three");
        let mut mine = Selection::caret(4);
        let mut theirs = Selection::spanning(8, 13);
        let mut ed = EditorMut::new(&mut buffer, &mut mine, vec![&mut theirs]);
        insert(&mut ed, "big ");
        assert_eq!(ed.text(), "one big two three");
        drop(ed);
        assert_eq!(theirs.range(), Some(12..17));
        assert_eq!(mine.cursor, 8);

        let mut ed = EditorMut::new(&mut buffer, &mut mine, vec![&mut theirs]);
        ed.select_all();
        indent(&mut ed, false);
        drop(ed);
        assert_eq!(buffer.text(), "  one big two three");
        assert_eq!(theirs.range(), Some(14..19));
    }

    #[test]
    fn typing_in_another_view_starts_a_new_undo_group() {
        let mut buffer = TextBuffer::new("ab");
        let mut left = Selection::caret(0);
        let mut right = Selection::caret(2);
        insert(
            &mut EditorMut::new(&mut buffer, &mut left, vec![&mut right]),
            "x",
        );
        insert(
            &mut EditorMut::new(&mut buffer, &mut right, vec![&mut left]),
            "y",
        );
        assert_eq!(buffer.text(), "xaby");
        EditorMut::new(&mut buffer, &mut right, vec![&mut left]).undo();
        assert_eq!(buffer.text(), "xab");
    }
}
