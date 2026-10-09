//! Read-only inline and side-by-side diff editor.

use std::path::PathBuf;

use lgui::core::{CursorIcon, EventPolicy, UiFocusHandle, WheelUnit, clip};
use lgui::prelude::{Element, State, TextAlign, UiRect, VisualStyle, panel, text};

use crate::editor::editor_view::{self, SCROLLBAR_SIZE, ScrollMetrics};
use crate::model::diff_document::{DiffRow, DiffRowKind, SplitDiffRow};
use crate::model::pane_layout::PaneId;
use crate::state::AppState;
use crate::theme;

const TOOLBAR_H: f32 = 34.0;
const ROW_H: f32 = 20.0;
const NUMBER_W: f32 = 44.0;
const MARKER_W: f32 = 20.0;
const BUTTON_H: f32 = 22.0;
const BUTTON_GAP: f32 = 4.0;

/// Render the diff shown by `pane` into `rect`.
pub fn render(
    pane: PaneId,
    rect: UiRect,
    state: State<AppState>,
    editor_focus: UiFocusHandle,
) -> Element {
    let app = state.get();
    let Some(document) = app.workspace.diff(pane) else {
        return panel(rect, VisualStyle::filled(theme::c().bg));
    };

    let split = app.git_split_diff;
    let starts = document.change_starts(split);
    let source_label = document.source_label();
    let absolute_path = document.absolute_path();
    let path_label = document.path.display().to_string();
    let additions = document.additions();
    let deletions = document.deletions();
    let (stored_x, stored_y) = app.workspace.scroll(pane);
    let current_line = (stored_y / ROW_H).floor() as usize;
    let current_change = current_change_number(&starts, current_line);

    let body_top = rect.top + TOOLBAR_H;
    let body_rect = UiRect::new(rect.left, body_top, rect.right, rect.bottom);
    let row_count = if split {
        document.split_rows.len()
    } else {
        document.rows.len()
    };
    let metrics = scroll_metrics(
        body_rect,
        split,
        row_count,
        document.longest_line(split),
        app.git_split_ratio,
    );
    let max_y = metrics.max_y;
    let max_x = metrics.max_x;
    let scroll_x = stored_x.clamp(0.0, max_x);
    let scroll_y = stored_y.clamp(0.0, max_y);

    let wheel_state = state.clone();
    let mut root = panel(rect, VisualStyle::filled(theme::c().bg))
        .key(format!("diff-pane-{}", pane.get()))
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
                let (x, y) = app.workspace.scroll(pane);
                app.workspace.set_scroll(
                    pane,
                    (x - step_x).clamp(0.0, max_x),
                    (y - step_y).clamp(0.0, max_y),
                );
            });
            cx.stop_propagation();
        });
    let pointer_state = state.clone();
    let pointer_focus = editor_focus.clone();
    root = root.on_pointer_down(move |_cx, _pointer| {
        pointer_state.update(move |app| {
            app.workspace.activate_pane(pane);
        });
        pointer_focus.focus();
    });

    root = root.child(toolbar(
        pane,
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

    let content_rect = UiRect::new(
        body_rect.left,
        body_rect.top,
        body_rect.right - if max_y > 0.0 { SCROLLBAR_SIZE } else { 0.0 },
        body_rect.bottom - if max_x > 0.0 { SCROLLBAR_SIZE } else { 0.0 },
    );
    root = if split {
        root.child(render_split(
            content_rect,
            &document.split_rows,
            scroll_x,
            scroll_y,
            app.git_split_ratio,
        ))
    } else {
        root.child(render_inline(
            content_rect,
            &document.rows,
            scroll_x,
            scroll_y,
        ))
    };

    if split {
        let down = state.clone();
        let moving = state.clone();
        let up = state.clone();
        root = root.child(crate::ui::pane_sash::horizontal(
            format!("git-split-{}", pane.get()),
            split_middle(content_rect, app.git_split_ratio),
            (content_rect.top, content_rect.bottom),
            app.git_split_drag == Some(pane),
            move || down.update(|app| app.git_split_drag = Some(pane)),
            move |x| {
                moving.try_update(|app| {
                    if app.git_split_drag != Some(pane) || content_rect.width() <= 0.0 {
                        return false;
                    }
                    let position = crate::ui::pane_sash::split_position(
                        content_rect.left,
                        content_rect.right,
                        x,
                    );
                    app.git_split_ratio = (position - content_rect.left) / content_rect.width();
                    true
                });
            },
            move || up.update(|app| app.git_split_drag = None),
        ));
    }

    let emphasized = app.editor_hovered == Some(pane)
        || (app.workspace.active_pane() == pane
            && (app.editor_vertical_scrollbar_dragging
                || app.editor_horizontal_scrollbar_dragging));
    if max_y > 0.0 {
        root = root.child(editor_view::vertical_scrollbar(
            pane,
            body_rect,
            metrics,
            scroll_y,
            max_x > 0.0,
            emphasized,
            state.clone(),
        ));
    }
    if max_x > 0.0 {
        root = root.child(editor_view::horizontal_scrollbar(
            pane,
            body_rect,
            code_left(body_rect, split),
            metrics,
            scroll_x,
            max_y > 0.0,
            emphasized,
            state,
        ));
    }

    root
}

fn code_left(rect: UiRect, split: bool) -> f32 {
    rect.left
        + if split {
            NUMBER_W
        } else {
            NUMBER_W * 2.0 + MARKER_W
        }
}

fn split_middle(rect: UiRect, ratio: f32) -> f32 {
    crate::ui::pane_sash::split_position(rect.left, rect.right, rect.left + rect.width() * ratio)
}

fn scroll_metrics(
    rect: UiRect,
    split: bool,
    rows: usize,
    longest_line: usize,
    ratio: f32,
) -> ScrollMetrics {
    let content_h = rows as f32 * ROW_H;
    let content_w = longest_line as f32 * theme::CHAR_W + theme::CODE_PAD * 2.0;
    let mut viewport_h = rect.height().max(1.0);
    let mut viewport_w = 1.0;
    for _ in 0..3 {
        let width = rect.width()
            - if content_h > viewport_h {
                SCROLLBAR_SIZE
            } else {
                0.0
            };
        viewport_w = (if split {
            let bounds = UiRect::new(rect.left, rect.top, rect.left + width, rect.bottom);
            let left_width = split_middle(bounds, ratio) - rect.left;
            left_width.min(width - left_width) - NUMBER_W
        } else {
            width - NUMBER_W * 2.0 - MARKER_W
        })
        .max(1.0);
        viewport_h = (rect.height()
            - if content_w > viewport_w {
                SCROLLBAR_SIZE
            } else {
                0.0
            })
        .max(1.0);
    }
    ScrollMetrics {
        viewport_w,
        viewport_h,
        content_w: content_w.max(viewport_w),
        content_h,
        max_x: (content_w - viewport_w).max(0.0),
        max_y: (content_h - viewport_h).max(0.0),
    }
}

/// Use the same window-level drag handling as the text editor, including
/// movements outside the scrollbar strip.
pub(crate) fn drag_scrollbars(app: &mut AppState, rect: UiRect, x: f32, y: f32) -> bool {
    if !app.editor_vertical_scrollbar_dragging && !app.editor_horizontal_scrollbar_dragging {
        return false;
    }
    let split = app.git_split_diff;
    let Some(document) = app.workspace.diff(app.workspace.active_pane()) else {
        return editor_view::finish_scrollbar_drag(app);
    };
    let body = UiRect::new(rect.left, rect.top + TOOLBAR_H, rect.right, rect.bottom);
    let rows = if split {
        document.split_rows.len()
    } else {
        document.rows.len()
    };
    let metrics = scroll_metrics(
        body,
        split,
        rows,
        document.longest_line(split),
        app.git_split_ratio,
    );
    editor_view::drag_scrollbars_with_metrics(app, body, code_left(body, split), metrics, x, y)
}

#[allow(clippy::too_many_arguments)]
fn toolbar(
    pane: PaneId,
    rect: UiRect,
    state: State<AppState>,
    editor_focus: UiFocusHandle,
    path: String,
    additions: usize,
    deletions: usize,
    split: bool,
    starts: std::sync::Arc<Vec<usize>>,
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
                    app.show_error(format!("Could not open file: {error}"));
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
                    app.workspace.activate_pane(pane);
                    app.workspace.set_scroll(pane, 0.0, 0.0);
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
                    app.workspace.activate_pane(pane);
                    app.workspace.set_scroll(pane, 0.0, 0.0);
                });
            },
        ),
    );

    let next_rect = take_button_rect(&mut right, button_top, 24.0);
    let next_state = state.clone();
    let next_starts = starts.clone();
    bar = bar.child(toolbar_button(next_rect, "↓", false).on_click(move || {
        next_state.update(|app| {
            app.workspace.activate_pane(pane);
            navigate_change(app, &next_starts, true);
        });
    }));

    let prev_rect = take_button_rect(&mut right, button_top, 24.0);
    let prev_state = state;
    let previous_starts = starts.clone();
    bar = bar.child(toolbar_button(prev_rect, "↑", false).on_click(move || {
        prev_state.update(|app| {
            app.workspace.activate_pane(pane);
            navigate_change(app, &previous_starts, false);
        });
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

fn render_split(
    rect: UiRect,
    rows: &[SplitDiffRow],
    scroll_x: f32,
    scroll_y: f32,
    ratio: f32,
) -> Element {
    let middle = split_middle(rect, ratio);
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
    let (display, _, columns) = editor_view::expand_tabs(value, 0);
    let width = (columns as f32 * theme::CHAR_W + 16.0).max(viewport.width());
    clip(viewport, 0.0, 0.0).child(text(
        UiRect::new(left, top, left + width, top + ROW_H),
        display,
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
    fn inline_diff_expands_tabs_before_rendering() {
        diff_expands_tabs_before_rendering(false);
    }

    #[test]
    fn split_diff_expands_tabs_before_rendering() {
        diff_expands_tabs_before_rendering(true);
    }

    fn diff_expands_tabs_before_rendering(split: bool) {
        use crate::git::{DiffTarget, diff::parse_unified};
        use crate::model::diff_document::DiffDocument;
        use lgui::application::AppView;
        use lgui::core::UiScale;
        use lgui::session::UiSession;
        use std::sync::Arc;

        let diff = parse_unified(
            "diff --git a/a.vue b/a.vue\n--- a/a.vue\n+++ b/a.vue\n@@ -1,2 +1,2 @@\n \tend\n-\t\t<div>\n+a\t中\tb\n",
        )
        .unwrap();
        let document = DiffDocument::from_unified(
            "repo".into(),
            "a.vue".into(),
            DiffTarget::IndexToWorktree,
            diff,
        );
        let viewport = UiRect::new(0.0, 0.0, 600.0, 100.0);
        let view: AppView = Arc::new(move |_| {
            if split {
                render_split(viewport, &document.split_rows, 0.0, 0.0, 0.5)
            } else {
                render_inline(viewport, &document.rows, 0.0, 0.0)
            }
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let rendered = session
            .tree()
            .nodes()
            .iter()
            .filter_map(|node| node.text.as_deref())
            .collect::<Vec<_>>();
        for expected in ["    end", "        <div>", "a   中  b"] {
            assert!(rendered.contains(&expected), "rendered text: {rendered:?}");
        }
        assert!(rendered.iter().all(|text| !text.contains('\t')));
    }

    #[test]
    fn inline_scrollbars_drag_and_release_outside_the_track() {
        scrollbars_drag_and_release(false);
    }

    #[test]
    fn split_scrollbars_drag_and_release_outside_the_track() {
        scrollbars_drag_and_release(true);
    }

    fn scrollbars_drag_and_release(split: bool) {
        use crate::git::DiffTarget;
        use crate::model::diff_document::DiffDocument;
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{
            InputEvent, Point, PointerButton, PointerData, UiEventKind, UiEventPayload, UiScale,
            dispatch_runtime_output,
        };
        use lgui::prelude::group;
        use lgui::session::UiSession;
        use std::sync::{Arc, Mutex};

        let exposed = Arc::new(Mutex::new(None::<State<AppState>>));
        let output = exposed.clone();
        let viewport = UiRect::new(0.0, 0.0, 500.0, 300.0);
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state_with(|| {
                let mut app = AppState::new();
                app.git_split_diff = split;
                app.workspace.open_diff(DiffDocument::added(
                    "test-repo".into(),
                    "long.txt".into(),
                    DiffTarget::HeadToWorktree,
                    &format!("{}\n", "x".repeat(200)).repeat(100),
                ));
                app
            });
            *output.lock().unwrap() = Some(state.clone());
            let id = cx.use_stable_id();
            let focus = cx.focus_handle(id);
            let drag_state = state.clone();
            let up_state = state.clone();
            group(viewport)
                .on_event_capture(UiEventKind::PointerMove, move |_, payload| {
                    if let UiEventPayload::PointerMove { pointer } = payload {
                        drag_state.try_update(|app| {
                            drag_scrollbars(app, viewport, pointer.point.x, pointer.point.y)
                        });
                    }
                })
                .on_event_capture(UiEventKind::PointerUp, move |_, _| {
                    up_state.try_update(editor_view::finish_scrollbar_drag);
                })
                .child(render(PaneId::new(1), viewport, state, focus))
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let state = exposed.lock().unwrap().clone().unwrap();
        let context = ApplicationContext::empty(Default::default());
        let mut send = |input| {
            let events = session.handle_input(input);
            dispatch_runtime_output(
                events,
                &context,
                &lgui::window::WindowId::new("diff-scrollbar-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
            session.render_view(&view, viewport, UiScale::ONE);
        };
        let pointer = |x, y| PointerData::mouse(Point::new(x, y));
        let body_top = TOOLBAR_H;

        if split {
            let body = UiRect::new(
                0.0,
                TOOLBAR_H,
                viewport.right - SCROLLBAR_SIZE,
                viewport.bottom - SCROLLBAR_SIZE,
            );
            let middle = split_middle(body, 0.5);
            send(InputEvent::PointerDown {
                pointer: pointer(middle, 80.0),
                button: PointerButton::Left,
            });
            assert_eq!(state.get().git_split_drag, Some(PaneId::new(1)));
            send(InputEvent::PointerMove(pointer(900.0, 80.0)));
            assert!(state.get().git_split_ratio > 0.5);
            assert!(
                (split_middle(body, state.get().git_split_ratio) - (body.right - 200.0)).abs()
                    < 0.01
            );
            send(InputEvent::PointerUp {
                pointer: pointer(900.0, 80.0),
                button: PointerButton::Left,
            });
            assert_eq!(state.get().git_split_drag, None);
            let ratio = state.get().git_split_ratio;
            send(InputEvent::PointerMove(pointer(20.0, 80.0)));
            assert_eq!(state.get().git_split_ratio, ratio);
        }

        // Grab the thumb, then move away from its strip and beyond the viewport.
        send(InputEvent::PointerDown {
            pointer: pointer(496.0, body_top + 4.0),
            button: PointerButton::Left,
        });
        assert!(state.get().editor_vertical_scrollbar_dragging);
        send(InputEvent::PointerMove(pointer(30.0, 500.0)));
        let bottom = state.get().workspace.active_scroll().1;
        assert!(bottom > 0.0);
        send(InputEvent::PointerUp {
            pointer: pointer(30.0, 500.0),
            button: PointerButton::Left,
        });
        assert!(!state.get().editor_vertical_scrollbar_dragging);
        send(InputEvent::PointerMove(pointer(30.0, body_top)));
        assert_eq!(state.get().workspace.active_scroll().1, bottom);

        // Clicking the track jumps toward the pointer and starts a new drag.
        send(InputEvent::PointerDown {
            pointer: pointer(499.0, body_top + 40.0),
            button: PointerButton::Left,
        });
        assert!(state.get().workspace.active_scroll().1 < bottom);
        send(InputEvent::PointerUp {
            pointer: pointer(499.0, body_top + 40.0),
            button: PointerButton::Left,
        });

        let left = code_left(viewport, split);
        send(InputEvent::PointerDown {
            pointer: pointer(left + 4.0, 296.0),
            button: PointerButton::Left,
        });
        assert!(state.get().editor_horizontal_scrollbar_dragging);
        send(InputEvent::PointerMove(pointer(900.0, 100.0)));
        let right = state.get().workspace.active_scroll().0;
        assert!(right > 0.0);
        send(InputEvent::PointerUp {
            pointer: pointer(900.0, 100.0),
            button: PointerButton::Left,
        });
        assert!(!state.get().editor_horizontal_scrollbar_dragging);
        send(InputEvent::PointerMove(pointer(left, 100.0)));
        assert_eq!(state.get().workspace.active_scroll().0, right);
        send(InputEvent::PointerDown {
            pointer: pointer(left + 30.0, 299.0),
            button: PointerButton::Left,
        });
        assert!(state.get().workspace.active_scroll().0 < right);
        send(InputEvent::PointerUp {
            pointer: pointer(left + 30.0, 299.0),
            button: PointerButton::Left,
        });
    }

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
