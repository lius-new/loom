//! The central code viewport: gutter, syntax-highlighted source, cursor and
//! editor-specific overlay scrollbars.

use std::collections::BTreeMap;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use lgui::core::{
    CursorIcon, EventPolicy, KeyState, PointerButton, SemanticRole, Semantics, UiElement,
    UiFocusHandle, UiId, WheelUnit, clip,
};
use lgui::prelude::{Element, State, UiRect, VisualStyle, group, panel, text};
use lgui::text::{self, TextLayout, TextLayoutRequest};

use super::commands::{self, Command};
use super::interaction::{DragSelection, ImeSession, SelectionUnit};
use crate::editor::gutter;
use crate::editor::normal;
use crate::editor::syntax;
use crate::git::{GitStoreSnapshot, LineChange};
use crate::model::pane_layout::PaneId;
use crate::model::text::TextBuffer;
use crate::state::AppState;
use crate::theme;
use crate::vim::{self, CursorShape};

use super::vim_input;

pub(crate) use crate::ui::components::scrollbar::SIZE as SCROLLBAR_SIZE;
use crate::ui::components::scrollbar::{
    INSET as SCROLLBAR_INSET, MIN_THUMB_LENGTH, thumb as scrollbar_thumb, track as scrollbar_track,
};
const TRAILING_CODE_SPACE: f32 = 32.0;
const CURSOR_REVEAL_MARGIN: f32 = 12.0;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ScrollMetrics {
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub content_w: f32,
    pub content_h: f32,
    pub max_x: f32,
    pub max_y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ThumbGeometry {
    start: f32,
    length: f32,
}

/// Render the editor of `pane`'s active file into `rect`.
pub fn render(
    pane: PaneId,
    rect: UiRect,
    state: State<AppState>,
    git_store: State<GitStoreSnapshot>,
    editor_id: UiId,
    editor_focus: UiFocusHandle,
) -> Element {
    let s = state.get();
    let git = git_store.get();
    let focused_pane = s.workspace.active_pane() == pane;

    let ime_rect = ime_cursor_rect(&s, rect);
    // Vim's Normal and Visual modes take keys as commands, so the input
    // method stays off until a mode accepts text.
    let role = if s.vim.is_enabled() && !s.vim.accepts_text() {
        SemanticRole::Group
    } else {
        SemanticRole::TextInput
    };
    let mut root = Element::new(move |cx| {
        UiElement::panel(editor_id, rect, VisualStyle::filled(theme::c().bg))
            .ime_cursor_rect(ime_rect)
            .semantics(Semantics::new(role).name("Code editor"))
            .children(cx.children)
    })
    .event_policy(EventPolicy::INTERACTIVE);

    if let Some(id) = s.workspace.pane(pane).and_then(|p| p.active()) {
        let meta = s
            .workspace
            .meta(id)
            .expect("active document metadata exists");
        let buffer = s.workspace.editor(pane).expect("pane shows a text file");
        let decorations = git
            .line_changes
            .get(&meta.path)
            .map(|changes| {
                changes
                    .iter()
                    .map(|(line, change)| {
                        let decoration = match change {
                            LineChange::Added => gutter::GutterDecoration::GitAdded,
                            LineChange::Modified => gutter::GutterDecoration::GitModified,
                            LineChange::Deleted => gutter::GutterDecoration::GitDeleted,
                        };
                        (*line, vec![decoration])
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        let lines = document_lines(&buffer);
        let (cursor_line, cursor_col) = buffer.line_col();
        let code_top = rect.top;
        let code_left = rect.left + theme::GUTTER_W + theme::CODE_PAD;
        let metrics = scroll_metrics(rect, code_left, &buffer);
        let (stored_x, stored_y) = s.workspace.scroll(pane);
        let scroll_x = stored_x.clamp(0.0, metrics.max_x);
        let scroll_y = stored_y.clamp(0.0, metrics.max_y);
        let visible_lines = visible_line_range(scroll_y, metrics.viewport_h, lines.len());
        let has_vertical = metrics.max_y > 0.0;
        let has_horizontal = metrics.max_x > 0.0;

        let st_pointer = state.clone();
        let pointer_focus = editor_focus.clone();
        root = root.on_pointer_down_with_button(move |cx, pointer, button| {
            let point = pointer.point;
            let over_vertical_scrollbar = has_vertical && point.x >= rect.right - SCROLLBAR_SIZE;
            let over_horizontal_scrollbar =
                has_horizontal && point.y >= rect.bottom - SCROLLBAR_SIZE;
            if !matches!(button, PointerButton::Left | PointerButton::Right)
                || over_vertical_scrollbar
                || over_horizontal_scrollbar
            {
                return;
            }

            pointer_focus.focus();
            st_pointer.update(move |app| {
                app.editor.caret_dragging = false;
                app.workspace.activate_pane(pane);
                let (scroll_x, scroll_y) = app.workspace.scroll(pane);
                let cursor = app.workspace.active_editor().map(|buffer| {
                    cursor_offset_from_point(
                        &buffer, rect, code_left, metrics, scroll_x, scroll_y, point.x, point.y,
                    )
                });
                if let Some(cursor) = cursor
                    && let Some(mut buffer) = app.workspace.active_editor_mut()
                {
                    if button == PointerButton::Right {
                        if !buffer.selection().is_some_and(|r| r.contains(&cursor)) {
                            buffer.set_cursor(cursor);
                        }
                        app.editor.menu = Some((point.x, point.y));
                        app.editor.menu_hover = None;
                        app.editor.drag = None;
                        app.tab_context_menu = None;
                        app.context_menu = None;
                        app.context_menu_target = None;
                        app.context_menu_hover = None;
                    } else {
                        app.editor.menu = None;
                        let clicks = app.editor.click_count(id, point.x, point.y);
                        let shift = app.editor.modifiers.shift();
                        let unit = if point.x < code_left || clicks == 3 {
                            SelectionUnit::Line
                        } else if clicks == 2 {
                            SelectionUnit::Word
                        } else {
                            SelectionUnit::Character
                        };
                        let origin = match unit {
                            SelectionUnit::Line => {
                                let line = line_index_from_point(
                                    point.y,
                                    rect.top,
                                    scroll_y,
                                    buffer.line_count(),
                                );
                                buffer.line_range_with_break(line)
                            }
                            SelectionUnit::Word => buffer.word_range(cursor),
                            SelectionUnit::Character => {
                                buffer.select_to(cursor, shift);
                                // Dragging extends from where the selection is anchored.
                                let anchor = buffer.selection_state().anchor.unwrap_or(cursor);
                                anchor..anchor
                            }
                        };
                        if unit != SelectionUnit::Character {
                            buffer.select_range(origin.clone());
                        }
                        app.editor.drag = Some(DragSelection {
                            document: id,
                            origin,
                            unit,
                            point: (point.x, point.y),
                        });
                    }
                }
                super::vim_input::after_pointer(app);
                reveal_cursor(app, rect);
            });
            cx.stop_propagation();
        });

        let st_drag = state.clone();
        root = root.on_pointer_drag(move |cx, pointer| {
            st_drag.update(|app| {
                if let Some(drag) = app.editor.drag.as_mut() {
                    app.editor.caret_dragging = true;
                    drag.point = (pointer.point.x, pointer.point.y);
                }
                update_drag_selection(app, rect);
            });
            cx.stop_propagation();
        });
        let st_up = state.clone();
        root = root.on_pointer_up(move |_, _| st_up.update(|app| app.editor.drag = None));

        let st_wheel = state.clone();
        root = root.on_wheel(move |cx, delta| {
            let (step_x, step_y) = match delta.unit {
                WheelUnit::Lines => (delta.x * theme::LINE_H * 3.0, delta.y * theme::LINE_H * 3.0),
                WheelUnit::Pixels => (delta.x, delta.y),
            };
            st_wheel.update(move |app| {
                let (step_x, step_y) = if app.editor.modifiers.shift() {
                    (step_x + step_y, 0.0)
                } else {
                    (step_x, step_y)
                };
                let (x, y) = app.workspace.scroll(pane);
                app.workspace.set_scroll(
                    pane,
                    (x - step_x).clamp(0.0, metrics.max_x),
                    (y - step_y).clamp(0.0, metrics.max_y),
                );
            });
            cx.stop_propagation();
        });

        // Default cursors inherit from ancestors in lgui. Keep the text cursor
        // on the content subtree so sibling scrollbars retain the arrow cursor.
        let content_rect = UiRect::new(
            rect.left,
            rect.top,
            rect.right - if has_vertical { SCROLLBAR_SIZE } else { 0.0 },
            rect.bottom - if has_horizontal { SCROLLBAR_SIZE } else { 0.0 },
        );
        let mut content = clip(content_rect, 0.0, 0.0)
            .key(format!("editor-content-{}", pane.get()))
            .cursor(CursorIcon::Text);
        if visible_lines.contains(&cursor_line) {
            let highlight_y = code_top + cursor_line as f32 * theme::LINE_H;
            let highlight = panel(
                UiRect::new(
                    rect.left + theme::GUTTER_W,
                    highlight_y,
                    rect.right,
                    highlight_y + theme::LINE_H,
                ),
                VisualStyle::filled(theme::c().active_line),
            );
            content = content.child(clip(rect, 0.0, -scroll_y).child(highlight));
        }

        let gutter_viewport = UiRect::new(
            rect.left,
            code_top,
            rect.left + theme::GUTTER_W,
            rect.bottom,
        );
        let gutter_content = UiRect::new(
            gutter_viewport.left,
            code_top,
            gutter_viewport.right,
            code_top + metrics.content_h,
        );
        content = content.child(clip(gutter_viewport, 0.0, -scroll_y).child(gutter::render(
            gutter_content,
            visible_lines.clone(),
            &decorations,
        )));

        let code_viewport = UiRect::new(
            code_left,
            code_top,
            code_left + metrics.viewport_w,
            code_top + metrics.viewport_h,
        );
        let layout_right = code_left + metrics.content_w;
        let mut code = group(UiRect::new(
            code_left,
            code_top,
            layout_right,
            code_top + metrics.content_h,
        ));

        // Vim draws its own visual selection; other views show the core one.
        let vim_view = s.vim.is_enabled() && s.vim.view() == Some((pane, id));
        let selections: Vec<Range<usize>> = vim_view
            .then(|| s.vim.highlights(buffer.buffer(), buffer.selection_state()))
            .flatten()
            .unwrap_or_else(|| buffer.selection().into_iter().collect());
        let matches = if s.vim.is_enabled() {
            s.vim.search_matches(buffer.buffer(), visible_lines.clone())
        } else {
            Vec::new()
        };
        for line_index in visible_lines.clone() {
            let y = code_top + line_index as f32 * theme::LINE_H;
            let line = lines[line_index];
            let layout = layout_line(line, code_left, y, layout_right);
            let (start, end) = (buffer.line_start(line_index), buffer.line_end(line_index));
            let next = buffer.line_range_with_break(line_index).end;
            // The part of `range` on this line, as a highlight rectangle.
            let line_rect = |range: &Range<usize>| {
                if range.start >= next || range.end <= start {
                    return None;
                }
                let from = range.start.max(start).min(end);
                let to = range.end.min(end).max(start);
                let left = caret_x(layout.as_ref(), buffer.text()[start..from].chars().count())
                    .unwrap_or(code_left);
                let mut right = caret_x(layout.as_ref(), buffer.text()[start..to].chars().count())
                    .unwrap_or(left);
                if range.end > end {
                    right += theme::CHAR_W;
                }
                Some(UiRect::new(
                    left,
                    y,
                    right.max(left + 2.0),
                    y + theme::LINE_H,
                ))
            };
            for found in &matches {
                if let Some(r) = line_rect(found) {
                    code = code.child(panel(r, VisualStyle::filled(theme::c().warning).alpha(45)));
                }
            }
            for selection in &selections {
                if let Some(r) = line_rect(selection) {
                    code = code.child(panel(r, VisualStyle::filled(theme::c().accent).alpha(55)));
                }
            }
            let mut x = code_left;
            let mut char_offset = 0;
            let mut display_column = 0;
            for (span, color) in syntax::highlight_line(line, meta.lang) {
                char_offset += span.chars().count();
                let next_x = caret_x(layout.as_ref(), char_offset)
                    .unwrap_or_else(|| x + span.chars().count() as f32 * theme::CHAR_W);
                let span_rect = UiRect::new(x, y, next_x.max(x) + 4.0, y + theme::LINE_H);
                let (display, _, column) = expand_tabs(&span, display_column);
                display_column = column;
                code = code.child(text(
                    span_rect,
                    display,
                    theme::mono(color, theme::CODE_SIZE),
                ));
                x = next_x;
            }
        }

        // Only the editor that owns keyboard focus shows a caret.
        let editor_focused = s.focused && focused_pane;
        if editor_focused && visible_lines.contains(&cursor_line) {
            let cursor_y = code_top + cursor_line as f32 * theme::LINE_H;
            let cursor_layout = layout_line(lines[cursor_line], code_left, cursor_y, layout_right);
            let cursor_x = caret_x(cursor_layout.as_ref(), cursor_col)
                .unwrap_or(code_left + cursor_col as f32 * theme::CHAR_W);
            let shape = if vim_view {
                s.vim.cursor_shape()
            } else {
                CursorShape::Bar
            };
            if shape == CursorShape::Bar {
                let cursor_rect = UiRect::new(
                    cursor_x,
                    cursor_y + 3.0,
                    cursor_x + 2.0,
                    cursor_y + theme::LINE_H - 3.0,
                );
                code = code.child(super::caret::render(
                    cursor_rect,
                    super::caret::CaretContext {
                        pane,
                        document: id,
                        viewport: rect,
                        scroll: (scroll_x, scroll_y),
                    },
                    s.smooth_caret
                        && !(s.editor.drag.is_some() && s.editor.caret_dragging)
                        && buffer.selection().is_none()
                        && s.editor.preedit_for(pane, id).is_none(),
                ));
            } else {
                // Block cursors cover the character under the cursor.
                let chars = buffer
                    .grapheme_at(buffer.cursor())
                    .filter(|grapheme| !grapheme.starts_with(['\r', '\n']))
                    .map_or(0, |grapheme| grapheme.chars().count());
                let right = (chars > 0)
                    .then(|| caret_x(cursor_layout.as_ref(), cursor_col + chars))
                    .flatten()
                    .unwrap_or(cursor_x + theme::CHAR_W)
                    .max(cursor_x + 2.0);
                let top = match shape {
                    CursorShape::Underline => cursor_y + theme::LINE_H - 3.0,
                    CursorShape::HalfBlock => cursor_y + theme::LINE_H / 2.0,
                    _ => cursor_y + 1.0,
                };
                let cursor_rect = UiRect::new(cursor_x, top, right, cursor_y + theme::LINE_H - 1.0);
                code = code.child(panel(
                    cursor_rect,
                    VisualStyle::filled(theme::c().accent).alpha(140),
                ));
            }
            if let Some(session) = s.editor.preedit_for(pane, id) {
                let width = layout_line(&session.text, 0.0, 0.0, metrics.content_w)
                    .map_or(80.0, |l| l.width)
                    .max(2.0);
                let r = UiRect::new(
                    cursor_x,
                    cursor_y,
                    cursor_x + width + 4.0,
                    cursor_y + theme::LINE_H,
                );
                code = code.child(
                    panel(r, VisualStyle::filled(theme::c().surface))
                        .child(text(
                            r,
                            session.text.clone(),
                            theme::mono(theme::c().text_bright, theme::CODE_SIZE),
                        ))
                        .child(panel(
                            UiRect::new(r.left, r.bottom - 2.0, r.right, r.bottom - 1.0),
                            VisualStyle::filled(theme::c().accent),
                        )),
                );
            }
        }

        content = content.child(clip(code_viewport, -scroll_x, -scroll_y).child(code));
        root = root.child(content);

        let emphasized = s.editor_hovered == Some(pane)
            || (focused_pane
                && (s.editor_vertical_scrollbar_dragging
                    || s.editor_horizontal_scrollbar_dragging));
        if has_vertical {
            root = root.child(vertical_scrollbar(
                pane,
                rect,
                metrics,
                scroll_y,
                has_horizontal,
                emphasized,
                state.clone(),
            ));
        }
        if has_horizontal {
            root = root.child(horizontal_scrollbar(
                pane,
                rect,
                code_left,
                metrics,
                scroll_x,
                has_vertical,
                emphasized,
                state.clone(),
            ));
        }
    }

    let st_input = state.clone();
    root = root.on_input(move |ctx, input| {
        let input = input.to_string();
        st_input.update(move |app| insert_text(app, &input, rect));
        ctx.stop_propagation();
    });

    let st_ime = state.clone();
    root = root.on_composition_update(move |_, value, cursor| {
        st_ime.update(|app| update_composition(app, value, cursor))
    });
    let st_ime_end = state.clone();
    root = root.on_composition_end(move |_| st_ime_end.update(|app| app.editor.ime = None));

    // Bound keys are dispatched from the keymap (`key_actions`) before they
    // reach the editor. Unbound shortcut chords must still not type text.
    root = root.on_key_down(move |ctx, event| {
        if event.state == KeyState::Down
            && (event.modifiers.ctrl() || event.modifiers.meta())
            && !event.modifiers.alt()
        {
            ctx.prevent_default();
        }
    });

    crate::ui::track_main_surface_focus(root, state)
}

pub(crate) fn vertical_scrollbar(
    pane: PaneId,
    rect: UiRect,
    metrics: ScrollMetrics,
    scroll: f32,
    has_horizontal: bool,
    emphasized: bool,
    state: State<AppState>,
) -> Element {
    let corner = if has_horizontal { SCROLLBAR_SIZE } else { 0.0 };
    let track = UiRect::new(
        rect.right - SCROLLBAR_SIZE,
        rect.top,
        rect.right,
        rect.bottom - corner,
    );
    let geometry = thumb_geometry(
        track.top,
        track.height(),
        metrics.viewport_h,
        metrics.content_h,
        scroll,
    );
    let travel = track.height() - geometry.length;
    let alpha = if emphasized { 0xb8 } else { 0x68 };

    let st_track_down = state.clone();
    let st_track_move = state.clone();
    let st_track_up = state.clone();
    let track_hit = scrollbar_track(track)
        .key(format!("editor-vertical-scrollbar-track-{}", pane.get()))
        .event_policy(EventPolicy::INTERACTIVE)
        .on_pointer_down(move |_cx, pointer| {
            let offset = geometry.length / 2.0;
            let thumb_start = (pointer.point.y - track.top - offset).clamp(0.0, travel);
            let next = scroll_from_thumb(thumb_start, travel, metrics.max_y);
            st_track_down.update(move |app| {
                app.workspace.activate_pane(pane);
                let (x, _) = app.workspace.scroll(pane);
                app.workspace.set_scroll(pane, x, next);
                app.editor_vertical_scrollbar_dragging = true;
                app.editor_vertical_scrollbar_drag_offset = offset;
            });
        })
        .on_pointer_drag(move |_cx, pointer| {
            st_track_move.update(move |app| {
                let thumb_start =
                    (pointer.point.y - track.top - app.editor_vertical_scrollbar_drag_offset)
                        .clamp(0.0, travel);
                let next = scroll_from_thumb(thumb_start, travel, metrics.max_y);
                let (x, _) = app.workspace.scroll(pane);
                app.workspace.set_scroll(pane, x, next);
            });
        })
        .on_pointer_up(move |_cx, _pointer| {
            st_track_up.update(|app| app.editor_vertical_scrollbar_dragging = false);
        });

    let thumb_rect = UiRect::new(
        track.left + SCROLLBAR_INSET,
        geometry.start,
        track.right - SCROLLBAR_INSET,
        geometry.start + geometry.length,
    );
    let st_thumb_down = state.clone();
    let st_thumb_move = state.clone();
    let st_thumb_up = state;
    let thumb = scrollbar_thumb(thumb_rect, alpha)
        .key(format!("editor-vertical-scrollbar-thumb-{}", pane.get()))
        .event_policy(EventPolicy::INTERACTIVE)
        .on_pointer_down(move |_cx, pointer| {
            st_thumb_down.update(move |app| {
                app.workspace.activate_pane(pane);
                app.editor_vertical_scrollbar_dragging = true;
                app.editor_vertical_scrollbar_drag_offset = pointer.point.y - geometry.start;
            });
        })
        .on_pointer_drag(move |_cx, pointer| {
            st_thumb_move.update(move |app| {
                let thumb_start =
                    (pointer.point.y - track.top - app.editor_vertical_scrollbar_drag_offset)
                        .clamp(0.0, travel);
                let next = scroll_from_thumb(thumb_start, travel, metrics.max_y);
                let (x, _) = app.workspace.scroll(pane);
                app.workspace.set_scroll(pane, x, next);
            });
        })
        .on_pointer_up(move |_cx, _pointer| {
            st_thumb_up.update(|app| app.editor_vertical_scrollbar_dragging = false);
        });

    group(track)
        .key(format!("editor-vertical-scrollbar-{}", pane.get()))
        .child(track_hit)
        .child(thumb)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn horizontal_scrollbar(
    pane: PaneId,
    rect: UiRect,
    code_left: f32,
    metrics: ScrollMetrics,
    scroll: f32,
    has_vertical: bool,
    emphasized: bool,
    state: State<AppState>,
) -> Element {
    let corner = if has_vertical { SCROLLBAR_SIZE } else { 0.0 };
    let track = UiRect::new(
        code_left,
        rect.bottom - SCROLLBAR_SIZE,
        rect.right - corner,
        rect.bottom,
    );
    let geometry = thumb_geometry(
        track.left,
        track.width(),
        metrics.viewport_w,
        metrics.content_w,
        scroll,
    );
    let travel = track.width() - geometry.length;
    let alpha = if emphasized { 0xb8 } else { 0x68 };

    let st_track_down = state.clone();
    let st_track_move = state.clone();
    let st_track_up = state.clone();
    let track_hit = scrollbar_track(track)
        .key(format!("editor-horizontal-scrollbar-track-{}", pane.get()))
        .event_policy(EventPolicy::INTERACTIVE)
        .on_pointer_down(move |_cx, pointer| {
            let offset = geometry.length / 2.0;
            let thumb_start = (pointer.point.x - track.left - offset).clamp(0.0, travel);
            let next = scroll_from_thumb(thumb_start, travel, metrics.max_x);
            st_track_down.update(move |app| {
                app.workspace.activate_pane(pane);
                let (_, y) = app.workspace.scroll(pane);
                app.workspace.set_scroll(pane, next, y);
                app.editor_horizontal_scrollbar_dragging = true;
                app.editor_horizontal_scrollbar_drag_offset = offset;
            });
        })
        .on_pointer_drag(move |_cx, pointer| {
            st_track_move.update(move |app| {
                let thumb_start =
                    (pointer.point.x - track.left - app.editor_horizontal_scrollbar_drag_offset)
                        .clamp(0.0, travel);
                let next = scroll_from_thumb(thumb_start, travel, metrics.max_x);
                let (_, y) = app.workspace.scroll(pane);
                app.workspace.set_scroll(pane, next, y);
            });
        })
        .on_pointer_up(move |_cx, _pointer| {
            st_track_up.update(|app| app.editor_horizontal_scrollbar_dragging = false);
        });

    let thumb_rect = UiRect::new(
        geometry.start,
        track.top + SCROLLBAR_INSET,
        geometry.start + geometry.length,
        track.bottom - SCROLLBAR_INSET,
    );
    let st_thumb_down = state.clone();
    let st_thumb_move = state.clone();
    let st_thumb_up = state;
    let thumb = scrollbar_thumb(thumb_rect, alpha)
        .key(format!("editor-horizontal-scrollbar-thumb-{}", pane.get()))
        .event_policy(EventPolicy::INTERACTIVE)
        .on_pointer_down(move |_cx, pointer| {
            st_thumb_down.update(move |app| {
                app.workspace.activate_pane(pane);
                app.editor_horizontal_scrollbar_dragging = true;
                app.editor_horizontal_scrollbar_drag_offset = pointer.point.x - geometry.start;
            });
        })
        .on_pointer_drag(move |_cx, pointer| {
            st_thumb_move.update(move |app| {
                let thumb_start =
                    (pointer.point.x - track.left - app.editor_horizontal_scrollbar_drag_offset)
                        .clamp(0.0, travel);
                let next = scroll_from_thumb(thumb_start, travel, metrics.max_x);
                let (_, y) = app.workspace.scroll(pane);
                app.workspace.set_scroll(pane, next, y);
            });
        })
        .on_pointer_up(move |_cx, _pointer| {
            st_thumb_up.update(|app| app.editor_horizontal_scrollbar_dragging = false);
        });

    group(track)
        .key(format!("editor-horizontal-scrollbar-{}", pane.get()))
        .child(track_hit)
        .child(thumb)
}

/// Update an active editor scrollbar drag from a window-level pointer move.
///
/// This deliberately uses only the pointer coordinate for the dragged axis,
/// so moving away from the scrollbar strip does not interrupt the drag.
pub fn drag_scrollbars(app: &mut AppState, rect: UiRect, pointer_x: f32, pointer_y: f32) -> bool {
    if !app.editor_vertical_scrollbar_dragging && !app.editor_horizontal_scrollbar_dragging {
        return false;
    }

    let code_left = rect.left + theme::GUTTER_W + theme::CODE_PAD;
    let Some(metrics) = app
        .workspace
        .active_editor()
        .map(|buffer| scroll_metrics(rect, code_left, &buffer))
    else {
        return finish_scrollbar_drag(app);
    };
    drag_scrollbars_with_metrics(app, rect, code_left, metrics, pointer_x, pointer_y)
}

pub(crate) fn drag_scrollbars_with_metrics(
    app: &mut AppState,
    rect: UiRect,
    code_left: f32,
    metrics: ScrollMetrics,
    pointer_x: f32,
    pointer_y: f32,
) -> bool {
    let (mut scroll_x, mut scroll_y) = app.workspace.active_scroll();

    if app.editor_vertical_scrollbar_dragging && metrics.max_y > 0.0 {
        let corner = if metrics.max_x > 0.0 {
            SCROLLBAR_SIZE
        } else {
            0.0
        };
        let track_top = rect.top;
        let track_length = (rect.bottom - corner) - track_top;
        let geometry = thumb_geometry(
            track_top,
            track_length,
            metrics.viewport_h,
            metrics.content_h,
            scroll_y,
        );
        let travel = track_length - geometry.length;
        let thumb_start =
            (pointer_y - track_top - app.editor_vertical_scrollbar_drag_offset).clamp(0.0, travel);
        scroll_y = scroll_from_thumb(thumb_start, travel, metrics.max_y);
    }

    if app.editor_horizontal_scrollbar_dragging && metrics.max_x > 0.0 {
        let corner = if metrics.max_y > 0.0 {
            SCROLLBAR_SIZE
        } else {
            0.0
        };
        let track_left = code_left;
        let track_length = (rect.right - corner) - track_left;
        let geometry = thumb_geometry(
            track_left,
            track_length,
            metrics.viewport_w,
            metrics.content_w,
            scroll_x,
        );
        let travel = track_length - geometry.length;
        let thumb_start = (pointer_x - track_left - app.editor_horizontal_scrollbar_drag_offset)
            .clamp(0.0, travel);
        scroll_x = scroll_from_thumb(thumb_start, travel, metrics.max_x);
    }

    let previous = app.workspace.active_scroll();
    app.workspace.set_active_scroll(scroll_x, scroll_y);
    previous != app.workspace.active_scroll()
}

pub fn finish_scrollbar_drag(app: &mut AppState) -> bool {
    let changed =
        app.editor_vertical_scrollbar_dragging || app.editor_horizontal_scrollbar_dragging;
    app.editor_vertical_scrollbar_dragging = false;
    app.editor_horizontal_scrollbar_dragging = false;
    changed
}

fn document_lines(buffer: &TextBuffer) -> Vec<&str> {
    buffer.lines().collect()
}

fn scroll_metrics(rect: UiRect, code_left: f32, buffer: &TextBuffer) -> ScrollMetrics {
    let mut viewport_w = (rect.right - code_left).max(1.0);
    let mut viewport_h = rect.height().max(1.0);
    let natural_w = buffer.max_line_display_width() as f32 * theme::CHAR_W + TRAILING_CODE_SPACE;
    let content_h = (buffer.line_count() as f32 * theme::LINE_H).max(theme::LINE_H);
    // A scrollbar may make the other axis overflow; reserve both before revealing the caret.
    for _ in 0..2 {
        viewport_w = (rect.right
            - code_left
            - if content_h > viewport_h {
                SCROLLBAR_SIZE
            } else {
                0.0
            })
        .max(1.0);
        viewport_h = (rect.height()
            - if natural_w > viewport_w {
                SCROLLBAR_SIZE
            } else {
                0.0
            })
        .max(1.0);
    }
    let content_w = natural_w.max(viewport_w);
    ScrollMetrics {
        viewport_w,
        viewport_h,
        content_w,
        content_h,
        max_x: (content_w - viewport_w).max(0.0),
        max_y: (content_h - viewport_h).max(0.0),
    }
}

fn visual_columns(line: &str) -> usize {
    crate::model::text::display_width(line)
}

fn visible_line_range(scroll_y: f32, viewport_h: f32, line_count: usize) -> Range<usize> {
    let first = ((scroll_y / theme::LINE_H).floor() as usize).saturating_sub(1);
    let last = (((scroll_y + viewport_h) / theme::LINE_H).ceil() as usize + 1).min(line_count);
    first.min(last)..last
}

fn thumb_geometry(
    track_start: f32,
    track_length: f32,
    viewport_length: f32,
    content_length: f32,
    scroll: f32,
) -> ThumbGeometry {
    let length = (track_length * viewport_length / content_length.max(1.0))
        .max(MIN_THUMB_LENGTH)
        .min(track_length);
    let travel = (track_length - length).max(0.0);
    let max_scroll = (content_length - viewport_length).max(0.0);
    let position = if max_scroll > 0.0 {
        travel * (scroll.clamp(0.0, max_scroll) / max_scroll)
    } else {
        0.0
    };
    ThumbGeometry {
        start: track_start + position,
        length,
    }
}

fn scroll_from_thumb(thumb_start: f32, travel: f32, max_scroll: f32) -> f32 {
    if travel <= 0.0 {
        0.0
    } else {
        (thumb_start / travel * max_scroll).clamp(0.0, max_scroll)
    }
}

/// Layout uses expanded tabs, while all buffer positions remain source character indices.
struct LineLayout {
    layout: TextLayout,
    source_to_display: Vec<usize>,
    width: f32,
}
/// Expand tabs at four-column stops for editor and diff display.
pub(crate) fn expand_tabs(line: &str, initial_column: usize) -> (String, Vec<usize>, usize) {
    let mut result = String::new();
    let mut mapping = vec![0];
    let mut column = initial_column;
    let mut display_chars = 0;
    for grapheme in line.graphemes(true) {
        if grapheme == "\t" {
            let spaces = 4 - column % 4;
            result.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
            display_chars += spaces;
            mapping.push(display_chars);
        } else {
            for ch in grapheme.chars() {
                result.push(ch);
                display_chars += 1;
                mapping.push(display_chars);
            }
            column += UnicodeWidthStr::width(grapheme);
        }
    }
    (result, mapping, column)
}
fn layout_line(line: &str, left: f32, top: f32, right: f32) -> Option<LineLayout> {
    let (display, source_to_display, _) = expand_tabs(line, 0);
    let bounds = UiRect::new(left, top, right.max(left + 1.0), top + theme::LINE_H);
    let mut request = TextLayoutRequest::single_line(&display, bounds, theme::CODE_SIZE, 400);
    request.font_families = theme::MONO_FAMILIES;
    text::layout(&request).map(|layout| LineLayout {
        width: layout.width,
        layout,
        source_to_display,
    })
}
fn caret_x(layout: Option<&LineLayout>, char_index: usize) -> Option<f32> {
    let layout = layout?;
    let index = *layout
        .source_to_display
        .get(char_index)
        .or_else(|| layout.source_to_display.last())?;
    layout.layout.caret_rect(index).map(|rect| rect.left)
}
fn hit_column(layout: &LineLayout, x: f32, y: f32) -> usize {
    let index = layout.layout.hit_test(x, y).index;
    layout
        .source_to_display
        .iter()
        .enumerate()
        .min_by_key(|(_, i)| i.abs_diff(index))
        .map_or(0, |(i, _)| i)
}
fn fallback_hit_column(line: &str, x: f32) -> usize {
    let mut column = 0;
    let mut chars = 0;
    for g in line.graphemes(true) {
        let width = if g == "\t" {
            4 - column % 4
        } else {
            UnicodeWidthStr::width(g)
        };
        if x < (column as f32 + width as f32 / 2.0) * theme::CHAR_W {
            return chars;
        }
        column += width;
        chars += g.chars().count();
    }
    chars
}

#[allow(clippy::too_many_arguments)]
fn cursor_offset_from_point(
    buffer: &TextBuffer,
    rect: UiRect,
    code_left: f32,
    metrics: ScrollMetrics,
    scroll_x: f32,
    scroll_y: f32,
    point_x: f32,
    point_y: f32,
) -> usize {
    let line = line_index_from_point(point_y, rect.top, scroll_y, buffer.line_count());
    let line_text = buffer.line(line);
    let content_x = point_x + scroll_x;
    let content_y = point_y + scroll_y;
    let line_top = rect.top + line as f32 * theme::LINE_H;
    let char_index = layout_line(
        line_text,
        code_left,
        line_top,
        code_left + metrics.content_w,
    )
    .map(|layout| hit_column(&layout, content_x, content_y))
    .unwrap_or_else(|| fallback_hit_column(line_text, content_x - code_left));
    let line_start = buffer.line_start(line);
    line_start
        + line_text
            .char_indices()
            .nth(char_index)
            .map(|(offset, _)| offset)
            .unwrap_or(line_text.len())
}

fn line_index_from_point(
    point_y: f32,
    viewport_top: f32,
    scroll_y: f32,
    line_count: usize,
) -> usize {
    if line_count == 0 {
        return 0;
    }
    (((point_y - viewport_top + scroll_y) / theme::LINE_H)
        .floor()
        .max(0.0) as usize)
        .min(line_count - 1)
}

pub(crate) fn reveal_cursor(app: &mut AppState, rect: UiRect) {
    let Some(buffer) = app.workspace.active_editor() else {
        return;
    };
    let (line, column) = buffer.line_col();
    let code_left = rect.left + theme::GUTTER_W + theme::CODE_PAD;
    let metrics = scroll_metrics(rect, code_left, &buffer);
    let (mut scroll_x, mut scroll_y) = app.workspace.active_scroll();

    let line_top = line as f32 * theme::LINE_H;
    let line_bottom = line_top + theme::LINE_H;
    if line_top < scroll_y {
        scroll_y = line_top;
    } else if line_bottom > scroll_y + metrics.viewport_h {
        scroll_y = line_bottom - metrics.viewport_h;
    }

    let layout = layout_line(
        buffer.line(line),
        code_left,
        rect.top,
        code_left + metrics.content_w,
    );
    let caret = caret_x(layout.as_ref(), column)
        .unwrap_or(code_left + column as f32 * theme::CHAR_W)
        - code_left;
    if caret < scroll_x + CURSOR_REVEAL_MARGIN {
        scroll_x = (caret - CURSOR_REVEAL_MARGIN).max(0.0);
    } else if caret > scroll_x + metrics.viewport_w - CURSOR_REVEAL_MARGIN {
        scroll_x = caret - metrics.viewport_w + CURSOR_REVEAL_MARGIN;
    }

    app.workspace.set_active_scroll(
        scroll_x.clamp(0.0, metrics.max_x),
        scroll_y.clamp(0.0, metrics.max_y),
    );
}

pub fn execute_command(
    state: &State<AppState>,
    command: Command,
    rect: UiRect,
    clipboard: lgui::services::ClipboardHandle,
) {
    state.update(|app| {
        if command == Command::Escape && app.editor.menu.take().is_some() {
            return;
        }
        app.editor.menu = None;
        app.editor.drag = None;
        // Copy and Cut of a Vim visual selection go through Vim's registers.
        if matches!(command, Command::Copy | Command::Cut)
            && vim_input::visual_clipboard(app, command == Command::Cut, rect, clipboard.as_ref())
        {
            return;
        }
        match apply_command(app, command, clipboard.as_ref()) {
            Ok(pasted) => vim_input::record_insert(app, insert_event(command, pasted)),
            Err(error) => app.show_error(format!("Clipboard: {error}")),
        }
        vim_input::after_command(app, matches!(command, Command::Undo | Command::Redo));
        reveal_cursor(app, rect);
    });
}

/// The typing event a default editing command makes in Vim's Insert mode;
/// `None` for commands that move the cursor or reach outside typing.
fn insert_event(command: Command, pasted: Option<String>) -> Option<vim::InsertEvent> {
    use vim::InsertEvent;
    Some(match command {
        Command::Delete(false, false) => InsertEvent::Backspace,
        Command::Delete(true, false) => InsertEvent::Delete,
        Command::Delete(false, true) => InsertEvent::DeleteWordBack,
        Command::Delete(true, true) => InsertEvent::DeleteWordForward,
        Command::Enter => InsertEvent::Newline,
        Command::Indent(false) => InsertEvent::Tab,
        Command::Indent(true) => InsertEvent::Backtab,
        Command::Paste => InsertEvent::Paste(pasted?),
        _ => return None,
    })
}

pub(crate) fn apply_command(
    app: &mut AppState,
    command: Command,
    clipboard: &dyn lgui::services::Clipboard,
) -> Result<Option<String>, lgui::services::ClipboardError> {
    if app.workspace.active_editor().is_none() {
        return Ok(None);
    }
    match command {
        Command::Copy | Command::Cut => {
            let selected = app
                .workspace
                .active_editor()
                .and_then(|buffer| buffer.selected_text())
                .map(str::to_owned);
            if let Some(value) = selected {
                // A failed copy must leave the source text and selection untouched.
                clipboard.write_text(&value)?;
                if command == Command::Cut {
                    app.workspace.promote_active_preview();
                }
                if let Some(mut buffer) = app.workspace.active_editor_mut() {
                    buffer.break_undo_group();
                    if command == Command::Cut {
                        normal::backspace(&mut buffer);
                        buffer.break_undo_group();
                    }
                }
            }
        }
        Command::Paste => {
            if let Some(value) = clipboard.read_text()? {
                let changes_text = !value.is_empty()
                    || app
                        .workspace
                        .active_editor()
                        .is_some_and(|buffer| buffer.selection().is_some());
                if changes_text {
                    app.workspace.promote_active_preview();
                }
                if let Some(mut buffer) = app.workspace.active_editor_mut() {
                    normal::paste(&mut buffer, &value);
                }
                return Ok(Some(value));
            }
        }
        Command::Delete(..)
        | Command::Enter
        | Command::Indent(..)
        | Command::Undo
        | Command::Redo => {
            let changes_text = match command {
                Command::Undo => app
                    .workspace
                    .active_editor()
                    .is_some_and(|editor| editor.can_undo()),
                Command::Redo => app
                    .workspace
                    .active_editor()
                    .is_some_and(|editor| editor.can_redo()),
                _ => true,
            };
            if changes_text {
                app.workspace.promote_active_preview();
            }
            if let Some(mut buffer) = app.workspace.active_editor_mut() {
                commands::apply(&mut buffer, command);
            }
        }
        _ => {
            if let Some(mut buffer) = app.workspace.active_editor_mut() {
                commands::apply(&mut buffer, command);
            }
        }
    }
    Ok(None)
}

fn ime_cursor_rect(app: &AppState, rect: UiRect) -> UiRect {
    let Some(buffer) = app.workspace.active_editor() else {
        return rect;
    };
    let (line, col) = buffer.line_col();
    let (sx, sy) = app.workspace.active_scroll();
    let left = rect.left + theme::GUTTER_W + theme::CODE_PAD;
    let top = rect.top + line as f32 * theme::LINE_H - sy;
    let layout = layout_line(buffer.line(line), left, top, left + 100000.0);
    let mut x = caret_x(layout.as_ref(), col).unwrap_or(left) - sx;
    let pane = app.workspace.active_pane();
    if let Some(session) = app
        .workspace
        .active()
        .and_then(|document| app.editor.preedit_for(pane, document))
    {
        let index = session
            .cursor
            .as_ref()
            .map_or(session.text.len(), |r| r.start)
            .min(session.text.len());
        let prefix = session.text.get(..index).unwrap_or("");
        x += layout_line(prefix, 0.0, 0.0, 100000.0).map_or(0.0, |l| l.width);
    }
    let x = x.clamp(left, rect.right.max(left));
    let top = top.clamp(rect.top, (rect.bottom - theme::LINE_H).max(rect.top));
    UiRect::new(x, top, x + 2.0, top + theme::LINE_H)
}

fn update_drag_selection(app: &mut AppState, rect: UiRect) {
    let Some(drag) = app.editor.drag.clone() else {
        return;
    };
    if app.workspace.active() != Some(drag.document) {
        app.editor.drag = None;
        return;
    }
    let (sx, sy) = app.workspace.active_scroll();
    let left = rect.left + theme::GUTTER_W + theme::CODE_PAD;
    if let Some(mut buffer) = app.workspace.active_editor_mut() {
        let metrics = scroll_metrics(rect, left, &buffer);
        let cursor = cursor_offset_from_point(
            &buffer,
            rect,
            left,
            metrics,
            sx,
            sy,
            drag.point.0,
            drag.point.1,
        );
        match drag.unit {
            SelectionUnit::Character => {
                buffer.set_cursor(drag.origin.start);
                buffer.select_to(cursor, true);
            }
            SelectionUnit::Word | SelectionUnit::Line => {
                let range = if drag.unit == SelectionUnit::Word {
                    buffer.word_range(cursor)
                } else {
                    buffer.line_range_with_break(line_index_from_point(
                        drag.point.1,
                        rect.top,
                        sy,
                        buffer.line_count(),
                    ))
                };
                if range.start < drag.origin.start {
                    buffer.set_cursor(drag.origin.end);
                    buffer.select_to(range.start, true);
                } else {
                    buffer.set_cursor(drag.origin.start);
                    buffer.select_to(range.end.max(drag.origin.end), true);
                }
            }
        }
    }
    super::vim_input::after_pointer(app);
}

/// Called while the pointer is held down, even when it stops moving outside the viewport.
pub fn drag_scroll_tick(app: &mut AppState, rect: UiRect) -> bool {
    let Some(drag) = app.editor.drag.as_ref() else {
        return false;
    };
    if app.workspace.active() != Some(drag.document) {
        app.editor.drag = None;
        return true;
    }
    let left = rect.left + theme::GUTTER_W + theme::CODE_PAD;
    let speed = |p: f32, low: f32, high: f32| {
        if p < low {
            -(low - p).clamp(4.0, 80.0)
        } else if p > high {
            (p - high).clamp(4.0, 80.0)
        } else {
            0.0
        }
    };
    let dx = if drag.unit == SelectionUnit::Line {
        0.0
    } else {
        speed(drag.point.0, left, rect.right - SCROLLBAR_SIZE)
    };
    let dy = speed(drag.point.1, rect.top, rect.bottom - SCROLLBAR_SIZE);
    if dx == 0.0 && dy == 0.0 {
        return false;
    }
    let Some(buffer) = app.workspace.active_editor() else {
        return false;
    };
    let metrics = scroll_metrics(rect, left, &buffer);
    let (x, y) = app.workspace.active_scroll();
    let next = (
        (x + dx).clamp(0.0, metrics.max_x),
        (y + dy).clamp(0.0, metrics.max_y),
    );
    if next == (x, y) {
        return false;
    }
    app.workspace.set_active_scroll(next.0, next.1);
    update_drag_selection(app, rect);
    true
}

/// Track the composition shown by the input method. A composition belongs to
/// the view that was active when it started.
fn update_composition(app: &mut AppState, text: &str, cursor: Option<Range<usize>>) {
    let pane = app.workspace.active_pane();
    let Some(document) = app.workspace.active() else {
        return;
    };
    match app.editor.ime.as_mut() {
        Some(session) => {
            // An emptied composition has ended; a new one starts where the
            // focus is now.
            if session.text.is_empty() {
                (session.pane, session.document) = (pane, document);
            }
            session.text = text.to_owned();
            session.cursor = cursor;
        }
        None if !text.is_empty() => {
            app.editor.ime = Some(ImeSession {
                pane,
                document,
                text: text.to_owned(),
                cursor,
            });
        }
        None => {}
    }
}

/// Insert typed, committed or replayed text. Input method commits go to the
/// view their composition started in, and are dropped if it has closed.
pub fn insert_text(app: &mut AppState, input: &str, rect: UiRect) {
    app.editor.drag = None;
    app.editor.menu = None;
    let session = app.editor.ime.take();
    let pane = session
        .as_ref()
        .map_or(app.workspace.active_pane(), |session| session.pane);
    let Some(document) = session
        .as_ref()
        .map(|session| session.document)
        .or_else(|| app.workspace.active())
    else {
        return;
    };
    let in_active_view =
        app.workspace.active_pane() == pane && app.workspace.active() == Some(document);
    if in_active_view && vim_input::is_active(app) {
        if vim_input::handle_text(app, input, rect) {
            return;
        }
        // Text in Normal or Visual mode is not a command. Only a composition
        // the input method commits late is still typed.
        if session.is_none() {
            return;
        }
    }
    if !input.is_empty() {
        app.workspace.promote_preview(pane, document);
    }
    if let Some(mut editor) = app.workspace.view_editor_mut(pane, document) {
        // A composition commit is an undo step of its own.
        if session.is_some() {
            editor.break_undo_group();
        }
        normal::insert(&mut editor, input);
        if session.is_some() {
            editor.break_undo_group();
        }
    }
    if app.workspace.active_pane() == pane && app.workspace.active() == Some(document) {
        reveal_cursor(app, rect);
    }
}

/// Lines in one page of an editor of this size, for page movements.
pub fn page_lines(rect: UiRect) -> usize {
    (rect.height() / theme::LINE_H).floor().max(1.0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgui::core::{KeyModifiers, KeyboardEvent, LogicalKey};
    use lgui::services::{Clipboard, ClipboardError};
    use std::sync::Mutex;

    #[derive(Default)]
    struct TestClipboard {
        text: Mutex<Option<String>>,
        fail: bool,
    }
    impl Clipboard for TestClipboard {
        fn read_text(&self) -> Result<Option<String>, ClipboardError> {
            if self.fail {
                Err(ClipboardError::AccessDenied)
            } else {
                Ok(self.text.lock().unwrap().clone())
            }
        }
        fn write_text(&self, text: &str) -> Result<(), ClipboardError> {
            if self.fail {
                return Err(ClipboardError::AccessDenied);
            }
            *self.text.lock().unwrap() = Some(text.to_owned());
            Ok(())
        }
    }
    fn document(contents: &str) -> AppState {
        let mut app = AppState::new();
        app.workspace
            .open_path("editing-test.txt".into(), contents.into());
        app
    }

    /// Exercise real lgui hit testing, focus and event dispatch without opening a native window.
    #[test]
    fn editor_events_route_drag_selection_shortcuts_and_ime_commit() {
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{
            ImeEvent, InputEvent, Point, PointerData, UiScale, dispatch_runtime_output,
        };
        use lgui::session::UiSession;
        use std::sync::Arc;
        let exposed = Arc::new(Mutex::new(None::<State<AppState>>));
        let output = exposed.clone();
        let viewport = UiRect::new(0.0, 0.0, 500.0, 300.0);
        let app = ApplicationContext::empty(Default::default());
        let view_app = app.clone();
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state(document("hello world\nsecond"));
            let git_store = cx.state(GitStoreSnapshot::default());
            *output.lock().unwrap() = Some(state.clone());
            let id = cx.use_stable_id();
            let focus = cx.focus_handle(id.clone());
            // Shortcuts reach the editor through the keymap, as in the app root.
            let env = crate::key_actions::KeyEnv {
                state: state.clone(),
                application: view_app.clone(),
                editor_focus: focus.clone(),
                editor_rect: viewport,
                terminal: None,
            };
            crate::key_actions::attach(
                group(viewport).child(render(
                    PaneId::new(1),
                    viewport,
                    state,
                    git_store,
                    id,
                    focus,
                )),
                env,
            )
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let state = exposed.lock().unwrap().clone().unwrap();
        let mut send = |input| {
            let events = session.handle_input(input);
            let mut prevented = false;
            dispatch_runtime_output(
                events,
                &app,
                &lgui::window::WindowId::new("editor-test"),
                |action| session.handle_default_action(action),
                |cx| prevented |= cx.default_prevented(),
            );
            session.render_view(&view, viewport, UiScale::ONE);
            prevented
        };
        let left = theme::GUTTER_W + theme::CODE_PAD;
        let pointer =
            |column: f32| PointerData::mouse(Point::new(left + column * theme::CHAR_W, 5.0));
        send(InputEvent::PointerDown {
            pointer: pointer(0.0),
            button: PointerButton::Left,
        });
        send(InputEvent::PointerMove(pointer(5.0)));
        send(InputEvent::PointerUp {
            pointer: pointer(5.0),
            button: PointerButton::Left,
        });
        assert_eq!(
            state
                .get()
                .workspace
                .active_editor()
                .unwrap()
                .selected_text(),
            Some("hello")
        );
        let key = |key, modifiers| {
            InputEvent::Keyboard(KeyboardEvent {
                state: KeyState::Down,
                key,
                modifiers,
                ..Default::default()
            })
        };
        assert!(send(key(
            LogicalKey::Named(lgui::core::NamedKey::Tab),
            KeyModifiers::empty()
        )));
        assert_eq!(
            state.get().workspace.active_editor().unwrap().text(),
            "  hello world\nsecond"
        );
        send(key(
            LogicalKey::Character("z".into()),
            KeyModifiers::CONTROL,
        ));
        assert_eq!(
            state
                .get()
                .workspace
                .active_editor()
                .unwrap()
                .selected_text(),
            Some("hello")
        );
        send(InputEvent::Ime(ImeEvent::Preedit {
            text: "nihao".into(),
            cursor: Some(0..5),
        }));
        assert_eq!(
            state.get().workspace.active_editor().unwrap().text(),
            "hello world\nsecond"
        );
        send(InputEvent::Ime(ImeEvent::Commit("你好".into())));
        assert_eq!(
            state.get().workspace.active_editor().unwrap().text(),
            "你好 world\nsecond"
        );
        assert!(state.get().editor.ime.is_none());
        send(key(
            LogicalKey::Character("z".into()),
            KeyModifiers::CONTROL,
        ));
        assert_eq!(
            state
                .get()
                .workspace
                .active_editor()
                .unwrap()
                .selected_text(),
            Some("hello")
        );
    }

    #[test]
    fn smooth_caret_keeps_input_and_ime_logical_and_snaps_during_drag_or_context_changes() {
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{
            ImeEvent, InputEvent, NamedKey, Point, PointerData, UiScale, dispatch_runtime_output,
        };
        use lgui::session::UiSession;
        use std::sync::Arc;

        let exposed = Arc::new(Mutex::new(None::<State<AppState>>));
        let output = exposed.clone();
        let viewport = UiRect::new(0.0, 0.0, 500.0, 300.0);
        let application = ApplicationContext::empty(Default::default());
        let view_app = application.clone();
        let view: AppView = Arc::new(move |cx| {
            let mut initial = document(&format!("hello world\n{}", "line\n".repeat(40)));
            initial.focused = true;
            let state = cx.state(initial);
            let git = cx.state(GitStoreSnapshot::default());
            *output.lock().unwrap() = Some(state.clone());
            let id = UiId::new("smooth-editor-test");
            let focus = cx.focus_handle(id.clone());
            let env = crate::key_actions::KeyEnv {
                state: state.clone(),
                application: view_app.clone(),
                editor_focus: focus.clone(),
                editor_rect: viewport,
                terminal: None,
            };
            crate::key_actions::attach(
                group(viewport).child(render(PaneId::new(1), viewport, state, git, id, focus)),
                env,
            )
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let state = exposed.lock().unwrap().clone().unwrap();
        let send = |session: &mut UiSession, input| {
            dispatch_runtime_output(
                session.handle_input(input),
                &application,
                &lgui::window::WindowId::new("smooth-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
            session.render_view(&view, viewport, UiScale::ONE);
        };
        let caret = |session: &UiSession| {
            let node = session.tree().node(&UiId::new("editor-caret-1")).unwrap();
            let layer = session.tree().node(node.parent.as_ref().unwrap()).unwrap();
            let transform = layer.compositing_layer.unwrap().transform;
            node.layout_rect
                .translate(transform.translation_x(), transform.translation_y())
        };
        // UiSession has no native text service. Use the editor's existing
        // fallback metrics for the painted target, and check IME geometry separately.
        let logical = |state: &State<AppState>| {
            let app = state.get();
            let buffer = app.workspace.active_editor().unwrap();
            let (line, col) = buffer.line_col();
            let left = viewport.left + theme::GUTTER_W + theme::CODE_PAD;
            let layout = layout_line(
                buffer.line(line),
                left,
                line as f32 * theme::LINE_H,
                viewport.right,
            );
            (caret_x(layout.as_ref(), col).unwrap_or(left + col as f32 * theme::CHAR_W)
                - app.workspace.active_scroll().0)
                .round()
        };
        let left = theme::GUTTER_W + theme::CODE_PAD;
        let pointer = |column| PointerData::mouse(Point::new(left + column * theme::CHAR_W, 5.0));
        send(
            &mut session,
            InputEvent::PointerDown {
                pointer: pointer(0.0),
                button: PointerButton::Left,
            },
        );
        send(
            &mut session,
            InputEvent::PointerUp {
                pointer: pointer(0.0),
                button: PointerButton::Left,
            },
        );
        let start = caret(&session);
        send(&mut session, InputEvent::TextInput("中".into()));
        assert!(
            state
                .get()
                .workspace
                .active_editor()
                .unwrap()
                .text()
                .starts_with("中hello")
        );
        assert_eq!(
            state.get().workspace.active_editor().unwrap().cursor(),
            "中".len()
        );
        assert_eq!(caret(&session), start, "only the visual caret should lag");
        assert!(logical(&state) > start.left);
        assert_eq!(
            session
                .tree()
                .node(&UiId::new("smooth-editor-test"))
                .unwrap()
                .ime_cursor_rect
                .unwrap()
                .left
                .round(),
            ime_cursor_rect(&state.get(), viewport).left.round()
        );
        session.advance(0.0);
        session.advance(40.0);
        session.render_view(&view, viewport, UiScale::ONE);
        assert!(caret(&session).left > start.left && caret(&session).left < logical(&state));
        let displayed = caret(&session);
        send(
            &mut session,
            InputEvent::Keyboard(KeyboardEvent {
                state: KeyState::Down,
                key: LogicalKey::Named(NamedKey::ArrowRight),
                ..Default::default()
            }),
        );
        assert_eq!(
            caret(&session),
            displayed,
            "navigation retargets without jumping"
        );
        session.advance(0.0);
        session.advance(80.0);
        session.render_view(&view, viewport, UiScale::ONE);
        assert_eq!(caret(&session).left, logical(&state));

        let before_click = caret(&session);
        send(
            &mut session,
            InputEvent::PointerDown {
                pointer: pointer(8.0),
                button: PointerButton::Left,
            },
        );
        assert_eq!(
            caret(&session),
            before_click,
            "single-click placement should animate"
        );
        send(&mut session, InputEvent::PointerMove(pointer(10.0)));
        assert!(
            state
                .get()
                .workspace
                .active_editor()
                .unwrap()
                .selection()
                .is_some()
        );
        assert_eq!(
            caret(&session).left,
            logical(&state),
            "drag selection must track immediately"
        );
        send(
            &mut session,
            InputEvent::PointerUp {
                pointer: pointer(10.0),
                button: PointerButton::Left,
            },
        );
        send(&mut session, InputEvent::TextInput("x".into()));
        send(
            &mut session,
            InputEvent::Ime(ImeEvent::Preedit {
                text: "ni".into(),
                cursor: Some(0..0),
            }),
        );
        assert_eq!(
            caret(&session).left,
            logical(&state),
            "composition cancels visual motion"
        );
        send(&mut session, InputEvent::Ime(ImeEvent::Commit("你".into())));
        assert!(state.get().editor.ime.is_none());
        state.update(|app| app.smooth_caret = false);
        session.render_view(&view, viewport, UiScale::ONE);
        assert_eq!(
            caret(&session).left,
            logical(&state),
            "turning off motion snaps an active animation"
        );
        state.update(|app| {
            app.smooth_caret = true;
            app.workspace.active_editor_mut().unwrap().set_cursor(0);
        });
        session.render_view(&view, viewport, UiScale::ONE);
        state.update(|app| app.workspace.set_active_scroll(0.0, 1.0));
        session.render_view(&view, viewport, UiScale::ONE);
        assert_eq!(
            caret(&session).left,
            logical(&state),
            "scrolling snaps even while moving"
        );
        state.update(|app| app.focused = false);
        session.render_view(&view, viewport, UiScale::ONE);
        assert!(session.tree().node(&UiId::new("editor-caret-1")).is_none());
        state.update(|app| {
            app.focused = true;
            app.workspace.active_editor_mut().unwrap().set_cursor(4);
        });
        session.render_view(&view, viewport, UiScale::ONE);
        assert_eq!(
            caret(&session).left,
            logical(&state),
            "focus restoration starts at the logical caret"
        );
        state.update(|app| {
            app.workspace
                .open_path("other.txt".into(), "other file".into());
        });
        session.render_view(&view, viewport, UiScale::ONE);
        assert_eq!(
            caret(&session).left,
            logical(&state),
            "switching files snaps immediately"
        );
    }

    #[test]
    fn scrollbar_release_survives_editor_child_reordering() {
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{
            InputEvent, Point, PointerData, UiEventKind, UiEventPayload, UiScale,
            dispatch_runtime_output,
        };
        use lgui::session::UiSession;
        use std::sync::Arc;

        let exposed = Arc::new(Mutex::new(None::<State<AppState>>));
        let output = exposed.clone();
        let viewport = UiRect::new(0.0, 0.0, 500.0, 300.0);
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state(document(&format!("{}\n", "line".repeat(100)).repeat(100)));
            *output.lock().unwrap() = Some(state.clone());
            let id = cx.use_stable_id();
            let focus = cx.focus_handle(id.clone());
            let git_store = cx.state(GitStoreSnapshot::default());

            let drag_state = state.clone();
            let up_state = state.clone();
            group(viewport)
                .on_event_capture(UiEventKind::PointerMove, move |_cx, payload| {
                    if let UiEventPayload::PointerMove { pointer } = payload {
                        drag_state.try_update(|app| {
                            drag_scrollbars(app, viewport, pointer.point.x, pointer.point.y)
                        });
                    }
                })
                .on_event_capture(UiEventKind::PointerUp, move |_cx, _payload| {
                    up_state.try_update(finish_scrollbar_drag);
                })
                .child(render(
                    PaneId::new(1),
                    viewport,
                    state,
                    git_store,
                    id,
                    focus,
                ))
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let state = exposed.lock().unwrap().clone().unwrap();
        let app = ApplicationContext::empty(Default::default());
        let thumb_point = Point::new(viewport.right - 4.0, 10.0);
        let thumb_id = session.tree().hit_test(thumb_point).unwrap().id;
        let idle_style = session.tree().node(&thumb_id).unwrap().style;
        assert_eq!(session.tree().cursor_at(thumb_point), None);
        assert_eq!(
            session
                .tree()
                .cursor_at(Point::new(viewport.right - 1.0, 100.0)),
            None
        );
        assert_eq!(
            session
                .tree()
                .cursor_at(Point::new(150.0, viewport.bottom - 4.0)),
            None
        );
        assert_eq!(
            session
                .tree()
                .cursor_at(Point::new(viewport.right - 4.0, viewport.bottom - 4.0)),
            None
        );
        assert_eq!(
            session.tree().cursor_at(Point::new(150.0, 10.0)),
            Some(CursorIcon::Text)
        );
        let events = session.handle_input(InputEvent::PointerMove(PointerData::mouse(thumb_point)));
        dispatch_runtime_output(
            events,
            &app,
            &lgui::window::WindowId::new("scrollbar-test"),
            |action| session.handle_default_action(action),
            |_| {},
        );
        session.render_view(&view, viewport, UiScale::ONE);
        let hover_style = session.tree().node(&thumb_id).unwrap().style;
        assert_ne!(hover_style, idle_style);
        assert_eq!(hover_style.fill, Some(theme::c().text_soft));
        let mut send = |input| {
            let events = session.handle_input(input);
            dispatch_runtime_output(
                events,
                &app,
                &lgui::window::WindowId::new("scrollbar-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
            session.render_view(&view, viewport, UiScale::ONE);
        };
        let pointer = |x, y| PointerData::mouse(Point::new(x, y));

        send(InputEvent::PointerDown {
            pointer: pointer(viewport.right - 4.0, 10.0),
            button: PointerButton::Left,
        });
        send(InputEvent::PointerMove(pointer(100.0, 260.0)));
        assert!(state.get().editor_vertical_scrollbar_dragging);
        assert!(state.get().workspace.active_scroll().1 > 0.0);

        send(InputEvent::PointerUp {
            pointer: pointer(100.0, 260.0),
            button: PointerButton::Left,
        });
        assert!(!state.get().editor_vertical_scrollbar_dragging);
        let released_scroll = state.get().workspace.active_scroll().1;

        send(InputEvent::PointerMove(pointer(100.0, 20.0)));
        assert_eq!(state.get().workspace.active_scroll().1, released_scroll);
    }

    #[test]
    fn clipboard_cut_paste_and_undo_preserve_unicode_and_selection() {
        let mut app = document("你好 world");
        let clipboard = TestClipboard::default();
        app.workspace
            .active_editor_mut()
            .unwrap()
            .select_range(0..6);
        apply_command(&mut app, Command::Copy, &clipboard).unwrap();
        assert_eq!(clipboard.read_text().unwrap().as_deref(), Some("你好"));
        apply_command(&mut app, Command::Cut, &clipboard).unwrap();
        assert_eq!(app.workspace.active_editor().unwrap().text(), " world");
        apply_command(&mut app, Command::Undo, &clipboard).unwrap();
        assert_eq!(
            app.workspace.active_editor().unwrap().selected_text(),
            Some("你好")
        );
        clipboard.write_text("再见").unwrap();
        apply_command(&mut app, Command::Paste, &clipboard).unwrap();
        assert_eq!(app.workspace.active_editor().unwrap().text(), "再见 world");
    }

    #[test]
    fn clipboard_failure_and_empty_clipboard_never_destroy_selection() {
        let mut app = document("keep this");
        app.workspace.active_editor_mut().unwrap().select_all();
        let unavailable = TestClipboard {
            fail: true,
            ..Default::default()
        };
        for command in [Command::Cut, Command::Copy, Command::Paste] {
            assert!(apply_command(&mut app, command, &unavailable).is_err());
            assert_eq!(
                app.workspace.active_editor().unwrap().selected_text(),
                Some("keep this")
            );
        }
        apply_command(&mut app, Command::Paste, &TestClipboard::default()).unwrap();
        assert_eq!(
            app.workspace.active_editor().unwrap().selected_text(),
            Some("keep this")
        );
        assert!(!app.workspace.active_editor().unwrap().can_undo());
    }

    #[test]
    fn editing_commands_promote_a_preview_while_navigation_does_not() {
        let mut app = AppState::new();
        let preview = app
            .workspace
            .preview_path("preview.txt".into(), "hello".into());
        let clipboard = TestClipboard::default();

        apply_command(
            &mut app,
            Command::Move(normal::Movement::Right, false),
            &clipboard,
        )
        .unwrap();
        assert_eq!(app.workspace.preview(), Some(preview));
        apply_command(&mut app, Command::Undo, &clipboard).unwrap();
        assert_eq!(app.workspace.preview(), Some(preview));

        clipboard.write_text("!").unwrap();
        apply_command(&mut app, Command::Paste, &clipboard).unwrap();
        assert_ne!(app.workspace.preview(), Some(preview));
        assert_eq!(app.workspace.active_editor().unwrap().text(), "h!ello");
    }

    #[test]
    fn a_failed_cut_does_not_promote_or_modify_a_preview() {
        let mut app = AppState::new();
        let preview = app
            .workspace
            .preview_path("preview.txt".into(), "hello".into());
        app.workspace.active_editor_mut().unwrap().select_all();
        let unavailable = TestClipboard {
            fail: true,
            ..Default::default()
        };

        assert!(apply_command(&mut app, Command::Cut, &unavailable).is_err());
        assert_eq!(app.workspace.preview(), Some(preview));
        assert_eq!(app.workspace.active_editor().unwrap().text(), "hello");
    }

    #[test]
    fn tab_expansion_keeps_source_indices_and_visual_columns_in_sync() {
        let (display, mapping, width) = expand_tabs("a\t中\tb", 0);
        assert_eq!(display, "a   中  b");
        assert_eq!(mapping, vec![0, 1, 4, 5, 7, 8]);
        assert_eq!(width, 9);
        assert_eq!(fallback_hit_column("a\t中\tb", 5.9 * theme::CHAR_W), 3);
        assert_eq!(fallback_hit_column("👩‍💻x", 1.5 * theme::CHAR_W), 3);
    }

    #[test]
    fn held_pointer_scrolls_and_extends_selection_without_mouse_movement() {
        let mut app = document(&(0..100).map(|i| format!("line {i}\n")).collect::<String>());
        let rect = UiRect::new(0.0, 0.0, 400.0, 100.0);
        app.editor.drag = Some(DragSelection {
            document: app.workspace.active().unwrap(),
            origin: 0..0,
            unit: SelectionUnit::Character,
            point: (100.0, 150.0),
        });
        assert!(drag_scroll_tick(&mut app, rect));
        let first = app.workspace.active_scroll().1;
        let first_selection = app.workspace.active_editor().unwrap().selection().unwrap();
        assert!(drag_scroll_tick(&mut app, rect));
        assert!(app.workspace.active_scroll().1 > first);
        assert!(
            app.workspace
                .active_editor()
                .unwrap()
                .selection()
                .unwrap()
                .end
                > first_selection.end
        );
        app.editor.drag = None;
        assert!(!drag_scroll_tick(&mut app, rect));
    }

    #[test]
    fn scrollbar_drag_uses_window_pointer_outside_the_scrollbar_strip() {
        let mut app = document(&(0..100).map(|i| format!("line {i}\n")).collect::<String>());
        let rect = UiRect::new(0.0, 0.0, 400.0, 100.0);
        app.editor_vertical_scrollbar_dragging = true;
        app.editor_vertical_scrollbar_drag_offset = 4.0;

        assert!(drag_scrollbars(&mut app, rect, 40.0, 500.0));
        let bottom = app.workspace.active_scroll().1;
        assert!(bottom > 0.0);

        assert!(drag_scrollbars(&mut app, rect, 40.0, rect.top));
        assert_eq!(app.workspace.active_scroll().1, 0.0);
        assert!(finish_scrollbar_drag(&mut app));
        assert!(!app.editor_vertical_scrollbar_dragging);
        assert!(!finish_scrollbar_drag(&mut app));

        let mut app = document(&"x".repeat(1_000));
        app.editor_horizontal_scrollbar_dragging = true;
        app.editor_horizontal_scrollbar_drag_offset = 4.0;

        assert!(drag_scrollbars(&mut app, rect, 1_000.0, 40.0));
        assert!(app.workspace.active_scroll().0 > 0.0);
        assert!(drag_scrollbars(&mut app, rect, rect.left, 40.0));
        assert_eq!(app.workspace.active_scroll().0, 0.0);
        assert!(finish_scrollbar_drag(&mut app));
        assert!(!app.editor_horizontal_scrollbar_dragging);
    }

    #[test]
    fn gutter_drag_selects_whole_lines_in_both_directions() {
        let mut app = document("one\ntwo\nthree");
        let rect = UiRect::new(0.0, 0.0, 400.0, 300.0);
        app.editor.drag = Some(DragSelection {
            document: app.workspace.active().unwrap(),
            origin: 4..8,
            unit: SelectionUnit::Line,
            point: (5.0, 1.0),
        });
        update_drag_selection(&mut app, rect);
        assert_eq!(
            app.workspace.active_editor().unwrap().selected_text(),
            Some("one\ntwo\n")
        );
        app.editor.drag.as_mut().unwrap().point.1 = theme::LINE_H * 2.0 + 1.0;
        update_drag_selection(&mut app, rect);
        assert_eq!(
            app.workspace.active_editor().unwrap().selected_text(),
            Some("two\nthree")
        );
    }

    #[test]
    fn switching_documents_cancels_drag_and_keeps_undo_histories_independent() {
        let mut app = document("first");
        let first = app.workspace.active().unwrap();
        normal::insert(&mut app.workspace.active_editor_mut().unwrap(), "A");
        app.editor.drag = Some(DragSelection {
            document: first,
            origin: 0..0,
            unit: SelectionUnit::Character,
            point: (100.0, 500.0),
        });
        app.workspace
            .open_path("second.txt".into(), "second".into());
        normal::insert(&mut app.workspace.active_editor_mut().unwrap(), "B");
        assert!(drag_scroll_tick(
            &mut app,
            UiRect::new(0.0, 0.0, 400.0, 100.0)
        ));
        assert!(app.editor.drag.is_none());
        app.workspace.active_editor_mut().unwrap().undo();
        assert_eq!(app.workspace.active_editor().unwrap().text(), "second");
        app.workspace.set_active(first);
        assert_eq!(app.workspace.active_editor().unwrap().text(), "Afirst");
        app.workspace.active_editor_mut().unwrap().undo();
        assert_eq!(app.workspace.active_editor().unwrap().text(), "first");
    }

    #[test]
    fn a_late_composition_commit_lands_in_the_view_it_started_in() {
        let rect = UiRect::new(0.0, 0.0, 400.0, 300.0);
        let mut app = document("first");
        let first = app.workspace.active().unwrap();
        let left = app.workspace.active_pane();
        update_composition(&mut app, "ni", None);
        assert!(app.editor.preedit_for(left, first).is_some());

        let second = app
            .workspace
            .open_path("second.txt".into(), "second".into());
        assert!(app.editor.preedit_for(left, second).is_none());
        insert_text(&mut app, "你", rect);
        assert_eq!(app.workspace.active(), Some(second));
        assert_eq!(app.workspace.active_editor().unwrap().text(), "second");
        app.workspace.set_active(first);
        assert_eq!(app.workspace.active_editor().unwrap().text(), "你first");
        assert!(app.editor.ime.is_none());

        // Plain typing without a composition goes to the active view.
        insert_text(&mut app, "x", rect);
        assert_eq!(app.workspace.active_editor().unwrap().text(), "你xfirst");
    }

    #[test]
    fn a_commit_for_a_closed_view_is_dropped() {
        let rect = UiRect::new(0.0, 0.0, 400.0, 300.0);
        let mut app = document("first");
        let first = app.workspace.active().unwrap();
        update_composition(&mut app, "ni", None);
        app.workspace
            .open_path("second.txt".into(), "second".into());
        app.workspace.close(first);
        insert_text(&mut app, "你", rect);
        assert_eq!(app.workspace.active_editor().unwrap().text(), "second");
    }

    #[test]
    fn an_emptied_composition_moves_to_the_view_that_starts_the_next_one() {
        let mut app = document("first");
        let left = app.workspace.active_pane();
        let first = app.workspace.active().unwrap();
        update_composition(&mut app, "ni", None);
        update_composition(&mut app, "", None);
        let right = app
            .workspace
            .split(left, crate::model::pane_layout::Direction::Right)
            .unwrap();
        update_composition(&mut app, "hao", None);
        assert!(app.editor.preedit_for(right, first).is_some());
        assert!(app.editor.preedit_for(left, first).is_none());
    }

    #[test]
    fn keyboard_reveal_scrolls_to_the_document_end() {
        let mut app = document(&"long line\n".repeat(100));
        app.workspace
            .active_editor_mut()
            .unwrap()
            .select_to(usize::MAX, false);
        reveal_cursor(&mut app, UiRect::new(0.0, 0.0, 400.0, 100.0));
        assert_eq!(
            app.workspace.active_scroll().1,
            101.0 * theme::LINE_H - 100.0
        );
    }

    #[test]
    fn visible_range_keeps_only_rows_around_the_viewport() {
        assert_eq!(visible_line_range(240.0, 120.0, 100), 9..16);
    }

    #[test]
    fn thumb_geometry_maps_the_full_scroll_range_to_the_track() {
        let start = thumb_geometry(10.0, 100.0, 100.0, 400.0, 0.0);
        let end = thumb_geometry(10.0, 100.0, 100.0, 400.0, 300.0);

        assert_eq!(
            start,
            ThumbGeometry {
                start: 10.0,
                length: 28.0
            }
        );
        assert_eq!(
            end,
            ThumbGeometry {
                start: 82.0,
                length: 28.0
            }
        );
    }

    #[test]
    fn visual_columns_expand_tabs_and_wide_characters() {
        assert_eq!(visual_columns("a\tb"), 5);
        assert_eq!(visual_columns("a界b"), 4);
    }

    #[test]
    fn pointer_line_accounts_for_vertical_scroll_and_clamps_to_document() {
        assert_eq!(
            line_index_from_point(100.0, 100.0, theme::LINE_H * 2.0, 10),
            2
        );
        assert_eq!(line_index_from_point(50.0, 100.0, 0.0, 10), 0);
        assert_eq!(line_index_from_point(900.0, 100.0, 0.0, 10), 9);
    }
}
