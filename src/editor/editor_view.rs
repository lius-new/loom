//! The central code viewport: gutter, syntax-highlighted source, cursor and
//! editor-specific overlay scrollbars.

use std::fs;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use lgui::core::{
    CursorIcon, EventPolicy, KeyState, KeyboardEvent, LogicalKey, PointerButton, SemanticRole,
    Semantics, UiElement, UiFocusHandle, UiId, WheelUnit, clip,
};
use lgui::prelude::{Element, State, UiRect, VisualStyle, group, panel, text};
use lgui::text::{self, TextLayout, TextLayoutRequest};

use super::commands::{self, Command};
use super::interaction::{DragSelection, SelectionUnit};
use crate::editor::gutter;
use crate::editor::syntax;
use crate::model::buffer::TextBuffer;
use crate::state::AppState;
use crate::theme;
use lgui::services::ServicesContextExt;

const SCROLLBAR_SIZE: f32 = 12.0;
const SCROLLBAR_INSET: f32 = 2.0;
const MIN_THUMB_LENGTH: f32 = 28.0;
const TRAILING_CODE_SPACE: f32 = 32.0;
const CURSOR_REVEAL_MARGIN: f32 = 12.0;

#[derive(Clone, Copy, Debug)]
struct ScrollMetrics {
    viewport_w: f32,
    viewport_h: f32,
    content_w: f32,
    content_h: f32,
    max_x: f32,
    max_y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ThumbGeometry {
    start: f32,
    length: f32,
}

/// Render the active file's editor surface into `rect`.
pub fn render(
    rect: UiRect,
    state: State<AppState>,
    editor_id: UiId,
    editor_focus: UiFocusHandle,
) -> Element {
    let s = state.get();

    let ime_rect = ime_cursor_rect(&s, rect);
    let mut root = Element::new(move |cx| {
        UiElement::panel(editor_id, rect, VisualStyle::filled(theme::BG))
            .ime_cursor_rect(ime_rect)
            .semantics(Semantics::new(SemanticRole::TextInput).name("Code editor"))
            .children(cx.children)
    })
    .event_policy(EventPolicy::INTERACTIVE)
    .cursor(CursorIcon::Text);

    if let Some(id) = s.workspace.active() {
        let meta = s
            .workspace
            .meta(id)
            .expect("active document metadata exists");
        let buffer = s.workspace.active_buffer().expect("active buffer exists");
        let lines = document_lines(buffer);
        let (cursor_line, cursor_col) = buffer.line_col();
        let code_top = rect.top;
        let code_left = rect.left + theme::GUTTER_W + theme::CODE_PAD;
        let metrics = scroll_metrics(rect, code_left, &lines);
        let (stored_x, stored_y) = s.workspace.active_scroll();
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
                let (scroll_x, scroll_y) = app.workspace.active_scroll();
                let cursor = app.workspace.active_buffer().map(|buffer| {
                    cursor_offset_from_point(
                        buffer, rect, code_left, metrics, scroll_x, scroll_y, point.x, point.y,
                    )
                });
                if let Some(cursor) = cursor
                    && let Some(buffer) = app.workspace.active_buffer_mut()
                {
                    if button == PointerButton::Right {
                        if !buffer.selection().is_some_and(|r| r.contains(&cursor)) {
                            buffer.set_cursor(cursor);
                        }
                        app.editor.menu = Some((point.x, point.y));
                        app.editor.menu_hover = None;
                        app.editor.drag = None;
                        app.context_menu = None;
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
                                buffer.line_range(line)
                            }
                            SelectionUnit::Word => buffer.word_range(cursor),
                            SelectionUnit::Character => cursor..cursor,
                        };
                        if unit == SelectionUnit::Character {
                            buffer.select_to(cursor, shift);
                        } else {
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
                reveal_cursor(app, rect);
            });
            cx.stop_propagation();
        });

        let st_drag = state.clone();
        root = root.on_pointer_drag(move |cx, pointer| {
            st_drag.update(|app| {
                if let Some(drag) = app.editor.drag.as_mut() {
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
                let (x, y) = app.workspace.active_scroll();
                app.workspace.set_active_scroll(
                    (x - step_x).clamp(0.0, metrics.max_x),
                    (y - step_y).clamp(0.0, metrics.max_y),
                );
            });
            cx.stop_propagation();
        });

        if visible_lines.contains(&cursor_line) {
            let highlight_y = code_top + cursor_line as f32 * theme::LINE_H;
            let highlight = panel(
                UiRect::new(
                    rect.left + theme::GUTTER_W,
                    highlight_y,
                    rect.right,
                    highlight_y + theme::LINE_H,
                ),
                VisualStyle::filled(theme::ACTIVE_LINE),
            );
            root = root.child(clip(rect, 0.0, -scroll_y).child(highlight));
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
        root = root.child(clip(gutter_viewport, 0.0, -scroll_y).child(gutter::render(
            gutter_content,
            visible_lines.clone(),
            None,
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

        let line_bounds = buffer.line_bounds();
        for line_index in visible_lines.clone() {
            let y = code_top + line_index as f32 * theme::LINE_H;
            let line = lines[line_index];
            let layout = layout_line(line, code_left, y, layout_right);
            if let Some(selection) = buffer.selection() {
                let (start, end) = line_bounds[line_index];
                let next = line_bounds
                    .get(line_index + 1)
                    .map_or(buffer.text().len(), |b| b.0);
                if selection.start < next && selection.end > start {
                    let from = selection.start.max(start).min(end);
                    let to = selection.end.min(end).max(start);
                    let left = caret_x(layout.as_ref(), buffer.text()[start..from].chars().count())
                        .unwrap_or(code_left);
                    let mut right =
                        caret_x(layout.as_ref(), buffer.text()[start..to].chars().count())
                            .unwrap_or(left);
                    if selection.end > end {
                        right += theme::CHAR_W;
                    }
                    code = code.child(panel(
                        UiRect::new(left, y, right.max(left + 2.0), y + theme::LINE_H),
                        VisualStyle::filled(theme::ACCENT).alpha(55),
                    ));
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

        if visible_lines.contains(&cursor_line) {
            let cursor_y = code_top + cursor_line as f32 * theme::LINE_H;
            let cursor_layout = layout_line(lines[cursor_line], code_left, cursor_y, layout_right);
            let cursor_x = caret_x(cursor_layout.as_ref(), cursor_col)
                .unwrap_or(code_left + cursor_col as f32 * theme::CHAR_W);
            let cursor_rect = UiRect::new(
                cursor_x,
                cursor_y + 3.0,
                cursor_x + 2.0,
                cursor_y + theme::LINE_H - 3.0,
            );
            let cursor_style = if s.focused {
                VisualStyle::filled(theme::ACCENT)
            } else {
                VisualStyle::default().stroked(theme::hairline(theme::ACCENT))
            };
            code = code.child(panel(cursor_rect, cursor_style));
            if !s.editor.preedit.is_empty() && s.focused {
                let width = layout_line(&s.editor.preedit, 0.0, 0.0, metrics.content_w)
                    .map_or(80.0, |l| l.width)
                    .max(2.0);
                let r = UiRect::new(
                    cursor_x,
                    cursor_y,
                    cursor_x + width + 4.0,
                    cursor_y + theme::LINE_H,
                );
                code = code.child(
                    panel(r, VisualStyle::filled(theme::SURFACE))
                        .child(text(
                            r,
                            s.editor.preedit.clone(),
                            theme::mono(theme::ZINC_100, theme::CODE_SIZE),
                        ))
                        .child(panel(
                            UiRect::new(r.left, r.bottom - 2.0, r.right, r.bottom - 1.0),
                            VisualStyle::filled(theme::ACCENT),
                        )),
                );
            }
        }

        root = root.child(clip(code_viewport, -scroll_x, -scroll_y).child(code));

        let emphasized = s.editor_hovered
            || s.editor_vertical_scrollbar_dragging
            || s.editor_horizontal_scrollbar_dragging;
        if has_vertical {
            root = root.child(vertical_scrollbar(
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
                rect,
                code_left,
                metrics,
                scroll_x,
                has_vertical,
                emphasized,
                state.clone(),
            ));
        }
    } else {
        root = root.child(crate::ui::welcome::render(
            rect,
            state.clone(),
            editor_focus.clone(),
        ));
    }

    let st_input = state.clone();
    root = root.on_input(move |ctx, input| {
        let input = input.to_string();
        st_input.update(move |app| {
            app.editor.drag = None;
            if let Some(buffer) = app.workspace.active_buffer_mut() {
                if app.editor.ime_pending {
                    buffer.break_undo_group();
                }
                buffer.insert(&input);
                if app.editor.ime_pending {
                    buffer.break_undo_group();
                }
            }
            app.editor.ime_pending = false;
            app.editor.preedit.clear();
            app.editor.preedit_cursor = None;
            app.editor.menu = None;
            reveal_cursor(app, rect);
        });
        ctx.stop_propagation();
    });

    let st_ime = state.clone();
    root = root.on_composition_update(move |_, value, cursor| {
        st_ime.update(|app| {
            if !value.is_empty() {
                app.editor.ime_pending = true;
            }
            app.editor.preedit = value.to_owned();
            app.editor.preedit_cursor = cursor;
        })
    });
    let st_ime_end = state.clone();
    root = root.on_composition_end(move |_| {
        st_ime_end.update(|app| {
            app.editor.ime_pending = false;
            app.editor.preedit.clear();
            app.editor.preedit_cursor = None;
        })
    });

    let st_key = state.clone();
    root = root.on_key_down(move |ctx, event| {
        if !st_key.get().editor.preedit.is_empty() {
            return;
        }
        if is_save_shortcut(event) {
            save_active_document(&st_key);
            ctx.prevent_default();
            ctx.stop_propagation();
        } else if let Some(command) = commands::command_for(
            event,
            (rect.height() / theme::LINE_H).floor().max(1.0) as usize,
        ) {
            execute_command(&st_key, command, rect, ctx.application().clipboard());
            ctx.prevent_default();
            ctx.stop_propagation();
        } else if (event.modifiers.ctrl() || event.modifiers.meta()) && !event.modifiers.alt() {
            // Let editor-wide shortcuts bubble, while suppressing printable key text.
            ctx.prevent_default();
        }
    });

    let st_focus = state.clone();
    root = root.on_focus(move |_ctx| st_focus.update(|app| app.focused = true));

    let st_blur = state.clone();
    root = root.on_blur(move |_ctx| {
        st_blur.update(|app| {
            app.focused = false;
            app.editor.drag = None;
            app.editor.ime_pending = false;
            app.editor.preedit.clear();
            app.editor.preedit_cursor = None;
            if let Some(buffer) = app.workspace.active_buffer_mut() {
                buffer.break_undo_group();
            }
        })
    });

    root
}

fn vertical_scrollbar(
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
    let track_hit = panel(track, VisualStyle::default())
        .key("editor-vertical-scrollbar-track")
        .event_policy(EventPolicy::INTERACTIVE)
        .on_pointer_down(move |_cx, pointer| {
            let offset = geometry.length / 2.0;
            let thumb_start = (pointer.point.y - track.top - offset).clamp(0.0, travel);
            let next = scroll_from_thumb(thumb_start, travel, metrics.max_y);
            st_track_down.update(move |app| {
                let (x, _) = app.workspace.active_scroll();
                app.workspace.set_active_scroll(x, next);
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
                let (x, _) = app.workspace.active_scroll();
                app.workspace.set_active_scroll(x, next);
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
    let thumb = panel(
        thumb_rect,
        VisualStyle::filled(theme::ZINC_500)
            .alpha(alpha)
            .radius(2.0),
    )
    .key("editor-vertical-scrollbar-thumb")
    .event_policy(EventPolicy::INTERACTIVE)
    .on_pointer_down(move |_cx, pointer| {
        st_thumb_down.update(move |app| {
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
            let (x, _) = app.workspace.active_scroll();
            app.workspace.set_active_scroll(x, next);
        });
    })
    .on_pointer_up(move |_cx, _pointer| {
        st_thumb_up.update(|app| app.editor_vertical_scrollbar_dragging = false);
    });

    group(track).child(track_hit).child(thumb)
}

fn horizontal_scrollbar(
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
    let track_hit = panel(track, VisualStyle::default())
        .key("editor-horizontal-scrollbar-track")
        .event_policy(EventPolicy::INTERACTIVE)
        .on_pointer_down(move |_cx, pointer| {
            let offset = geometry.length / 2.0;
            let thumb_start = (pointer.point.x - track.left - offset).clamp(0.0, travel);
            let next = scroll_from_thumb(thumb_start, travel, metrics.max_x);
            st_track_down.update(move |app| {
                let (_, y) = app.workspace.active_scroll();
                app.workspace.set_active_scroll(next, y);
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
                let (_, y) = app.workspace.active_scroll();
                app.workspace.set_active_scroll(next, y);
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
    let thumb = panel(
        thumb_rect,
        VisualStyle::filled(theme::ZINC_500)
            .alpha(alpha)
            .radius(2.0),
    )
    .key("editor-horizontal-scrollbar-thumb")
    .event_policy(EventPolicy::INTERACTIVE)
    .on_pointer_down(move |_cx, pointer| {
        st_thumb_down.update(move |app| {
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
            let (_, y) = app.workspace.active_scroll();
            app.workspace.set_active_scroll(next, y);
        });
    })
    .on_pointer_up(move |_cx, _pointer| {
        st_thumb_up.update(|app| app.editor_horizontal_scrollbar_dragging = false);
    });

    group(track).child(track_hit).child(thumb)
}

fn document_lines(buffer: &TextBuffer) -> Vec<&str> {
    buffer
        .line_bounds()
        .into_iter()
        .map(|(a, b)| &buffer.text()[a..b])
        .collect()
}

fn scroll_metrics(rect: UiRect, code_left: f32, lines: &[&str]) -> ScrollMetrics {
    let mut viewport_w = (rect.right - code_left).max(1.0);
    let mut viewport_h = rect.height().max(1.0);
    let longest_columns = lines
        .iter()
        .map(|line| visual_columns(line))
        .max()
        .unwrap_or(0);
    let natural_w = longest_columns as f32 * theme::CHAR_W + TRAILING_CODE_SPACE;
    let content_h = (lines.len() as f32 * theme::LINE_H).max(theme::LINE_H);
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
    crate::model::buffer::display_columns(line)
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
fn expand_tabs(line: &str, initial_column: usize) -> (String, Vec<usize>, usize) {
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
    let lines = document_lines(buffer);
    let line = line_index_from_point(point_y, rect.top, scroll_y, lines.len());
    let line_text = lines[line];
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
    let line_start = buffer.line_bounds()[line].0;
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

fn reveal_cursor(app: &mut AppState, rect: UiRect) {
    let Some(buffer) = app.workspace.active_buffer() else {
        return;
    };
    let lines = document_lines(buffer);
    let (line, column) = buffer.line_col();
    let code_left = rect.left + theme::GUTTER_W + theme::CODE_PAD;
    let metrics = scroll_metrics(rect, code_left, &lines);
    let (mut scroll_x, mut scroll_y) = app.workspace.active_scroll();

    let line_top = line as f32 * theme::LINE_H;
    let line_bottom = line_top + theme::LINE_H;
    if line_top < scroll_y {
        scroll_y = line_top;
    } else if line_bottom > scroll_y + metrics.viewport_h {
        scroll_y = line_bottom - metrics.viewport_h;
    }

    let layout = layout_line(
        lines[line],
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
        if let Err(error) = apply_command(app, command, clipboard.as_ref()) {
            app.show_toast(format!("Clipboard: {error}"));
        }
        reveal_cursor(app, rect);
    });
}

fn apply_command(
    app: &mut AppState,
    command: Command,
    clipboard: &dyn lgui::services::Clipboard,
) -> Result<(), lgui::services::ClipboardError> {
    let Some(buffer) = app.workspace.active_buffer_mut() else {
        return Ok(());
    };
    match command {
        Command::Copy | Command::Cut => {
            if let Some(value) = buffer.selected_text() {
                // A failed copy must leave the source text and selection untouched.
                clipboard.write_text(value)?;
                buffer.break_undo_group();
                if command == Command::Cut {
                    buffer.backspace();
                    buffer.break_undo_group();
                }
            }
        }
        Command::Paste => {
            if let Some(value) = clipboard.read_text()? {
                buffer.paste(&value);
            }
        }
        _ => commands::apply(buffer, command),
    }
    Ok(())
}

fn ime_cursor_rect(app: &AppState, rect: UiRect) -> UiRect {
    let Some(buffer) = app.workspace.active_buffer() else {
        return rect;
    };
    let (line, col) = buffer.line_col();
    let (sx, sy) = app.workspace.active_scroll();
    let left = rect.left + theme::GUTTER_W + theme::CODE_PAD;
    let top = rect.top + line as f32 * theme::LINE_H - sy;
    let layout = layout_line(buffer.line(line), left, top, left + 100000.0);
    let mut x = caret_x(layout.as_ref(), col).unwrap_or(left) - sx;
    if !app.editor.preedit.is_empty() {
        let index = app
            .editor
            .preedit_cursor
            .as_ref()
            .map_or(app.editor.preedit.len(), |r| r.start)
            .min(app.editor.preedit.len());
        let prefix = app.editor.preedit.get(..index).unwrap_or("");
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
    if let Some(buffer) = app.workspace.active_buffer_mut() {
        let metrics = scroll_metrics(rect, left, &document_lines(buffer));
        let cursor = cursor_offset_from_point(
            buffer,
            rect,
            left,
            metrics,
            sx,
            sy,
            drag.point.0,
            drag.point.1,
        );
        match drag.unit {
            SelectionUnit::Character => buffer.select_to(cursor, true),
            SelectionUnit::Word | SelectionUnit::Line => {
                let range = if drag.unit == SelectionUnit::Word {
                    buffer.word_range(cursor)
                } else {
                    buffer.line_range(line_index_from_point(
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
    let Some(buffer) = app.workspace.active_buffer() else {
        return false;
    };
    let metrics = scroll_metrics(rect, left, &document_lines(buffer));
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

fn is_save_shortcut(event: &KeyboardEvent) -> bool {
    event.state == KeyState::Down
        && (event.modifiers.ctrl() || event.modifiers.meta())
        && matches!(
            &event.key,
            LogicalKey::Character(value) if value.eq_ignore_ascii_case("s")
        )
}

fn save_active_document(state: &State<AppState>) {
    let Some((id, path, contents)) = state.get().workspace.active_save_snapshot() else {
        return;
    };

    match fs::write(&path, contents.as_bytes()) {
        Ok(()) => state.update(move |app| {
            app.workspace.mark_saved(id);
        }),
        Err(error) => state.update(move |app| {
            app.show_toast(format!("Could not save {}: {error}", path.display()));
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgui::core::KeyModifiers;
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
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state(document("hello world\nsecond"));
            *output.lock().unwrap() = Some(state.clone());
            let id = cx.use_stable_id();
            let focus = cx.focus_handle(id.clone());
            render(viewport, state, id, focus)
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let state = exposed.lock().unwrap().clone().unwrap();
        let app = ApplicationContext::empty(Default::default());
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
                .active_buffer()
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
            state.get().workspace.active_buffer().unwrap().text(),
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
                .active_buffer()
                .unwrap()
                .selected_text(),
            Some("hello")
        );
        send(InputEvent::Ime(ImeEvent::Preedit {
            text: "nihao".into(),
            cursor: Some(0..5),
        }));
        assert_eq!(
            state.get().workspace.active_buffer().unwrap().text(),
            "hello world\nsecond"
        );
        send(InputEvent::Ime(ImeEvent::Commit("你好".into())));
        assert_eq!(
            state.get().workspace.active_buffer().unwrap().text(),
            "你好 world\nsecond"
        );
        assert!(state.get().editor.preedit.is_empty());
        send(key(
            LogicalKey::Character("z".into()),
            KeyModifiers::CONTROL,
        ));
        assert_eq!(
            state
                .get()
                .workspace
                .active_buffer()
                .unwrap()
                .selected_text(),
            Some("hello")
        );
    }

    #[test]
    fn clipboard_cut_paste_and_undo_preserve_unicode_and_selection() {
        let mut app = document("你好 world");
        let clipboard = TestClipboard::default();
        app.workspace
            .active_buffer_mut()
            .unwrap()
            .select_range(0..6);
        apply_command(&mut app, Command::Copy, &clipboard).unwrap();
        assert_eq!(clipboard.read_text().unwrap().as_deref(), Some("你好"));
        apply_command(&mut app, Command::Cut, &clipboard).unwrap();
        assert_eq!(app.workspace.active_buffer().unwrap().text(), " world");
        apply_command(&mut app, Command::Undo, &clipboard).unwrap();
        assert_eq!(
            app.workspace.active_buffer().unwrap().selected_text(),
            Some("你好")
        );
        clipboard.write_text("再见").unwrap();
        apply_command(&mut app, Command::Paste, &clipboard).unwrap();
        assert_eq!(app.workspace.active_buffer().unwrap().text(), "再见 world");
    }

    #[test]
    fn clipboard_failure_and_empty_clipboard_never_destroy_selection() {
        let mut app = document("keep this");
        app.workspace.active_buffer_mut().unwrap().select_all();
        let unavailable = TestClipboard {
            fail: true,
            ..Default::default()
        };
        for command in [Command::Cut, Command::Copy, Command::Paste] {
            assert!(apply_command(&mut app, command, &unavailable).is_err());
            assert_eq!(
                app.workspace.active_buffer().unwrap().selected_text(),
                Some("keep this")
            );
        }
        apply_command(&mut app, Command::Paste, &TestClipboard::default()).unwrap();
        assert_eq!(
            app.workspace.active_buffer().unwrap().selected_text(),
            Some("keep this")
        );
        assert!(!app.workspace.active_buffer().unwrap().can_undo());
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
        let first_selection = app.workspace.active_buffer().unwrap().selection().unwrap();
        assert!(drag_scroll_tick(&mut app, rect));
        assert!(app.workspace.active_scroll().1 > first);
        assert!(
            app.workspace
                .active_buffer()
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
            app.workspace.active_buffer().unwrap().selected_text(),
            Some("one\ntwo\n")
        );
        app.editor.drag.as_mut().unwrap().point.1 = theme::LINE_H * 2.0 + 1.0;
        update_drag_selection(&mut app, rect);
        assert_eq!(
            app.workspace.active_buffer().unwrap().selected_text(),
            Some("two\nthree")
        );
    }

    #[test]
    fn switching_documents_cancels_drag_and_keeps_undo_histories_independent() {
        let mut app = document("first");
        let first = app.workspace.active().unwrap();
        app.workspace.active_buffer_mut().unwrap().insert("A");
        app.editor.drag = Some(DragSelection {
            document: first,
            origin: 0..0,
            unit: SelectionUnit::Character,
            point: (100.0, 500.0),
        });
        app.workspace
            .open_path("second.txt".into(), "second".into());
        app.workspace.active_buffer_mut().unwrap().insert("B");
        assert!(drag_scroll_tick(
            &mut app,
            UiRect::new(0.0, 0.0, 400.0, 100.0)
        ));
        assert!(app.editor.drag.is_none());
        app.workspace.active_buffer_mut().unwrap().undo();
        assert_eq!(app.workspace.active_buffer().unwrap().text(), "second");
        app.workspace.set_active(first);
        assert_eq!(app.workspace.active_buffer().unwrap().text(), "Afirst");
        app.workspace.active_buffer_mut().unwrap().undo();
        assert_eq!(app.workspace.active_buffer().unwrap().text(), "first");
    }

    #[test]
    fn keyboard_reveal_scrolls_to_the_document_end() {
        let mut app = document(&"long line\n".repeat(100));
        app.workspace
            .active_buffer_mut()
            .unwrap()
            .navigate(crate::model::buffer::Movement::Finish, false);
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

    #[test]
    fn save_shortcut_accepts_control_or_command_s() {
        let event = |modifiers| KeyboardEvent {
            state: KeyState::Down,
            key: LogicalKey::Character("s".into()),
            modifiers,
            ..Default::default()
        };

        assert!(is_save_shortcut(&event(KeyModifiers::CONTROL)));
        assert!(is_save_shortcut(&event(KeyModifiers::META)));
        assert!(!is_save_shortcut(&event(KeyModifiers::SHIFT)));
    }
}
