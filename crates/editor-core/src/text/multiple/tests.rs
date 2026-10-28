use super::*;
use crate::normal::{self, LineOperation, Movement};
struct Multi {
    buffer: TextBuffer,
    primary: Selection,
    extra: Vec<Selection>,
}
impl Multi {
    fn new(text: &str, states: Vec<Selection>) -> Self {
        let mut states = states.into_iter();
        Self {
            buffer: TextBuffer::new(text),
            primary: states.next().unwrap(),
            extra: states.collect(),
        }
    }
    fn editor(&mut self) -> EditorMut<'_> {
        EditorMut::new(&mut self.buffer, &mut self.primary, vec![]).with_secondary(&mut self.extra)
    }
}
#[test]
fn atomic_multicursor_typing_history_peer_mapping_and_unicode() {
    let mut m = Multi::new(
        "中e\u{301}👩‍💻\r\n尾",
        vec![Selection::caret(3), Selection::caret(19)],
    );
    let mut peer = Selection::caret(22);
    let mut peer_extra = Selection::caret(19);
    let mut ed = EditorMut::new(
        &mut m.buffer,
        &mut m.primary,
        vec![&mut peer, &mut peer_extra],
    )
    .with_secondary(&mut m.extra);
    let before = ed.selections();
    let version = ed.version();
    normal::insert(&mut ed, "文");
    assert_eq!(ed.version(), version + 1);
    assert_eq!(ed.text(), "中文e\u{301}👩‍💻\r\n文尾");
    normal::insert(&mut ed, "字");
    let after = ed.selections();
    assert!(
        ed.selections()
            .iter()
            .all(|s| ed.is_grapheme_boundary(s.cursor))
    );
    ed.undo();
    assert_eq!(ed.text(), "中e\u{301}👩‍💻\r\n尾");
    assert_eq!(ed.selections(), before);
    ed.redo();
    assert_eq!(ed.selections(), after);
    assert_eq!(ed.text(), "中文字e\u{301}👩‍💻\r\n文字尾");
    drop(ed);
    assert_eq!(peer.cursor, 34);
    assert_eq!(peer_extra.cursor, 25);
}
#[test]
fn multicursor_normalizes_overlap_preserves_direction_and_keeps_adjacent_ranges() {
    let mut m = Multi::new("abcdef", vec![Selection::spanning(4, 1)]);
    let mut e = m.editor();
    e.set_selections(
        Selection::spanning(4, 1),
        vec![
            Selection::spanning(3, 6),
            Selection::caret(2),
            Selection::caret(6),
        ],
    );
    assert_eq!(e.selections(), vec![Selection::spanning(6, 1)]);
    e.set_selections(Selection::spanning(0, 1), vec![Selection::spanning(1, 2)]);
    normal::insert(&mut e, "x");
    assert_eq!(e.text(), "xxcdef");
    assert_eq!(e.selections().len(), 2);
    e.undo();
    assert_eq!(
        e.selections(),
        vec![Selection::spanning(0, 1), Selection::spanning(1, 2)]
    );
    e.set_selections(Selection::caret(2), vec![Selection::caret(4)]);
    normal::erase(&mut e, false, true);
    assert_eq!(e.text(), "ef");
    assert_eq!(e.selections(), vec![Selection::caret(0)]);
    e.undo();
    assert_eq!(
        e.selections(),
        vec![Selection::caret(2), Selection::caret(4)]
    );
}
#[test]
fn multicursor_pair_provenance_smart_enter_dedent_and_ime_literal() {
    let mut m = Multi::new("\r\n", vec![Selection::caret(0), Selection::caret(2)]);
    let mut e = m.editor();
    normal::type_text(&mut e, "{", "rust");
    assert_eq!(e.text(), "{}\r\n{}");
    normal::enter(&mut e);
    assert_eq!(e.text(), "{\r\n  \r\n}\r\n{\r\n  \r\n}");
    e.undo();
    normal::backspace(&mut e);
    assert_eq!(e.text(), "\r\n");
    e.undo();
    normal::type_text(&mut e, "}", "rust");
    assert_eq!(e.text(), "{}\r\n{}");
    assert_eq!(
        e.selections().iter().map(|s| s.cursor).collect::<Vec<_>>(),
        [2, 6]
    );
    normal::insert(&mut e, "中文()");
    assert_eq!(e.text(), "{}中文()\r\n{}中文()");
    let mut m = Multi::new("    \n    ", vec![Selection::caret(4), Selection::caret(9)]);
    let mut e = m.editor();
    normal::type_text(&mut e, "}", "rust");
    assert_eq!(e.text(), "  }\n  }");
}
#[test]
fn multicursor_paste_distributes_source_order_cut_and_format_undo_restore_all() {
    let mut m = Multi::new(
        "a\r\nb",
        vec![Selection::spanning(3, 4), Selection::spanning(0, 1)],
    );
    let mut e = m.editor();
    assert_eq!(e.as_ref().selected_texts().as_deref(), Some("a\r\nb"));
    normal::paste(&mut e, "中\n尾");
    assert_eq!(e.text(), "中\r\n尾");
    assert_eq!(e.cursor(), 8);
    e.undo();
    assert_eq!(e.selections().len(), 2);
    normal::paste(&mut e, "x\ny\nz");
    assert_eq!(e.text(), "x\r\ny\r\nz\r\nx\r\ny\r\nz");
    e.undo();
    e.set_selections(Selection::spanning(0, 1), vec![Selection::caret(3)]);
    e.cut_selections();
    assert_eq!(e.text(), "\r\nb");
    e.undo();
    assert_eq!(
        e.selections(),
        vec![Selection::spanning(0, 1), Selection::caret(3)]
    );
    e.set_selections(Selection::caret(1), vec![Selection::caret(4)]);
    let before = e.selections();
    let state = e.selection_state();
    e.edit(
        EditKind::Other,
        vec![Edit::insert(0, "  "), Edit::insert(3, "  ")],
        |c, _| Selection::caret(c.map(state.cursor, Assoc::After)),
    )
    .unwrap();
    assert_eq!(
        e.selections().iter().map(|s| s.cursor).collect::<Vec<_>>(),
        [3, 8]
    );
    e.undo();
    assert_eq!(e.selections(), before);
    e.redo();
    assert_eq!(e.text(), "  a\r\n  b");
    e.set_line_ending(crate::text::LineEnding::Lf).unwrap();
    assert_eq!(
        e.selections().iter().map(|s| s.cursor).collect::<Vec<_>>(),
        [3, 7]
    );
    e.undo();
    assert_eq!(e.text(), "  a\r\n  b");
}
#[test]
fn multicursor_line_commands_merge_touched_rows_without_losing_carets() {
    for op in [
        LineOperation::MoveUp,
        LineOperation::MoveDown,
        LineOperation::CopyUp,
        LineOperation::CopyDown,
        LineOperation::Delete,
    ] {
        let mut m = Multi::new(
            "zero\r\none\r\ntwo\r\nthree\r\nfour",
            vec![
                Selection::caret(7),
                Selection::caret(8),
                Selection::caret(17),
            ],
        );
        let mut e = m.editor();
        let before = e.selections();
        normal::lines(&mut e, op);
        let expected = match op {
            LineOperation::MoveUp => "one\r\nzero\r\nthree\r\ntwo\r\nfour",
            LineOperation::MoveDown => "zero\r\ntwo\r\none\r\nfour\r\nthree",
            LineOperation::CopyUp | LineOperation::CopyDown => {
                "zero\r\none\r\none\r\ntwo\r\nthree\r\nthree\r\nfour"
            }
            LineOperation::Delete => "zero\r\ntwo\r\nfour",
        };
        assert_eq!(e.text(), expected, "{op:?}");
        let after = e.selections();
        e.undo();
        assert_eq!(e.text(), "zero\r\none\r\ntwo\r\nthree\r\nfour");
        assert_eq!(e.selections(), before);
        e.redo();
        assert_eq!(e.selections(), after);
    }
    let mut m = Multi::new(
        "one\ntwo",
        vec![
            Selection::caret(0),
            Selection::caret(1),
            Selection::caret(4),
        ],
    );
    let mut e = m.editor();
    normal::indent(&mut e, true);
    normal::comment(&mut e, "//");
    assert_eq!(e.text(), "// one\n// two");
    normal::comment(&mut e, "//");
    assert_eq!(e.text(), "one\ntwo");
    e.set_selections(
        Selection::spanning(0, 1),
        vec![Selection::spanning(1, 2), Selection::caret(4)],
    );
    normal::indent(&mut e, false);
    assert_eq!(e.text(), "  one\n  two");
}
#[test]
fn multicursor_limits_large_documents_and_moves_each_caret_by_grapheme() {
    let mut m = Multi::new(
        &"中😀e\u{301}\r\n".repeat(100_000),
        vec![Selection::caret(0)],
    );
    let mut e = m.editor();
    e.set_selections(
        Selection::caret(0),
        (1..300)
            .map(|row| Selection::caret(e.line_start(row)))
            .collect(),
    );
    assert_eq!(e.selections().len(), 256);
    normal::navigate(&mut e, Movement::Right, false);
    assert!(
        e.selections()
            .iter()
            .all(|s| e.buffer().line_col(s.cursor).1 == 1)
    );
    let v = e.version();
    normal::insert(&mut e, "字");
    assert_eq!(e.version(), v + 1);
    assert_eq!(e.selections().len(), 256);
    e.undo();
    assert_eq!(e.len(), "中😀e\u{301}\r\n".len() * 100_000);
}

#[test]
fn multicursor_same_line_closers_do_not_make_overlapping_dedent_edits() {
    let mut m = Multi::new("    ", vec![Selection::caret(3), Selection::caret(4)]);
    let mut e = m.editor();
    let version = e.version();
    normal::type_text(&mut e, "}", "rust");
    assert_eq!(e.text(), "   } }");
    assert_eq!(e.version(), version + 1);
    e.undo();
    assert_eq!(e.text(), "    ");
}

#[test]
fn multicursor_readonly_copy_merges_selections_transformed_by_another_view() {
    let buffer = TextBuffer::new("abc");
    let extra = vec![Selection::spanning(1, 3), Selection::spanning(0, 2)];
    let editor =
        crate::text::Editor::new(&buffer, Selection::spanning(0, 2)).with_secondary(&extra);
    assert_eq!(editor.selections(), vec![Selection::spanning(0, 3)]);
    assert_eq!(editor.selected_texts().as_deref(), Some("abc"));
}
