//! Line operations preserve selection direction and trailing-newline shape.
use crate::text::{Assoc, Edit, EditKind, EditorMut, Selection};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineOperation {
    MoveUp,
    MoveDown,
    CopyUp,
    CopyDown,
    Delete,
}

pub fn touched(editor: &EditorMut<'_>) -> (usize, usize) {
    let range = editor
        .selection()
        .unwrap_or(editor.cursor()..editor.cursor());
    let first = editor.line_of(range.start);
    let mut last = editor.line_of(range.end);
    if range.end > range.start && range.end == editor.line_start(last) {
        last = last.saturating_sub(1);
    }
    (first, last)
}
pub fn lines(editor: &mut EditorMut<'_>, operation: LineOperation) {
    if editor.has_secondary() {
        crate::text::multiple::lines(editor, operation);
        return;
    }
    let (first, last) = touched(editor);
    let start = editor.line_start(first);
    let end = if last + 1 < editor.line_count() {
        editor.line_start(last + 1)
    } else {
        editor.len()
    };
    let state = editor.selection_state();
    match operation {
        LineOperation::Delete => {
            let from = if end == editor.len() && last + 1 == editor.line_count() && first > 0 {
                editor.line_end(first - 1)
            } else {
                start
            };
            let column = state
                .cursor
                .saturating_sub(editor.line_start(editor.line_of(state.cursor)));
            let _ = editor.edit(
                EditKind::Other,
                vec![Edit::delete(from..end)],
                |_, buffer| {
                    let row = first.min(buffer.line_count() - 1);
                    Selection::caret((buffer.line_start(row) + column).min(buffer.line_end(row)))
                },
            );
        }
        LineOperation::CopyUp | LineOperation::CopyDown => {
            let block = &editor.text()[start..end];
            let down = operation == LineOperation::CopyDown;
            let eof = last + 1 == editor.line_count();
            let newline = editor.line_ending();
            let value = if eof {
                if down {
                    format!("{newline}{block}")
                } else {
                    format!("{block}{newline}")
                }
            } else {
                block.into()
            };
            let at = if down { end } else { start };
            let origin = at + if down && eof { newline.len() } else { 0 };
            let map = |p: usize| origin + p.saturating_sub(start);
            let _ = editor.edit(EditKind::Other, vec![Edit::insert(at, value)], |_, _| {
                Selection {
                    cursor: map(state.cursor),
                    anchor: state.anchor.map(map),
                    preferred_column: None,
                    wrap_affinity: Default::default(),
                }
            });
        }
        LineOperation::MoveUp | LineOperation::MoveDown => {
            let down = operation == LineOperation::MoveDown;
            if (!down && first == 0) || (down && last + 1 >= editor.line_count()) {
                return;
            }
            let low = if down { first } else { first - 1 };
            let high = if down { last + 1 } else { last };
            let from = editor.line_start(low);
            let to = if high + 1 < editor.line_count() {
                editor.line_start(high + 1)
            } else {
                editor.len()
            };
            let contents = (low..=high)
                .map(|row| editor.line(row).to_owned())
                .collect::<Vec<_>>();
            let endings = (low..=high)
                .map(|row| {
                    let next = if row + 1 < editor.line_count() {
                        editor.line_start(row + 1)
                    } else {
                        editor.len()
                    };
                    editor.text()[editor.line_end(row)..next].to_owned()
                })
                .collect::<Vec<_>>();
            let mut order = (0..contents.len()).collect::<Vec<_>>();
            if down {
                order.rotate_right(1);
            } else {
                order.rotate_left(1);
            }
            let mut origins = vec![0; contents.len()];
            let mut value = String::new();
            let mut block_end = from;
            for (position, &original) in order.iter().enumerate() {
                origins[original] = from + value.len();
                value.push_str(&contents[original]);
                value.push_str(&endings[position]);
                if low + original == last {
                    block_end = from + value.len();
                }
            }
            let boundary = end > start && last + 1 < editor.line_count();
            let map = |p: usize| {
                if boundary && p == end {
                    return block_end;
                }
                let row = editor.line_of(p);
                let col = p - editor.line_start(row);
                if (low..=high).contains(&row) {
                    origins[row - low] + col.min(contents[row - low].len())
                } else {
                    p
                }
            };
            let after = Selection {
                cursor: map(state.cursor),
                anchor: state.anchor.map(map),
                preferred_column: None,
                wrap_affinity: Default::default(),
            };
            let _ = editor.edit(
                EditKind::Other,
                vec![Edit::replace(from..to, value)],
                |_, _| after,
            );
        }
    }
    editor.break_undo_group();
}

pub fn comment(editor: &mut EditorMut<'_>, prefix: &str) {
    if editor.has_secondary() {
        crate::text::multiple::comment(editor, prefix);
        return;
    }
    if prefix.is_empty() {
        return;
    }
    let (first, last) = touched(editor);
    let rows = (first..=last)
        .filter_map(|row| {
            let line = editor.line(row);
            let trimmed = line.trim_start_matches([' ', '\t']);
            if trimmed.is_empty() && first != last {
                return None;
            }
            Some((
                editor.line_start(row) + line.len() - trimmed.len(),
                trimmed.starts_with(prefix),
            ))
        })
        .collect::<Vec<_>>();
    let remove = !rows.is_empty() && rows.iter().all(|(_, commented)| *commented);
    let edits = rows
        .into_iter()
        .map(|(at, _)| {
            if remove {
                let length =
                    prefix.len() + usize::from(editor.text()[at + prefix.len()..].starts_with(' '));
                Edit::delete(at..at + length)
            } else {
                Edit::insert(at, format!("{prefix} "))
            }
        })
        .collect();
    let state = editor.selection_state();
    let _ = editor.edit(EditKind::Other, edits, |changes, _| Selection {
        cursor: changes.map(state.cursor, Assoc::After),
        anchor: state.anchor.map(|a| changes.map(a, Assoc::After)),
        preferred_column: None,
        wrap_affinity: Default::default(),
    });
    editor.break_undo_group();
}
