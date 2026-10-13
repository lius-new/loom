//! Read-only inline and side-by-side diff editor.

use std::path::PathBuf;

use lgui::core::{
    CursorIcon, EventPolicy, IconStyle, SemanticRole, Semantics, UiElement, UiFocusHandle, UiId,
    WheelUnit, clip, precompiled,
};
use lgui::prelude::{Element, State, TextAlign, UiRect, VisualStyle, panel, text};

use crate::app::git::BlockAction;
use crate::features::git::diff_document::{
    ChangesLayout, DiffDocument, DiffRow, DiffRowKind, SplitDiffRow,
};
use crate::features::git::{DiffTarget, GitStoreSnapshot};
use crate::fonts;
use crate::model::pane_layout::PaneId;
use crate::state::AppState;
use crate::theme;
use crate::ui::editor::view::{self as editor_view, SCROLLBAR_SIZE, ScrollMetrics};

const TOOLBAR_H: f32 = 34.0;
const NUMBER_W: f32 = 44.0;
const MARKER_W: f32 = 20.0;
const BUTTON_H: f32 = 22.0;
const BUTTON_GAP: f32 = 4.0;

/// Height of one diff row; rows grow with the editor font size.
pub(crate) fn row_h() -> f32 {
    crate::features::git::diff_document::row_height_for(crate::fonts::editor().size)
}

/// Render the diff shown by `pane` into `rect`.
pub fn render(
    pane: PaneId,
    rect: UiRect,
    state: State<AppState>,
    store: State<GitStoreSnapshot>,
    editor_focus: UiFocusHandle,
    scrollbars: crate::ui::editor_binding::Binding,
) -> Element {
    let app = state.get();
    let Some(document) = app.workspace.diff(pane) else {
        return panel(rect, VisualStyle::filled(theme::c().bg));
    };

    let split = app.git.ui.split_diff;
    let collapsed = app.workspace.collapsed_diffs(pane);
    let overview = document
        .sections
        .as_ref()
        .map(|_| document.changes_layout(split, &collapsed));
    let starts = overview.as_ref().map_or_else(
        || document.change_starts(split),
        |layout| layout.starts.clone(),
    );
    let source_label = document.source_label();
    let absolute_path = document
        .sections
        .is_none()
        .then(|| document.absolute_path());
    let path_label = document.sections.as_ref().map_or_else(
        || document.path.display().to_string(),
        |_| {
            let files = document.changed_file_count();
            match &document.revision {
                Some(revision) => format!(
                    "Commit {} · {files} files",
                    crate::features::git::diff_document::short_oid(revision)
                ),
                None => format!("All Changes · {files} files"),
            }
        },
    );
    let additions = document.additions();
    let deletions = document.deletions();
    let (stored_x, stored_y) = app.workspace.scroll(pane);
    let current_line = (stored_y / row_h()).floor() as usize;
    let current_change = current_change_number(&starts, current_line);

    let body_top = rect.top + TOOLBAR_H;
    let body_rect = UiRect::new(rect.left, body_top, rect.right, rect.bottom);
    let row_count = if let Some(overview) = &overview {
        overview.row_count
    } else if split {
        document.split_rows.len()
    } else {
        document.rows.len()
    };
    let metrics = scroll_metrics(
        body_rect,
        split,
        row_count,
        document.longest_line(split),
        app.editor.diff_split.ratio,
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
                WheelUnit::Lines => (delta.x * row_h() * 3.0, delta.y * row_h() * 3.0),
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
        editor_focus.clone(),
        path_label,
        additions,
        deletions,
        split,
        starts.clone(),
        current_change,
        absolute_path,
    ));

    if row_count == 0 {
        if overview.is_some() {
            return root.child(text(
                UiRect::new(
                    body_rect.left + 24.0,
                    body_rect.top + 24.0,
                    body_rect.right - 24.0,
                    body_rect.top + 64.0,
                ),
                "No changes in this repository.",
                theme::mono(theme::c().text_dim, theme::UI_SIZE),
            ));
        }
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

    // Rows run under the overlay scrollbars, whose tracks are transparent.
    root = if let Some(overview) = &overview {
        root.child(render_changes(
            pane,
            body_rect,
            document,
            overview,
            split,
            scroll_x,
            scroll_y,
            app.editor.diff_split.ratio,
            &state,
            &store,
            editor_focus,
        ))
    } else if split {
        root.child(render_split(
            body_rect,
            &document.split_rows,
            scroll_x,
            scroll_y,
            app.editor.diff_split.ratio,
        ))
    } else {
        root.child(render_inline(body_rect, &document.rows, scroll_x, scroll_y))
    };

    if !document.blocks.is_empty() {
        root = root.child(block_buttons(
            body_rect, document, &starts, scroll_y, &state, &store,
        ));
    }

    if split {
        let down = state.clone();
        let moving = state.clone();
        let up = state.clone();
        root = root.child(crate::ui::pane_sash::horizontal(
            format!("git-split-{}", pane.get()),
            split_middle(body_rect, app.editor.diff_split.ratio),
            (body_rect.top, body_rect.bottom),
            app.editor.diff_split.drag == Some(pane),
            move || down.update(|app| app.editor.diff_split.drag = Some(pane)),
            move |x| {
                moving.try_update(|app| {
                    if app.editor.diff_split.drag != Some(pane) || body_rect.width() <= 0.0 {
                        return false;
                    }
                    let position =
                        crate::ui::pane_sash::split_position(body_rect.left, body_rect.right, x);
                    app.editor.diff_split.ratio = (position - body_rect.left) / body_rect.width();
                    true
                });
            },
            move || up.update(|app| app.editor.diff_split.drag = None),
        ));
    }

    let emphasized = app.editor.editor_hovered == Some(pane)
        || (app.workspace.active_pane() == pane
            && (app.editor.editor_vertical_scrollbar_dragging
                || app.editor.editor_horizontal_scrollbar_dragging));
    if !split && document.sections.is_none() {
        let marks = document.inline_change_marks();
        if !marks.is_empty() && marks.len() <= 10_000 {
            root = root.child(editor_view::overview_ruler(
                body_rect,
                metrics,
                max_x > 0.0,
                row_h(),
                marks.iter().copied(),
            ));
        }
    }
    if max_y > 0.0 {
        root = root.child(editor_view::vertical_scrollbar(
            pane,
            body_rect,
            metrics,
            scroll_y,
            max_x > 0.0,
            emphasized,
            scrollbars.clone(),
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
            scrollbars.clone(),
        ));
    }
    if max_x > 0.0 && max_y > 0.0 {
        root = root.child(editor_view::scrollbar_corner(pane, body_rect));
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
    let content_h = rows as f32 * row_h();
    let content_w = longest_line as f32 * fonts::editor().char_w + theme::CODE_PAD * 2.0;
    let mut viewport_h = rect.height().max(1.0);
    let mut viewport_w = 1.0;
    for _ in 0..3 {
        // Columns span the full width; only the text under the vertical
        // scrollbar must still scroll into view.
        let reserved = if content_h > viewport_h {
            SCROLLBAR_SIZE
        } else {
            0.0
        };
        viewport_w = (if split {
            let middle = split_middle(rect, ratio);
            (middle - rect.left).min(rect.right - middle - reserved) - NUMBER_W
        } else {
            rect.width() - reserved - NUMBER_W * 2.0 - MARKER_W
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
    if !app.editor.editor_vertical_scrollbar_dragging
        && !app.editor.editor_horizontal_scrollbar_dragging
    {
        return false;
    }
    let split = app.git.ui.split_diff;
    let Some(document) = app.workspace.diff(app.workspace.active_pane()) else {
        return crate::app::editor::finish_scrollbar_drag(app);
    };
    let body = UiRect::new(rect.left, rect.top + TOOLBAR_H, rect.right, rect.bottom);
    let rows = if document.sections.is_some() {
        document
            .changes_layout(
                split,
                &app.workspace.collapsed_diffs(app.workspace.active_pane()),
            )
            .row_count
    } else if split {
        document.split_rows.len()
    } else {
        document.rows.len()
    };
    let metrics = scroll_metrics(
        body,
        split,
        rows,
        document.longest_line(split),
        app.editor.diff_split.ratio,
    );
    crate::app::editor::drag_scrollbars_with_metrics(
        app,
        body,
        code_left(body, split),
        metrics,
        x,
        y,
    )
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
    absolute_path: Option<PathBuf>,
) -> Element {
    let mut bar = panel(rect, VisualStyle::filled(theme::c().sidebar));
    bar = bar.child(panel(
        UiRect::new(rect.left, rect.bottom - 1.0, rect.right, rect.bottom),
        VisualStyle::filled(theme::c().border),
    ));

    let compact = rect.width() < 560.0;
    let button_top = rect.top + (rect.height() - BUTTON_H) / 2.0;
    let mut right = rect.right - 8.0;

    if let Some(absolute_path) = absolute_path {
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
    }

    let split_rect = take_button_rect(&mut right, button_top, if compact { 28.0 } else { 42.0 });
    let split_state = state.clone();
    bar = bar.child(
        toolbar_button(split_rect, if compact { "S" } else { "Split" }, split).on_click(
            move || {
                split_state.update(|app| {
                    app.git.ui.split_diff = true;
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
                    app.git.ui.split_diff = false;
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
            path_rect.left + path.chars().count() as f32 * theme::UI_CHAR_W + 12.0,
            path_rect.bottom,
        ),
        path,
        theme::mono(theme::c().text_soft, theme::UI_SIZE),
    )))
}

/// Stage, unstage or discard buttons at the right of each change block's
/// first row. `starts` lists those rows in block order.
fn block_buttons(
    rect: UiRect,
    document: &DiffDocument,
    starts: &[usize],
    scroll_y: f32,
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
) -> Element {
    let actions: &[(BlockAction, &str)] = match document.target {
        DiffTarget::HeadToIndex => &[(BlockAction::Unstage, "Unstage")],
        _ => &[
            (BlockAction::Discard, "Discard"),
            (BlockAction::Stage, "Stage"),
        ],
    };
    let mut layer = clip(rect, 0.0, 0.0);
    for (&row, &block) in starts.iter().zip(&document.blocks) {
        let top = rect.top + row as f32 * row_h() - scroll_y;
        if top + row_h() < rect.top || top > rect.bottom {
            continue;
        }
        let mut right = rect.right - SCROLLBAR_SIZE - 6.0;
        for &(action, label) in actions.iter().rev() {
            let width = super::tabs::measure(label, theme::SMALL, 400) + 14.0;
            let button = UiRect::new(right - width, top + 2.0, right, top + row_h() - 2.0);
            right = button.left - 4.0;
            let click_state = state.clone();
            let click_store = store.clone();
            let root = document.repository_root.clone();
            let path = document.path.clone();
            let target = document.target;
            let mut style = theme::mono(theme::c().text_soft, theme::SMALL);
            style.align = TextAlign::Center;
            layer = layer.child(
                theme::bordered(button, theme::c().surface, theme::c().border, 3.0, 1.0)
                    .key(format!("diff-block-{row}-{label}"))
                    .event_policy(EventPolicy::INTERACTIVE)
                    .cursor(CursorIcon::Pointer)
                    .on_click(move |cx: &mut lgui::core::UiEventContext| {
                        if action == BlockAction::Discard {
                            crate::app::git::request_discard_block(
                                &click_state,
                                root.clone(),
                                path.clone(),
                                block,
                            );
                        } else {
                            crate::app::git::apply_block(
                                &click_state,
                                &click_store,
                                &root,
                                path.clone(),
                                target,
                                block,
                                action,
                            );
                        }
                        cx.stop_propagation();
                    })
                    .child(text(button, label, style)),
            );
        }
    }
    layer
}

/// Build only visible file sections; the existing inline/split renderers
/// further limit each visible section to the rows inside the viewport.
#[allow(clippy::too_many_arguments)]
fn render_changes(
    pane: PaneId,
    rect: UiRect,
    overview: &DiffDocument,
    layout: &ChangesLayout,
    split: bool,
    scroll_x: f32,
    scroll_y: f32,
    ratio: f32,
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    editor_focus: UiFocusHandle,
) -> Element {
    let sections = overview.sections.as_ref().unwrap();
    let mut body = clip(rect, 0.0, 0.0);
    let first = layout
        .sections
        .partition_point(|item| (item.body_top + item.body_rows) as f32 * row_h() < scroll_y);
    for item in layout.sections.iter().skip(first) {
        let top = rect.top + item.top as f32 * row_h() - scroll_y;
        if top - row_h() > rect.bottom {
            break;
        }
        let section = &sections[item.index];
        let document = &section.document;
        if item.group_header {
            body = body.child(text(
                UiRect::new(rect.left + 12.0, top - row_h(), rect.right - 12.0, top),
                section.group.label(),
                theme::mono_bold(theme::c().text_dim, theme::SMALL),
            ));
        }
        let header = UiRect::new(rect.left, top, rect.right, top + row_h() * 2.0);
        let toggle_state = state.clone();
        let request = section.request();
        let toggle_request = request.clone();
        let collapsed = item.body_rows == 0;
        let action_w = if rect.width() < 360.0 { 66.0 } else { 100.0 };
        let action_rect = UiRect::new(
            rect.right - SCROLLBAR_SIZE - action_w - 8.0,
            top + 9.0,
            rect.right - SCROLLBAR_SIZE - 8.0,
            top + 31.0,
        );
        let stats_right = action_rect.left - 10.0;
        let stats_w = if rect.width() < 460.0 { 0.0 } else { 96.0 };
        let label_rect = UiRect::new(
            rect.left + 32.0,
            top,
            (stats_right - stats_w).max(rect.left + 32.0),
            header.bottom,
        );
        let path = super::tabs::ellipsize(
            &super::display_path(&document.path),
            (label_rect.width() - 6.0).max(0.0),
            theme::UI_SIZE,
            400,
        );
        let mut file_header = panel(header, VisualStyle::filled(theme::c().sidebar))
            .key(format!("changes-header-{}", item.index))
            .event_policy(EventPolicy::INTERACTIVE)
            .semantics(Semantics::new(SemanticRole::Button).name(format!(
                "{} {}",
                if collapsed { "Expand" } else { "Collapse" },
                document.path.display()
            )))
            .cursor(CursorIcon::Pointer)
            .on_click(move || {
                toggle_state.update(|app| {
                    app.workspace.activate_pane(pane);
                    app.workspace
                        .toggle_diff_section(pane, toggle_request.clone());
                })
            })
            .child(panel(
                UiRect::new(
                    header.left,
                    header.bottom - 1.0,
                    header.right,
                    header.bottom,
                ),
                VisualStyle::filled(theme::c().border),
            ))
            .child(text(
                label_rect,
                path,
                theme::mono(theme::c().text, theme::UI_SIZE),
            ));
        file_header = file_header.child(precompiled(
            UiElement::icon(
                UiId::owned(format!("changes-chevron-{}", item.index)),
                theme::icon_rect(UiRect::new(
                    rect.left + 8.0,
                    top + 12.0,
                    rect.left + 24.0,
                    top + 28.0,
                )),
                if collapsed {
                    "chevron-right"
                } else {
                    "chevron-down"
                },
            )
            .icon_style(IconStyle::new(theme::c().text_dim)),
        ));
        if stats_w > 0.0 {
            let stats = if document.binary {
                "Binary".into()
            } else {
                format!("+{}  −{}", document.additions(), document.deletions())
            };
            file_header = file_header.child(text(
                UiRect::new(stats_right - stats_w, top, stats_right, header.bottom),
                stats,
                theme::mono_right(theme::c().text_dim, theme::SMALL),
            ));
        }
        let open_state = state.clone();
        let focus = editor_focus.clone();
        let untracked = section.group == crate::features::git::ChangeGroup::Untracked;
        let revision = overview.revision.clone();
        file_header = file_header.child(
            toolbar_button(
                action_rect,
                if rect.width() < 360.0 {
                    "Changes"
                } else {
                    "Open Changes"
                },
                false,
            )
            .semantics(
                Semantics::new(SemanticRole::Button)
                    .name(format!("Open Changes for {}", document.path.display())),
            )
            .on_click(move |cx: &mut lgui::core::UiEventContext| {
                cx.stop_propagation();
                match &revision {
                    Some(revision) => crate::app::git::open_commit_diff(
                        &open_state,
                        request.repository_root.clone(),
                        revision.clone(),
                        request.path.clone(),
                    ),
                    None => crate::app::git::open_diff(&open_state, request.clone(), untracked),
                }
                focus.focus();
            }),
        );
        body = body.child(file_header);
        if collapsed {
            continue;
        }
        let body_top = rect.top + item.body_top as f32 * row_h() - scroll_y;
        let body_bottom = body_top + item.body_rows as f32 * row_h();
        let visible = UiRect::new(
            rect.left,
            body_top.max(rect.top),
            rect.right,
            body_bottom.min(rect.bottom),
        );
        if visible.bottom <= visible.top {
            continue;
        }
        let local_y = visible.top - body_top;
        let mut file_body =
            panel(visible, VisualStyle::default()).key(format!("changes-body-{}", item.index));
        if document.rows.is_empty() {
            let message = section.error.clone().unwrap_or_else(|| {
                if document.binary {
                    "Binary file changed.".into()
                } else {
                    "No textual changes.".into()
                }
            });
            file_body = file_body.child(text(
                UiRect::new(rect.left + 32.0, body_top, rect.right - 12.0, body_bottom),
                message,
                theme::mono(theme::c().text_dim, theme::SMALL),
            ));
        } else {
            file_body = file_body.child(if split {
                render_split(visible, &document.split_rows, scroll_x, local_y, ratio)
            } else {
                render_inline(visible, &document.rows, scroll_x, local_y)
            });
            if !document.blocks.is_empty() {
                file_body = file_body.child(block_buttons(
                    visible,
                    document,
                    &document.change_starts(split),
                    local_y,
                    state,
                    store,
                ));
            }
        }
        body = body.child(file_body);
    }
    body
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
    let start = (scroll_y / row_h()).floor() as usize;
    let end = (((scroll_y + rect.height()) / row_h()).ceil() as usize + 1).min(rows.len());
    let mut body = clip(rect, 0.0, 0.0);

    for (index, row) in rows.iter().enumerate().take(end).skip(start) {
        let top = rect.top + index as f32 * row_h() - scroll_y;
        let row_rect = UiRect::new(rect.left, top, rect.right, top + row_h());
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
                UiRect::new(rect.left + 10.0, top, rect.right - 10.0, top + row_h()),
                row.text.clone(),
                theme::mono(theme::c().text_dim, theme::SMALL),
            ));
            continue;
        }

        body = body
            .child(line_number(rect.left, top, row.old_line))
            .child(line_number(rect.left + NUMBER_W, top, row.new_line))
            .child(text(
                UiRect::new(rect.left + NUMBER_W * 2.0, top, text_left, top + row_h()),
                marker,
                fonts::editor().style(foreground),
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
    let start = (scroll_y / row_h()).floor() as usize;
    let end = (((scroll_y + rect.height()) / row_h()).ceil() as usize + 1).min(rows.len());
    let mut body = clip(rect, 0.0, 0.0);

    for (index, row) in rows.iter().enumerate().take(end).skip(start) {
        let top = rect.top + index as f32 * row_h() - scroll_y;
        if let Some(hunk) = row.hunk.as_ref() {
            body = body
                .child(panel(
                    UiRect::new(rect.left, top, rect.right, top + row_h()),
                    VisualStyle::filled(theme::c().surface),
                ))
                .child(text(
                    UiRect::new(rect.left + 10.0, top, rect.right - 10.0, top + row_h()),
                    hunk.clone(),
                    theme::mono(theme::c().text_dim, theme::SMALL),
                ));
            continue;
        }

        if row.changed && row.old_text.is_some() {
            body = body.child(panel(
                UiRect::new(rect.left, top, middle, top + row_h()),
                VisualStyle::filled(theme::c().diff.del_bg),
            ));
        }
        if row.changed && row.new_text.is_some() {
            body = body.child(panel(
                UiRect::new(middle, top, rect.right, top + row_h()),
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
        UiRect::new(left, top, left + NUMBER_W - 8.0, top + row_h()),
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
    let width = (columns as f32 * fonts::editor().char_w + 16.0).max(viewport.width());
    clip(viewport, 0.0, 0.0).child(text(
        UiRect::new(left, top, left + width, top + row_h()),
        display,
        fonts::editor().style(color),
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
    let current = (y / row_h()).floor() as usize;
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
    app.workspace.set_active_scroll(x, target as f32 * row_h());
}

#[cfg(test)]
mod tests {
    use super::*;
    fn render(
        pane: PaneId,
        rect: UiRect,
        state: State<AppState>,
        store: State<GitStoreSnapshot>,
        focus: UiFocusHandle,
    ) -> Element {
        let binding = crate::app::editor_view::binding(state.clone(), store.clone(), pane, rect);
        super::render(pane, rect, state, store, focus, binding)
    }

    fn sample_overview(extra_files: usize) -> DiffDocument {
        use crate::features::git::{
            ChangeDiff, ChangeGroup, DiffContent, DiffRequest, diff::parse_unified,
        };
        let file = |path: &str, group, target, patch: &str| ChangeDiff {
            path: path.into(),
            group,
            target,
            content: Ok(DiffContent::Unified(parse_unified(patch).unwrap())),
        };
        let mut files = vec![
            file(
                "src/app.rs",
                ChangeGroup::Staged,
                DiffTarget::HeadToIndex,
                "diff --git a/src/app.rs b/src/app.rs\n--- a/src/app.rs\n+++ b/src/app.rs\n@@ -1,3 +1,3 @@\n pub fn render() {\n-    open_diff(file);\n+    open_changes(repository, file);\n }\n",
            ),
            file(
                "src/ui/git_panel.rs",
                ChangeGroup::Unstaged,
                DiffTarget::IndexToWorktree,
                "diff --git a/src/ui/git_panel.rs b/src/ui/git_panel.rs\n--- a/src/ui/git_panel.rs\n+++ b/src/ui/git_panel.rs\n@@ -1,3 +1,3 @@\n fn on_click() {\n-    show_file_diff();\n+    show_all_changes();\n }\n",
            ),
            ChangeDiff {
                path: "notes/review.md".into(),
                group: ChangeGroup::Untracked,
                target: DiffTarget::IndexToWorktree,
                content: Ok(DiffContent::Untracked(
                    "# Review notes\n\nOne Changes tab for the whole repository.\n".into(),
                )),
            },
        ];
        files.extend((0..extra_files).map(|index| ChangeDiff {
            path: format!("generated/file-{index:04}.txt").into(),
            group: ChangeGroup::Untracked,
            target: DiffTarget::IndexToWorktree,
            content: Ok(DiffContent::Untracked("first\nsecond\nthird\n".into())),
        }));
        DiffDocument::from_content(
            DiffRequest {
                repository_root: "loom".into(),
                path: PathBuf::new(),
                target: DiffTarget::Changes,
            },
            DiffContent::Changes(files),
        )
    }

    #[test]
    fn changes_view_collapses_files_and_virtualizes_inline_and_split_sections() {
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{
            InputEvent, Point, PointerButton, PointerData, UiScale, dispatch_runtime_output,
        };
        use lgui::session::UiSession;
        use std::sync::{Arc, Mutex};
        for split in [false, true] {
            let exposed = Arc::new(Mutex::new(None::<State<AppState>>));
            let output = exposed.clone();
            let viewport = UiRect::new(0.0, 0.0, 900.0, 600.0);
            let view: AppView = Arc::new(move |cx| {
                let state = cx.state_with(|| {
                    let mut app = AppState::new();
                    app.git.ui.split_diff = split;
                    app.workspace.open_diff(sample_overview(0));
                    app
                });
                *output.lock().unwrap() = Some(state.clone());
                let store = cx.state_with(GitStoreSnapshot::default);
                let id = cx.use_stable_id();
                let focus = cx.focus_handle(id);
                render(
                    state.get().workspace.active_pane(),
                    viewport,
                    state,
                    store,
                    focus,
                )
            });
            let context = ApplicationContext::empty(Default::default());
            let _text = lgui::backend::install_text_environment(
                &context,
                lgui::render_skia::skia_text_system_handle(),
            );
            let mut session = UiSession::new();
            session.render_view(&view, viewport, UiScale::ONE);
            let state = exposed.lock().unwrap().clone().unwrap();
            assert!(!session.tree().nodes().iter().any(|node| {
                node.layout_rect.left == viewport.right - SCROLLBAR_SIZE + 1.0
                    && node.layout_rect.width() == 4.0
                    && matches!(node.style.fill, Some(color) if color == theme::c().diff.add_mark
                        || color == theme::c().diff.del_mark)
            }), "All Changes must not show overview marks");
            let texts = || {
                session
                    .tree()
                    .nodes()
                    .iter()
                    .filter_map(|node| node.text.as_deref())
                    .collect::<Vec<_>>()
            };
            for expected in [
                "STAGED CHANGES",
                "CHANGES",
                "UNTRACKED FILES",
                "src/app.rs",
                "src/ui/git_panel.rs",
                "notes/review.md",
            ] {
                assert!(texts().contains(&expected), "Missing {expected}");
            }
            if !split && let Some(path) = std::env::var_os("LOOM_CHANGES_PREVIEW") {
                use lgui::core::PhysicalRect;
                use lgui::icons::IconRegistration;
                use lgui::renderer::{FrameInfo, FrameReason};
                context
                    .resources()
                    .provide(IconRegistration(Arc::new(crate::svg_icons())));
                let scale = UiScale::new(2.0);
                let mut surface = lgui::render_skia::SkiaSoftwareSurface::new(12 * 1024 * 1024);
                let frame = FrameInfo::new(
                    PhysicalRect::new(0, 0, 1800, 1200),
                    &[],
                    scale,
                    FrameReason::SceneChange,
                    true,
                );
                lgui::assets::backend::with_render_resources(
                    &context,
                    lgui::RenderResources::new(),
                    || {
                        surface
                            .draw(&session.tree().scene().project_to_physical(scale), &frame)
                            .unwrap();
                    },
                );
                let mut pixels = surface.pixels().to_vec();
                for pixel in pixels.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
                image::save_buffer(path, &pixels, 1800, 1200, image::ColorType::Rgba8).unwrap();
            }
            let header = session
                .tree()
                .nodes()
                .iter()
                .find(|node| {
                    node.semantics.as_ref().and_then(|s| s.name.as_deref())
                        == Some("Collapse src/app.rs")
                })
                .unwrap()
                .layout_rect;
            let point = Point::new(header.left + 48.0, (header.top + header.bottom) / 2.0);
            for input in [
                InputEvent::PointerDown {
                    pointer: PointerData::mouse(point),
                    button: PointerButton::Left,
                },
                InputEvent::PointerUp {
                    pointer: PointerData::mouse(point),
                    button: PointerButton::Left,
                },
            ] {
                let events = session.handle_input(input);
                dispatch_runtime_output(
                    events,
                    &context,
                    &lgui::window::WindowId::new("changes-view-test"),
                    |action| session.handle_default_action(action),
                    |_| {},
                );
                session.render_view(&view, viewport, UiScale::ONE);
            }
            let app = state.get();
            let collapsed = app.workspace.collapsed_diffs(app.workspace.active_pane());
            assert_eq!(collapsed.len(), 1);
            assert_eq!(
                collapsed.iter().next().unwrap().target,
                DiffTarget::HeadToIndex
            );
            state.update(|app| {
                app.workspace.open_diff(sample_overview(1000));
                app.workspace.set_active_scroll(0.0, 100_000.0);
            });
            session.render_view(&view, viewport, UiScale::ONE);
            let texts = session
                .tree()
                .nodes()
                .iter()
                .filter_map(|node| node.text.as_deref())
                .collect::<Vec<_>>();
            assert!(texts.contains(&"generated/file-0999.txt"));
            assert!(!texts.contains(&"src/app.rs"));
            assert!(
                session.tree().nodes().len() < 600,
                "Offscreen files must not create UI nodes"
            );
        }
    }

    #[test]
    fn inline_overview_draws_added_and_deleted_display_rows_without_scrolling() {
        use crate::features::git::diff::parse_unified;
        use lgui::application::AppView;
        use lgui::core::UiScale;
        use lgui::session::UiSession;
        use std::sync::Arc;

        // Source line 10 appears at display row 1 after the hunk header.
        let document = DiffDocument::from_unified(
            "repo".into(),
            "a.txt".into(),
            DiffTarget::IndexToWorktree,
            parse_unified(
                "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -10,3 +12,3 @@\n-old\n-older\n+new\n+newer\n same\n",
            )
            .unwrap(),
        );
        let viewport = UiRect::new(0.0, 0.0, 600.0, 300.0);
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state_with(|| {
                let mut app = AppState::new();
                app.git.ui.split_diff = false;
                app.workspace.open_diff(document.clone());
                app
            });
            let store = cx.state_with(GitStoreSnapshot::default);
            let id = cx.use_stable_id();
            render(
                state.get().workspace.active_pane(),
                viewport,
                state,
                store,
                cx.focus_handle(id),
            )
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let marks = session
            .tree()
            .nodes()
            .iter()
            .filter(|node| {
                node.layout_rect.left == viewport.right - SCROLLBAR_SIZE + 1.0
                    && node.layout_rect.width() == 4.0
                    && node.style.fill.is_some()
            })
            .map(|node| (node.layout_rect, node.style.fill.unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(
            marks,
            [
                (
                    UiRect::new(589.0, TOOLBAR_H + row_h(), 593.0, TOOLBAR_H + 3.0 * row_h()),
                    theme::c().diff.del_mark,
                ),
                (
                    UiRect::new(
                        589.0,
                        TOOLBAR_H + 3.0 * row_h(),
                        593.0,
                        TOOLBAR_H + 5.0 * row_h()
                    ),
                    theme::c().diff.add_mark,
                ),
            ]
        );
    }

    #[test]
    fn inline_diff_expands_tabs_before_rendering() {
        diff_expands_tabs_before_rendering(false);
    }

    #[test]
    fn split_diff_expands_tabs_before_rendering() {
        diff_expands_tabs_before_rendering(true);
    }

    fn diff_expands_tabs_before_rendering(split: bool) {
        use crate::features::git::diff_document::DiffDocument;
        use crate::features::git::{DiffTarget, diff::parse_unified};
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
        use crate::features::git::DiffTarget;
        use crate::features::git::diff_document::DiffDocument;
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
                app.git.ui.split_diff = split;
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
                    up_state.try_update(crate::app::editor::finish_scrollbar_drag);
                })
                .child(render(
                    PaneId::new(1),
                    viewport,
                    state,
                    cx.state(crate::features::git::GitStoreSnapshot::default()),
                    focus,
                ))
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
            let body = UiRect::new(0.0, TOOLBAR_H, viewport.right, viewport.bottom);
            let middle = split_middle(body, 0.5);
            send(InputEvent::PointerDown {
                pointer: pointer(middle, 80.0),
                button: PointerButton::Left,
            });
            assert_eq!(state.get().editor.diff_split.drag, Some(PaneId::new(1)));
            send(InputEvent::PointerMove(pointer(900.0, 80.0)));
            assert!(state.get().editor.diff_split.ratio > 0.5);
            assert!(
                (split_middle(body, state.get().editor.diff_split.ratio) - (body.right - 200.0))
                    .abs()
                    < 0.01
            );
            send(InputEvent::PointerUp {
                pointer: pointer(900.0, 80.0),
                button: PointerButton::Left,
            });
            assert_eq!(state.get().editor.diff_split.drag, None);
            let ratio = state.get().editor.diff_split.ratio;
            send(InputEvent::PointerMove(pointer(20.0, 80.0)));
            assert_eq!(state.get().editor.diff_split.ratio, ratio);
        }

        // Grab the thumb, then move away from its strip and beyond the viewport.
        send(InputEvent::PointerDown {
            pointer: pointer(496.0, body_top + 4.0),
            button: PointerButton::Left,
        });
        assert!(state.get().editor.editor_vertical_scrollbar_dragging);
        send(InputEvent::PointerMove(pointer(30.0, 500.0)));
        let bottom = state.get().workspace.active_scroll().1;
        assert!(bottom > 0.0);
        send(InputEvent::PointerUp {
            pointer: pointer(30.0, 500.0),
            button: PointerButton::Left,
        });
        assert!(!state.get().editor.editor_vertical_scrollbar_dragging);
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
        assert!(state.get().editor.editor_horizontal_scrollbar_dragging);
        send(InputEvent::PointerMove(pointer(900.0, 100.0)));
        let right = state.get().workspace.active_scroll().0;
        assert!(right > 0.0);
        send(InputEvent::PointerUp {
            pointer: pointer(900.0, 100.0),
            button: PointerButton::Left,
        });
        assert!(!state.get().editor.editor_horizontal_scrollbar_dragging);
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
        assert_eq!(app.workspace.active_scroll().1, 2.0 * row_h());
        app.workspace.set_active_scroll(0.0, 8.0 * row_h());
        navigate_change(&mut app, &[2, 8], true);
        assert_eq!(app.workspace.active_scroll().1, 2.0 * row_h());
        navigate_change(&mut app, &[2, 8], false);
        assert_eq!(app.workspace.active_scroll().1, 8.0 * row_h());
    }
}
