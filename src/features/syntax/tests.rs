use super::*;
use crate::domain::{identity::FileId, language::builtin};
use loom_editor_core::text::{EditKind, EditorMut, Selection, TextBuffer};
use std::{ops::Range, sync::atomic::AtomicBool};

fn windows(buffer: &TextBuffer) -> Vec<Window> {
    (0..buffer.line_count())
        .map(|line| Window {
            line: buffer.line_range(line),
            range: buffer.line_range(line),
        })
        .collect()
}
fn expected(buffer: &TextBuffer, window: &Window) -> Vec<TokenSpan> {
    Tokens::new(
        &buffer.text()[window.line.clone()],
        builtin("javascript"),
        0,
    )
    .map(|(range, kind)| TokenSpan {
        range: window.line.start + range.start..window.line.start + range.end,
        class: kind.highlight(),
    })
    .filter(|span| span.range.end > window.range.start && span.range.start < window.range.end)
    .collect()
}
fn edit(buffer: &mut TextBuffer, range: Range<usize>, text: &str) {
    EditorMut::new(buffer, &mut Selection::default(), Vec::new())
        .replace(range, text, EditKind::Other)
        .unwrap();
}
fn check(index: &mut Index, buffer: &TextBuffer) {
    let windows = windows(buffer);
    let result = index
        .prepare(
            FileId::new(1),
            Source::new(buffer),
            builtin("javascript"),
            &windows,
            &AtomicBool::new(false),
        )
        .unwrap();
    for (actual, window) in result.iter().zip(&windows) {
        assert_eq!(actual.spans.as_ref(), expected(buffer, window));
    }
}

#[test]
fn long_line_edit_reuses_unchanged_tokens_after_lexical_state_converges() {
    let mut buffer = TextBuffer::new(
        "const item={name:\"中e\u{301}\",value:123};function get(a){return a.value;}".repeat(8000),
    );
    let mut index = Index::default();
    check(&mut index, &buffer);
    let before = index.stats;
    let start = buffer.text().find("123").unwrap();
    edit(&mut buffer, start..start + 3, "4567");
    check(&mut index, &buffer);
    assert!(index.stats.scanned_bytes - before.scanned_bytes < 200);
    assert!(index.stats.reused_tokens - before.reused_tokens > 100_000);
}

#[test]
fn coalesced_edits_undo_redo_newlines_and_reload_match_a_fresh_scan() {
    let mut buffer = TextBuffer::new(
        "const value=123;\nnew Widget();\nconst text=\"hello\\\"world\";\n// comment\n".repeat(4),
    );
    let mut index = Index::default();
    check(&mut index, &buffer);
    let mut selection = Selection::default();
    let mut seed = 13u64;
    for turn in 0..120 {
        for _ in 0..3 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let start = buffer.grapheme_floor((seed as usize) % (buffer.len() + 1));
            let end = if turn % 3 == 0 {
                buffer.next_grapheme(start)
            } else {
                start
            };
            let parts = [
                "x", " ", "\"", "/*", "*/", "\n", "中", "", "new ", "(", "\\",
            ];
            EditorMut::new(&mut buffer, &mut selection, vec![])
                .replace(start..end, parts[turn % parts.len()], EditKind::Other)
                .unwrap();
        }
        check(&mut index, &buffer);
        if turn % 5 == 0 {
            EditorMut::new(&mut buffer, &mut selection, vec![]).undo();
            check(&mut index, &buffer);
            EditorMut::new(&mut buffer, &mut selection, vec![]).redo();
            check(&mut index, &buffer);
        }
    }
    buffer.reload("const replaced=\"entire document\";".into());
    check(&mut index, &buffer);
}

#[test]
fn unchanged_lines_keep_tokens_after_an_edit_and_line_number_shift() {
    let mut buffer = TextBuffer::new("const first=1;\nconst second=2;\nconst third=3;".to_owned());
    let mut index = Index::default();
    check(&mut index, &buffer);
    let before = index.stats;
    edit(&mut buffer, 0..0, "// inserted\n");
    let line = buffer.line_range(3);
    let result = index
        .prepare(
            FileId::new(1),
            Source::new(&buffer),
            builtin("javascript"),
            &[Window {
                line: line.clone(),
                range: line.clone(),
            }],
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(
        result[0].spans.as_ref(),
        expected(
            &buffer,
            &Window {
                line: line.clone(),
                range: line
            }
        )
    );
    assert_eq!(index.stats.scanned_bytes, before.scanned_bytes);
}

#[test]
fn partial_windows_preserve_string_comment_and_unicode_token_context() {
    let buffer = TextBuffer::new(format!(
        "const text=\"{}\";/*{}*/new Widget();//{}",
        "中e\u{301}😀 ".repeat(300),
        "abcdef ".repeat(300),
        "tail ".repeat(100)
    ));
    let mut index = Index::default();
    for column in [0, 400, 1000, 2000, 50, 3800, 100] {
        let Some(range) = buffer.display_column_range(0, column, column + 100) else {
            continue;
        };
        let window = Window {
            line: buffer.line_range(0),
            range,
        };
        let actual = index
            .prepare(
                FileId::new(1),
                Source::new(&buffer),
                builtin("javascript"),
                std::slice::from_ref(&window),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(actual[0].spans.as_ref(), expected(&buffer, &window));
    }
}

#[test]
fn indexes_evict_under_budget_and_cancellation_never_returns_partial_results() {
    let mut index = Index::new(1024);
    for id in 1..12 {
        let buffer = TextBuffer::new("const value=123;".repeat(100));
        index
            .prepare(
                FileId::new(id),
                Source::new(&buffer),
                builtin("javascript"),
                &windows(&buffer),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert!(index.stats.bytes <= 1024);
    }
    assert!(index.stats.evictions > 0);
    let buffer = TextBuffer::new("let value=1;".repeat(1000));
    assert!(
        index
            .prepare(
                FileId::new(20),
                Source::new(&buffer),
                builtin("javascript"),
                &windows(&buffer),
                &AtomicBool::new(true)
            )
            .is_none()
    );
}
