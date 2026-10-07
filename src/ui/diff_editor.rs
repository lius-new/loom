//! Read-only inline and side-by-side diff editor.

use std::path::PathBuf;

use lgui::core::{CursorIcon, EventPolicy, UiFocusHandle, WheelUnit, clip};
use lgui::prelude::{Element, State, TextAlign, UiRect, VisualStyle, panel, text};

use crate::model::diff_document::{DiffRow, DiffRowKind, SplitDiffRow};
use crate::state::AppState;
use crate::theme;

const TOOLBAR_H: f32 = 34.0;
const PANE_HEADER_H: f32 = 22.0;
const ROW_H: f32 = 20.0;
const NUMBER_W: f32 = 44.0;
const MARKER_W: f32 = 20.0;
const SCROLLBAR_W: f32 = 4.0;
const BUTTON_H: f32 = 22.0;
const BUTTON_GAP: f32 = 4.0;

pub fn render(rect: UiRect, state: State<AppState>, editor_focus: UiFocusHandle) -> Element {
    let app = state.get();
    let Some(document) = app.workspace.active_diff() else {
        return panel(rect, VisualStyle::filled(theme::c().bg));
    };

    let split = app.git_split_diff;
    let starts = document.change_starts(split);
    let source_label = document.source_label();
    let absolute_path = document.absolute_path();
    let path_label = document.path.display().to_string();
    let additions = document.additions();
    let deletions = document.deletions();
    let (stored_x, stored_y) = app.workspace.active_scroll();
    let current_line = (stored_y / ROW_H).floor() as usize;
    let current_change = current_change_number(&starts, current_line);

    let body_top = rect.top + TOOLBAR_H + if split { PANE_HEADER_H } else { 0.0 };
    let body_rect = UiRect::new(rect.left, body_top, rect.right, rect.bottom);
    let row_count = if split {
        document.split_rows.len()
    } else {
        document.rows.len()
    };
    let content_h = row_count as f32 * ROW_H;
    let max_y = (content_h - body_rect.height()).max(0.0);
    let code_width = if split {
        (body_rect.width() / 2.0 - NUMBER_W - theme::CODE_PAD).max(1.0)
    } else {
        (body_rect.width() - NUMBER_W * 2.0 - MARKER_W - theme::CODE_PAD).max(1.0)
    };
    let longest_line = if split {
        document
            .split_rows
            .iter()
            .flat_map(|row| [row.old_text.as_deref(), row.new_text.as_deref()])
            .flatten()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or_default()
    } else {
        document
            .rows
            .iter()
            .map(|row| row.text.chars().count())
            .max()
            .unwrap_or_default()
    };
    let content_w = longest_line as f32 * theme::CHAR_W + theme::CODE_PAD * 2.0;
    let max_x = (content_w - code_width).max(0.0);
    let scroll_x = stored_x.clamp(0.0, max_x);
    let scroll_y = stored_y.clamp(0.0, max_y);

    let wheel_state = state.clone();
    let mut root = panel(rect, VisualStyle::filled(theme::c().bg))
        .event_policy(EventPolicy::INTERACTIVE)
        .on_wheel(move |cx, delta| {
            let (step_x, step_y) = match delta.unit {
                WheelUnit::Lines => (delta.x * ROW_H * 3.0, delta.y * ROW_H * 3.0),
                WheelUnit::Pixels => (delta.x, delta.y),
            };
            wheel_state.update(move |app| {
                let (step_x, step_y) = if app.editor.modifiers.shift() {
                    (step_x + step_y, 0.0)
                } else {
                    (step_x, step_y)
                };
                let (x, y) = app.workspace.active_scroll();
                app.workspace.set_active_scroll(
                    (x - step_x).clamp(0.0, max_x),
                    (y - step_y).clamp(0.0, max_y),
                );
            });
            cx.stop_propagation();
        });

    root = root.child(toolbar(
        UiRect::new(rect.left, rect.top, rect.right, rect.top + TOOLBAR_H),
        state.clone(),
        editor_focus,
        path_label,
        additions,
        deletions,
        split,
        starts,
        current_change,
        absolute_path,
    ));

    if split {
        root = root.child(split_headers(
            UiRect::new(rect.left, rect.top + TOOLBAR_H, rect.right, body_top),
            source_label,
        ));
    }

    if row_count == 0 {
        let label = format!("{} — {source_label}", document.path.display());
        let message = if document.binary {
            format!("Binary file changed: {label}")
        } else {
            format!("No textual changes: {label}")
        };
        return root.child(text(
            UiRect::new(
                body_rect.left + 24.0,
                body_rect.top + 24.0,
                body_rect.right - 24.0,
                body_rect.top + 64.0,
            ),
            message,
            theme::mono(theme::c().text_dim, theme::UI_SIZE),
        ));
    }

    root = if split {
        root.child(render_split(
            body_rect,
            &document.split_rows,
            scroll_x,
            scroll_y,
        ))
    } else {
        root.child(render_inline(body_rect, &document.rows, scroll_x, scroll_y))
    };

    if max_y > 0.0 {
        let thumb_h =
            (body_rect.height() * body_rect.height() / content_h).clamp(24.0, body_rect.height());
        let travel = body_rect.height() - thumb_h;
        let thumb_top = body_rect.top + travel * (scroll_y / max_y);
        root = root.child(panel(
            UiRect::new(
                body_rect.right - SCROLLBAR_W,
                thumb_top,
                body_rect.right,
                thumb_top + thumb_h,
            ),
            VisualStyle::filled(theme::c().text_faint).radius(2.0),
        ));
    }

    root
}

#[allow(clippy::too_many_arguments)]
fn toolbar(
    rect: UiRect,
    state: State<AppState>,
    editor_focus: UiFocusHandle,
    path: String,
    additions: usize,
    deletions: usize,
    split: bool,
    starts: Vec<usize>,
    current_change: Option<usize>,
    absolute_path: PathBuf,
) -> Element {
    let mut bar = panel(rect, VisualStyle::filled(theme::c().sidebar));
    bar = bar.child(panel(
        UiRect::new(rect.left, rect.bottom - 1.0, rect.right, rect.bottom),
        VisualStyle::filled(theme::c().border),
    ));

    let compact = rect.width() < 560.0;
    let button_top = rect.top + (rect.height() - BUTTON_H) / 2.0;
    let mut right = rect.right - 8.0;

    let open_rect = take_button_rect(&mut right, button_top, if compact { 42.0 } else { 68.0 });
    let open_state = state.clone();
    bar = bar.child(
        toolbar_button(open_rect, if compact { "Open" } else { "Open File" }, false).on_click(
            move || match std::fs::read_to_string(&absolute_path) {
                Ok(contents) => {
                    let path = absolute_path.clone();
                    open_state.update(move |app| {
                        app.workspace.open_path(path, contents);
                    });
                    editor_focus.focus();
                }
                Err(error) => open_state.update(move |app| {
                    app.show_toast(format!("Could not open file: {error}"));
                }),
            },
        ),
    );

    let split_rect = take_button_rect(&mut right, button_top, if compact { 28.0 } else { 42.0 });
    let split_state = state.clone();
    bar = bar.child(
        toolbar_button(split_rect, if compact { "S" } else { "Split" }, split).on_click(
            move || {
                split_state.update(|app| {
                    app.git_split_diff = true;
                    app.workspace.set_active_scroll(0.0, 0.0);
                });
            },
        ),
    );

    let inline_rect = take_button_rect(&mut right, button_top, if compact { 28.0 } else { 46.0 });
    let inline_state = state.clone();
    bar = bar.child(
        toolbar_button(inline_rect, if compact { "I" } else { "Inline" }, !split).on_click(
            move || {
                inline_state.update(|app| {
                    app.git_split_diff = false;
                    app.workspace.set_active_scroll(0.0, 0.0);
                });
            },
        ),
    );

    let next_rect = take_button_rect(&mut right, button_top, 24.0);
    let next_state = state.clone();
    let next_starts = starts.clone();
    bar = bar.child(toolbar_button(next_rect, "↓", false).on_click(move || {
        next_state.update(|app| navigate_change(app, &next_starts, true));
    }));

    let prev_rect = take_button_rect(&mut right, button_top, 24.0);
    let prev_state = state;
    let previous_starts = starts.clone();
    bar = bar.child(toolbar_button(prev_rect, "↑", false).on_click(move || {
        prev_state.update(|app| navigate_change(app, &previous_starts, false));
    }));

    let position = current_change.map_or_else(
        || format!("0/{}", starts.len()),
        |current| format!("{current}/{}", starts.len()),
    );
    let position_w = 42.0;
    let position_rect = UiRect::new(right - position_w, rect.top, right, rect.bottom);
    right -= position_w + BUTTON_GAP;
    bar = bar.child(text(
        position_rect,
        position,
        theme::mono_right(theme::c().text_faint, theme::SMALL),
    ));

    let stats = format!("+{additions}  −{deletions}");
    let stats_w = if compact { 62.0 } else { 82.0 };
    let stats_rect = UiRect::new(right - stats_w, rect.top, right, rect.bottom);
    right -= stats_w + 8.0;
    bar = bar.child(text(
        stats_rect,
        stats,
        theme::mono_right(theme::c().text_dim, theme::SMALL),
    ));

    let path_rect = UiRect::new(
        rect.left + 12.0,
        rect.top,
        right.max(rect.left + 12.0),
        rect.bottom,
    );
    bar.child(clip(path_rect, 0.0, 0.0).child(text(
        UiRect::new(
            path_rect.left,
            path_rect.top,
            path_rect.left + path.chars().count() as f32 * theme::CHAR_W + 12.0,
            path_rect.bottom,
        ),
        path,
        theme::mono(theme::c().text_soft, theme::UI_SIZE),
    )))
}

fn take_button_rect(right: &mut f32, top: f32, width: f32) -> UiRect {
    let rect = UiRect::new(*right - width, top, *right, top + BUTTON_H);
    *right -= width + BUTTON_GAP;
    rect
}

fn toolbar_button(rect: UiRect, label: &'static str, active: bool) -> Element {
    let mut style = theme::mono(
        if active {
            theme::c().text
        } else {
            theme::c().text_dim
        },
        theme::SMALL,
    );
    style.align = TextAlign::Center;
    panel(
        rect,
        if active {
            VisualStyle::filled(theme::c().active_line).radius(3.0)
        } else {
            VisualStyle::default().radius(3.0)
        },
    )
    .event_policy(EventPolicy::INTERACTIVE)
    .cursor(CursorIcon::Pointer)
    .child(text(rect, label, style))
}

fn split_headers(rect: UiRect, source_label: &str) -> Element {
    let middle = rect.left + rect.width() / 2.0;
    panel(rect, VisualStyle::filled(theme::c().sidebar))
        .child(text(
            UiRect::new(rect.left + 12.0, rect.top, middle - 8.0, rect.bottom),
            "Original",
            theme::mono(theme::c().text_faint, theme::SMALL),
        ))
        .child(text(
            UiRect::new(middle + 12.0, rect.top, rect.right - 8.0, rect.bottom),
            source_label.to_owned(),
            theme::mono(theme::c().text_faint, theme::SMALL),
        ))
        .child(panel(
            UiRect::new(middle, rect.top, middle + 1.0, rect.bottom),
            VisualStyle::filled(theme::c().border),
        ))
        .child(panel(
            UiRect::new(rect.left, rect.bottom - 1.0, rect.right, rect.bottom),
            VisualStyle::filled(theme::c().border),
        ))
}

fn render_inline(rect: UiRect, rows: &[DiffRow], scroll_x: f32, scroll_y: f32) -> Element {
    let text_left = rect.left + NUMBER_W * 2.0 + MARKER_W;
    let text_viewport = UiRect::new(text_left, rect.top, rect.right, rect.bottom);
    let start = (scroll_y / ROW_H).floor() as usize;
    let end = (((scroll_y + rect.height()) / ROW_H).ceil() as usize + 1).min(rows.len());
    let mut body = clip(rect, 0.0, 0.0);

    for (index, row) in rows.iter().enumerate().take(end).skip(start) {
        let top = rect.top + index as f32 * ROW_H - scroll_y;
        let row_rect = UiRect::new(rect.left, top, rect.right, top + ROW_H);
        let (background, foreground, marker) = match row.kind {
            DiffRowKind::Context => (None, theme::c().text_soft, ""),
            DiffRowKind::Addition => (Some(theme::c().diff.add_bg), theme::c().diff.add_fg, "+"),
            DiffRowKind::Deletion => (Some(theme::c().diff.del_bg), theme::c().diff.del_fg, "−"),
            DiffRowKind::Hunk => (Some(theme::c().surface), theme::c().text_dim, ""),
        };
        if let Some(background) = background {
            body = body.child(panel(row_rect, VisualStyle::filled(background)));
        }
        if row.kind == DiffRowKind::Hunk {
            body = body.child(text(
                UiRect::new(rect.left + 10.0, top, rect.right - 10.0, top + ROW_H),
                row.text.clone(),
                theme::mono(theme::c().text_dim, theme::SMALL),
            ));
            continue;
        }

        body = body
            .child(line_number(rect.left, top, row.old_line))
            .child(line_number(rect.left + NUMBER_W, top, row.new_line))
            .child(text(
                UiRect::new(rect.left + NUMBER_W * 2.0, top, text_left, top + ROW_H),
                marker,
                theme::mono(foreground, theme::CODE_SIZE),
            ))
            .child(code_line(
                text_viewport,
                text_left + theme::CODE_PAD - scroll_x,
                top,
                &row.text,
                foreground,
            ));
    }

    body.child(vertical_rule(rect.left + NUMBER_W, rect))
        .child(vertical_rule(text_left, rect))
}

fn render_split(rect: UiRect, rows: &[SplitDiffRow], scroll_x: f32, scroll_y: f32) -> Element {
    let middle = rect.left + rect.width() / 2.0;
    let old_text_left = rect.left + NUMBER_W;
    let new_text_left = middle + NUMBER_W;
    let old_viewport = UiRect::new(old_text_left, rect.top, middle, rect.bottom);
    let new_viewport = UiRect::new(new_text_left, rect.top, rect.right, rect.bottom);
    let start = (scroll_y / ROW_H).floor() as usize;
    let end = (((scroll_y + rect.height()) / ROW_H).ceil() as usize + 1).min(rows.len());
    let mut body = clip(rect, 0.0, 0.0);

    for (index, row) in rows.iter().enumerate().take(end).skip(start) {
        let top = rect.top + index as f32 * ROW_H - scroll_y;
        if let Some(hunk) = row.hunk.as_ref() {
            body = body
                .child(panel(
                    UiRect::new(rect.left, top, rect.right, top + ROW_H),
                    VisualStyle::filled(theme::c().surface),
                ))
                .child(text(
                    UiRect::new(rect.left + 10.0, top, rect.right - 10.0, top + ROW_H),
                    hunk.clone(),
                    theme::mono(theme::c().text_dim, theme::SMALL),
                ));
            continue;
        }

        if row.changed && row.old_text.is_some() {
            body = body.child(panel(
                UiRect::new(rect.left, top, middle, top + ROW_H),
                VisualStyle::filled(theme::c().diff.del_bg),
            ));
        }
        if row.changed && row.new_text.is_some() {
            body = body.child(panel(
                UiRect::new(middle, top, rect.right, top + ROW_H),
                VisualStyle::filled(theme::c().diff.add_bg),
            ));
        }

        let old_color = if row.changed {
            theme::c().diff.del_fg
        } else {
            theme::c().text_soft
        };
        let new_color = if row.changed {
            theme::c().diff.add_fg
        } else {
            theme::c().text_soft
        };
        body = body
            .child(line_number(rect.left, top, row.old_line))
            .child(line_number(middle, top, row.new_line));
        if let Some(old_text) = row.old_text.as_ref() {
            body = body.child(code_line(
                old_viewport,
                old_text_left + theme::CODE_PAD - scroll_x,
                top,
                old_text,
                old_color,
            ));
        }
        if let Some(new_text) = row.new_text.as_ref() {
            body = body.child(code_line(
                new_viewport,
                new_text_left + theme::CODE_PAD - scroll_x,
                top,
                new_text,
                new_color,
            ));
        }
    }

    body.child(vertical_rule(old_text_left, rect))
        .child(vertical_rule(middle, rect))
        .child(vertical_rule(new_text_left, rect))
}

fn line_number(left: f32, top: f32, number: Option<usize>) -> Element {
    text(
        UiRect::new(left, top, left + NUMBER_W - 8.0, top + ROW_H),
        number.map(|line| line.to_string()).unwrap_or_default(),
        theme::mono_right(theme::c().text_faint, theme::SMALL),
    )
    .into()
}

fn code_line(
    viewport: UiRect,
    left: f32,
    top: f32,
    value: &str,
    color: lgui::core::Color,
) -> Element {
    let width = (value.chars().count() as f32 * theme::CHAR_W + 16.0).max(viewport.width());
    clip(viewport, 0.0, 0.0).child(text(
        UiRect::new(left, top, left + width, top + ROW_H),
        value.to_owned(),
        theme::mono(color, theme::CODE_SIZE),
    ))
}

fn vertical_rule(x: f32, rect: UiRect) -> Element {
    panel(
        UiRect::new(x - 1.0, rect.top, x, rect.bottom),
        VisualStyle::filled(theme::c().border),
    )
}

fn current_change_number(starts: &[usize], current_line: usize) -> Option<usize> {
    if starts.is_empty() {
        return None;
    }
    Some(
        starts
            .iter()
            .rposition(|start| *start <= current_line)
            .map_or(1, |index| index + 1),
    )
}

fn navigate_change(app: &mut AppState, starts: &[usize], next: bool) {
    if starts.is_empty() {
        return;
    }
    let (x, y) = app.workspace.active_scroll();
    let current = (y / ROW_H).floor() as usize;
    let target = if next {
        starts
            .iter()
            .copied()
            .find(|start| *start > current)
            .unwrap_or(starts[0])
    } else {
        starts
            .iter()
            .rev()
            .copied()
            .find(|start| *start < current)
            .unwrap_or(*starts.last().expect("change list is not empty"))
    };
    app.workspace.set_active_scroll(x, target as f32 * ROW_H);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_counter_tracks_the_nearest_change_above_the_viewport() {
        let starts = vec![3, 10, 20];
        assert_eq!(current_change_number(&starts, 0), Some(1));
        assert_eq!(current_change_number(&starts, 10), Some(2));
        assert_eq!(current_change_number(&starts, 19), Some(2));
        assert_eq!(current_change_number(&[], 19), None);
    }

    #[test]
    fn change_navigation_wraps_in_both_directions() {
        let mut app = AppState::new();
        app.workspace.open_path("a.txt".into(), "text".into());
        navigate_change(&mut app, &[2, 8], true);
        assert_eq!(app.workspace.active_scroll().1, 2.0 * ROW_H);
        app.workspace.set_active_scroll(0.0, 8.0 * ROW_H);
        navigate_change(&mut app, &[2, 8], true);
        assert_eq!(app.workspace.active_scroll().1, 2.0 * ROW_H);
        navigate_change(&mut app, &[2, 8], false);
        assert_eq!(app.workspace.active_scroll().1, 8.0 * ROW_H);
    }
}
