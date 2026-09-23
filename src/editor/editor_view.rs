//! The central code viewport: gutter, syntax-highlighted source, cursor and
//! editor-specific overlay scrollbars.

use std::fs;
use std::ops::Range;

use lgui::core::{
    EventPolicy, KeyState, KeyboardEvent, LogicalKey, NamedKey, PointerButton, UiElement,
    UiFocusHandle, UiId, WheelUnit, clip,
};
use lgui::prelude::{Element, State, UiRect, VisualStyle, group, panel, text};
use lgui::text::{self, TextLayout, TextLayoutRequest};

use crate::editor::gutter;
use crate::editor::syntax;
use crate::model::buffer::TextBuffer;
use crate::state::AppState;
use crate::theme;

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

#[derive(Clone, Copy)]
enum EditCommand {
    Backspace,
    Delete,
    Enter,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

/// Render the active file's editor surface into `rect`.
pub fn render(
    rect: UiRect,
    state: State<AppState>,
    editor_id: UiId,
    editor_focus: UiFocusHandle,
) -> Element {
    let s = state.get();

    let mut root = Element::new(move |cx| {
        UiElement::panel(editor_id, rect, VisualStyle::filled(theme::BG)).children(cx.children)
    })
    .event_policy(EventPolicy::INTERACTIVE);

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
        root = root.on_pointer_down_with_button(move |cx, pointer, button| {
            let point = pointer.point;
            let over_vertical_scrollbar =
                has_vertical && point.x >= rect.right - SCROLLBAR_SIZE;
            let over_horizontal_scrollbar =
                has_horizontal && point.y >= rect.bottom - SCROLLBAR_SIZE;
            if button != PointerButton::Left
                || point.x < code_left
                || over_vertical_scrollbar
                || over_horizontal_scrollbar
            {
                return;
            }

            st_pointer.update(move |app| {
                let cursor = app.workspace.active_buffer().map(|buffer| {
                    cursor_offset_from_point(
                        buffer, rect, code_left, metrics, scroll_x, scroll_y, point.x, point.y,
                    )
                });
                if let Some(cursor) = cursor
                    && let Some(buffer) = app.workspace.active_buffer_mut()
                {
                    buffer.set_cursor(cursor);
                }
                reveal_cursor(app, rect);
            });
            cx.stop_propagation();
        });

        let st_wheel = state.clone();
        root = root.on_wheel(move |_cx, delta| {
            let (step_x, step_y) = match delta.unit {
                WheelUnit::Lines => (delta.x * theme::LINE_H * 3.0, delta.y * theme::LINE_H * 3.0),
                WheelUnit::Pixels => (delta.x, delta.y),
            };
            st_wheel.update(move |app| {
                let (x, y) = app.workspace.active_scroll();
                app.workspace.set_active_scroll(
                    (x - step_x).clamp(0.0, metrics.max_x),
                    (y - step_y).clamp(0.0, metrics.max_y),
                );
            });
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

        let code_viewport = UiRect::new(code_left, code_top, rect.right, rect.bottom);
        let layout_right = code_left + metrics.content_w;
        let mut code = group(UiRect::new(
            code_left,
            code_top,
            layout_right,
            code_top + metrics.content_h,
        ));

        for line_index in visible_lines.clone() {
            let y = code_top + line_index as f32 * theme::LINE_H;
            let line = lines[line_index];
            let layout = layout_line(line, code_left, y, layout_right);
            let mut x = code_left;
            let mut char_offset = 0;
            for (span, color) in syntax::highlight_line(line, meta.lang) {
                char_offset += span.chars().count();
                let next_x = caret_x(layout.as_ref(), char_offset)
                    .unwrap_or_else(|| x + span.chars().count() as f32 * theme::CHAR_W);
                let span_rect = UiRect::new(x, y, next_x.max(x) + 4.0, y + theme::LINE_H);
                code = code.child(text(span_rect, span, theme::mono(color, theme::CODE_SIZE)));
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
            editor_focus,
        ));
    }

    let st_input = state.clone();
    root = root.on_input(move |_ctx, input| {
        let input = input.to_string();
        st_input.update(move |app| {
            if let Some(buffer) = app.workspace.active_buffer_mut() {
                buffer.insert(&input);
            }
            reveal_cursor(app, rect);
        });
    });

    let st_key = state.clone();
    root = root.on_key_down(move |ctx, event| {
        if is_save_shortcut(event) {
            save_active_document(&st_key);
            ctx.prevent_default();
            ctx.stop_propagation();
        } else {
            handle_key(&st_key, event, rect);
        }
    });

    let st_focus = state.clone();
    root = root.on_focus(move |_ctx| st_focus.update(|app| app.focused = true));

    let st_blur = state.clone();
    root = root.on_blur(move |_ctx| st_blur.update(|app| app.focused = false));

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
    buffer.text().split('\n').collect()
}

fn scroll_metrics(rect: UiRect, code_left: f32, lines: &[&str]) -> ScrollMetrics {
    let viewport_w = (rect.right - code_left).max(1.0);
    let viewport_h = rect.height().max(1.0);
    let longest_columns = lines
        .iter()
        .map(|line| visual_columns(line))
        .max()
        .unwrap_or(0);
    let content_w = (longest_columns as f32 * theme::CHAR_W + TRAILING_CODE_SPACE).max(viewport_w);
    let content_h = (lines.len() as f32 * theme::LINE_H).max(theme::LINE_H);
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
    line.chars().fold(0, |column, ch| match ch {
        '\t' => column + (4 - column % 4),
        _ if ch.is_ascii() => column + 1,
        _ => column + 2,
    })
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

fn layout_line(line: &str, left: f32, top: f32, right: f32) -> Option<TextLayout> {
    let bounds = UiRect::new(left, top, right.max(left + 1.0), top + theme::LINE_H);
    let mut request = TextLayoutRequest::single_line(line, bounds, theme::CODE_SIZE, 400);
    request.font_families = theme::MONO_FAMILIES;
    text::layout(&request)
}

fn caret_x(layout: Option<&TextLayout>, char_index: usize) -> Option<f32> {
    layout?.caret_rect(char_index).map(|rect| rect.left)
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
    .map(|layout| layout.hit_test(content_x, content_y).index)
    .unwrap_or_else(|| {
        (((content_x - code_left) / theme::CHAR_W).max(0.0).round() as usize)
            .min(line_text.chars().count())
    });
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
    (((point_y - viewport_top + scroll_y) / theme::LINE_H).floor().max(0.0) as usize)
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

fn handle_key(state: &State<AppState>, event: &KeyboardEvent, rect: UiRect) {
    if event.state != KeyState::Down {
        return;
    }
    let command = match &event.key {
        LogicalKey::Named(NamedKey::Backspace) => EditCommand::Backspace,
        LogicalKey::Named(NamedKey::Delete) => EditCommand::Delete,
        LogicalKey::Named(NamedKey::Enter) => EditCommand::Enter,
        LogicalKey::Named(NamedKey::Tab) => EditCommand::Tab,
        LogicalKey::Named(NamedKey::ArrowLeft) => EditCommand::Left,
        LogicalKey::Named(NamedKey::ArrowRight) => EditCommand::Right,
        LogicalKey::Named(NamedKey::ArrowUp) => EditCommand::Up,
        LogicalKey::Named(NamedKey::ArrowDown) => EditCommand::Down,
        LogicalKey::Named(NamedKey::Home) => EditCommand::Home,
        LogicalKey::Named(NamedKey::End) => EditCommand::End,
        _ => return,
    };

    state.update(move |app| {
        if let Some(buffer) = app.workspace.active_buffer_mut() {
            match command {
                EditCommand::Backspace => buffer.backspace(),
                EditCommand::Delete => buffer.delete_forward(),
                EditCommand::Enter => buffer.insert("\n"),
                EditCommand::Tab => buffer.insert("  "),
                EditCommand::Left => buffer.move_left(),
                EditCommand::Right => buffer.move_right(),
                EditCommand::Up => buffer.move_up(),
                EditCommand::Down => buffer.move_down(),
                EditCommand::Home => buffer.move_home(),
                EditCommand::End => buffer.move_end(),
            }
        }
        reveal_cursor(app, rect);
    });
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
