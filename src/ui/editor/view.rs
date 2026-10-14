//! The central code viewport: gutter, syntax-highlighted source, cursor and
//! editor-specific overlay scrollbars.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use lgui::core::{
    CursorIcon, EventPolicy, KeyState, PointerButton, SemanticRole, Semantics, UiElement,
    UiEventKind, UiEventPayload, UiFocusHandle, UiId, WheelUnit, clip,
};
use lgui::prelude::{Element, UiRect, VisualStyle, group, panel, text};
use lgui::text::{self, TextLayout, TextLayoutRequest};

use super::gutter;
use super::syntax;
use crate::domain::identity::PaneId;
use crate::domain::line_change::LineChange;
use crate::ui::editor_binding::{Binding, Event, PeekPlacement, ViewState};
use crate::ui::fonts;
use crate::ui::theme;
use loom_editor_core::text::TextBuffer;
use loom_editor_core::vim::CursorShape;

pub(crate) use crate::ui::components::scrollbar::SIZE as SCROLLBAR_SIZE;
use crate::ui::components::scrollbar::{
    INSET as SCROLLBAR_INSET, MIN_THUMB_LENGTH, thumb as scrollbar_thumb, track as scrollbar_track,
};
const TRAILING_CODE_SPACE: f32 = 32.0;
/// The shortest change mark on the scrollbar, so one line stays visible.
const MIN_MARK_LENGTH: f32 = 3.0;

pub(crate) use crate::ui::editor_binding::Metrics as ScrollMetrics;

pub(crate) use crate::ui::editor_binding::OverviewMark;

/// The overview ruler marks of a file's Git `changes`, keyed by 1-based
/// line; adjacent lines with the same change form one mark.
pub(crate) fn change_marks(
    changes: &BTreeMap<usize, LineChange>,
    line_count: usize,
) -> Vec<OverviewMark> {
    let mut marks: Vec<(usize, usize, LineChange)> = Vec::new();
    for (&line, &change) in changes {
        let line = line.clamp(1, line_count.max(1));
        match marks.last_mut() {
            Some((_, end, last)) if *last == change && *end + 1 >= line => *end = line,
            _ => marks.push((line, line, change)),
        }
    }
    marks
        .into_iter()
        .map(|(first, last, change)| OverviewMark {
            first: first - 1,
            lines: last + 1 - first,
            change,
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ThumbGeometry {
    pub(crate) start: f32,
    pub(crate) length: f32,
}

/// Render the editor of `pane`'s active file into `rect`.
pub(crate) fn render(
    pane: PaneId,
    rect: UiRect,
    state: Binding,
    editor_id: UiId,
    editor_focus: UiFocusHandle,
) -> Element {
    let s = state.get();
    let focused_pane = s.focused_pane;

    let ime_rect = ime_cursor_rect(&s, rect);
    // Vim's Normal and Visual modes take keys as commands, so the input
    // method stays off until a mode accepts text.
    let role = if !s.accepts_text {
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

    if let Some(document) = &s.document {
        let id = document.id;
        let meta = &document.meta;
        let buffer = document.editor();
        let line_count = buffer.line_count();
        let (cursor_line, cursor_col) = buffer.line_col();
        let code_top = rect.top;
        let code_left = rect.left + fonts::editor().gutter_w() + theme::CODE_PAD;
        let metrics = scroll_metrics(rect, code_left, &buffer);
        let (stored_x, stored_y) = document.scroll;
        let scroll_x = stored_x.clamp(0.0, metrics.max_x);
        let scroll_y = stored_y.clamp(0.0, metrics.max_y);
        let visible_lines = visible_line_range(scroll_y, rect.height(), line_count);
        let decorations = s
            .line_changes
            .range(visible_lines.start + 1..visible_lines.end + 1)
            .map(|(line, change)| {
                let decoration = match change {
                    LineChange::Added => gutter::GutterDecoration::GitAdded,
                    LineChange::Modified => gutter::GutterDecoration::GitModified,
                    LineChange::Deleted => gutter::GutterDecoration::GitDeleted,
                };
                (*line, vec![decoration])
            })
            .collect::<BTreeMap<_, _>>();
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
            st_pointer.send(Event::PointerDown {
                document: id,
                point: (point.x, point.y),
                button,
                code_left,
                metrics,
            });
            cx.stop_propagation();
        });

        let st_drag = state.clone();
        root = root.on_pointer_drag(move |cx, pointer| {
            st_drag.send(Event::PointerDrag((pointer.point.x, pointer.point.y)));
            cx.stop_propagation();
        });
        let st_up = state.clone();
        root = root.on_pointer_up(move |_, _| st_up.send(Event::PointerUp));

        let st_wheel = state.clone();
        root = root.on_wheel(move |cx, delta| {
            let (step_x, step_y) = match delta.unit {
                WheelUnit::Lines => (
                    delta.x * fonts::editor().line_h * 3.0,
                    delta.y * fonts::editor().line_h * 3.0,
                ),
                WheelUnit::Pixels => (delta.x, delta.y),
            };
            st_wheel.send(Event::Wheel {
                x: step_x,
                y: step_y,
                metrics,
            });
            cx.stop_propagation();
        });

        // Default cursors inherit from ancestors in lgui. Keep the text cursor
        // on the content subtree so sibling scrollbars retain the arrow cursor.
        // Content runs under the overlay scrollbars, whose tracks are transparent.
        let mut content = clip(rect, 0.0, 0.0)
            .key(format!("editor-content-{}", pane.get()))
            .cursor(CursorIcon::Text);
        if visible_lines.contains(&cursor_line) {
            let highlight_y = code_top + cursor_line as f32 * fonts::editor().line_h;
            let highlight = panel(
                UiRect::new(
                    rect.left + fonts::editor().gutter_w(),
                    highlight_y,
                    rect.right,
                    highlight_y + fonts::editor().line_h,
                ),
                VisualStyle::filled(theme::c().active_line),
            );
            content = content.child(clip(rect, 0.0, -scroll_y).child(highlight));
        }

        let gutter_viewport = UiRect::new(
            rect.left,
            code_top,
            rect.left + fonts::editor().gutter_w(),
            rect.bottom,
        );
        let gutter_content = UiRect::new(
            gutter_viewport.left,
            code_top,
            gutter_viewport.right,
            code_top + metrics.content_h,
        );
        let gutter_hover = s
            .editor_gutter_hover
            .filter(|(hovered_pane, _)| *hovered_pane == pane)
            .map(|(_, line)| line);
        content = content.child(clip(gutter_viewport, 0.0, -scroll_y).child(gutter::render(
            gutter_content,
            visible_lines.clone(),
            &decorations,
            gutter_hover,
        )));
        // The hovered change bar opens its peek, like a link.
        if let Some(run) = gutter::hovered_run(&decorations, gutter_hover)
            && let Some(line) = gutter_hover
            && s.has_repository
        {
            let top = (code_top + (*run.start() - 1) as f32 * fonts::editor().line_h - scroll_y)
                .max(gutter_viewport.top);
            let bottom = (code_top + *run.end() as f32 * fonts::editor().line_h - scroll_y)
                .min(gutter_viewport.bottom);
            let st_peek = state.clone();
            content = content.child(
                panel(
                    UiRect::new(gutter_viewport.left, top, gutter_viewport.right, bottom),
                    VisualStyle::default(),
                )
                .key(format!("gutter-change-{}", pane.get()))
                .event_policy(EventPolicy::INTERACTIVE)
                .cursor(CursorIcon::Pointer)
                .semantics(Semantics::new(SemanticRole::Button).name("Show change"))
                .on_pointer_down(move |cx, _pointer| {
                    cx.stop_propagation();
                    st_peek.send(Event::Peek { line });
                }),
            );
        }
        let st_gutter_hover = state.clone();
        root = root.on_event_capture(UiEventKind::PointerMove, move |_cx, payload| {
            if let UiEventPayload::PointerMove { pointer } = payload {
                let point = pointer.point;
                let hovered = gutter_viewport.contains(point).then(|| {
                    let line =
                        ((point.y - code_top + scroll_y) / fonts::editor().line_h).floor() as usize;
                    (pane, line + 1)
                });
                let hovered = hovered.filter(|(_, line)| *line <= line_count + 1);
                st_gutter_hover.send(Event::GutterHover(hovered));
            }
        });

        let code_viewport = UiRect::new(code_left, code_top, rect.right, rect.bottom);
        let layout_right = code_left + metrics.content_w;
        let mut code = group(UiRect::new(
            code_left,
            code_top,
            layout_right,
            code_top + metrics.content_h,
        ));

        // Vim draws its own visual selection; other views show the core one.
        let selections = &s.selections;
        let large_file = buffer.len() > 4 * 1024 * 1024 || buffer.max_line_display_width() > 4096;
        let matches = &s.search_matches;
        for line_index in visible_lines.clone() {
            let y = code_top + line_index as f32 * fonts::editor().line_h;
            let window = line_window(&buffer, line_index, scroll_x, metrics.viewport_w);
            let line = &buffer.text()[window.clone()];
            let initial_column = buffer.display_column(window.start);
            let line_left = code_left + initial_column as f32 * fonts::editor().char_w;
            let layout = layout_line_at_column(line, line_left, y, layout_right, initial_column);
            let (start, end) = (window.start, window.end);
            let next = buffer.line_range_with_break(line_index).end;
            // The part of `range` on this line, as a highlight rectangle.
            let line_rect = |range: &Range<usize>| {
                if range.start >= next || range.end <= start || range.start > end {
                    return None;
                }
                let from = range.start.max(start).min(end);
                let to = range.end.min(end).max(start);
                let left = caret_x(layout.as_ref(), buffer.text()[start..from].chars().count())
                    .unwrap_or(line_left);
                let mut right = caret_x(layout.as_ref(), buffer.text()[start..to].chars().count())
                    .unwrap_or(left);
                if range.end > end {
                    right += fonts::editor().char_w;
                }
                Some(UiRect::new(
                    left,
                    y,
                    right.max(left + 2.0),
                    y + fonts::editor().line_h,
                ))
            };
            for found in matches {
                if let Some(r) = line_rect(found) {
                    code = code.child(panel(r, VisualStyle::filled(theme::c().warning).alpha(45)));
                }
            }
            for selection in selections {
                if let Some(r) = line_rect(selection) {
                    code = code.child(panel(r, VisualStyle::filled(theme::c().accent).alpha(55)));
                }
            }
            let mut x = line_left;
            let mut char_offset = 0;
            let mut display_column = initial_column;
            let spans = if large_file {
                vec![(line.to_owned(), theme::c().text)]
            } else {
                syntax::highlight_line(line, meta.lang)
            };
            for (span, color) in spans {
                char_offset += span.chars().count();
                let next_x = caret_x(layout.as_ref(), char_offset)
                    .unwrap_or_else(|| x + span.chars().count() as f32 * fonts::editor().char_w);
                let span_rect = UiRect::new(x, y, next_x.max(x) + 4.0, y + fonts::editor().line_h);
                let (display, _, column) = expand_tabs(&span, display_column);
                display_column = column;
                code = code.child(text(span_rect, display, fonts::editor().style(color)));
                x = next_x;
            }
        }

        // Only the editor that owns keyboard focus shows a caret.
        let editor_focused = s.focused && focused_pane;
        if editor_focused && visible_lines.contains(&cursor_line) {
            let cursor_y = code_top + cursor_line as f32 * fonts::editor().line_h;
            let (cursor_x, cursor_layout) = document_caret(
                &buffer,
                cursor_line,
                cursor_col,
                code_left,
                cursor_y,
                layout_right,
            );
            let shape = s.cursor_shape;
            let cell = [cursor_x, cursor_y];
            let (cursor_rect, cursor_style) = if shape == CursorShape::Bar {
                (
                    super::caret::bar(cell),
                    VisualStyle::filled(theme::c().accent),
                )
            } else {
                // Block cursors cover the character under the cursor.
                let chars = buffer
                    .grapheme_at(buffer.cursor())
                    .filter(|grapheme| !grapheme.starts_with(['\r', '\n']))
                    .map_or(0, |grapheme| grapheme.chars().count());
                let right = (chars > 0)
                    .then(|| {
                        if buffer.line(cursor_line).len() > 4096 {
                            let offset = buffer.next_grapheme(buffer.cursor());
                            Some(
                                code_left
                                    + buffer.display_column(offset) as f32 * fonts::editor().char_w,
                            )
                        } else {
                            caret_x(cursor_layout.as_ref(), cursor_col + chars)
                        }
                    })
                    .flatten()
                    .unwrap_or(cursor_x + fonts::editor().char_w)
                    .max(cursor_x + 2.0);
                let top = match shape {
                    CursorShape::Underline => cursor_y + fonts::editor().line_h - 3.0,
                    CursorShape::HalfBlock => cursor_y + fonts::editor().line_h / 2.0,
                    _ => cursor_y + 1.0,
                };
                (
                    UiRect::new(
                        cursor_x,
                        top,
                        right,
                        cursor_y + fonts::editor().line_h - 1.0,
                    ),
                    VisualStyle::filled(theme::c().accent).alpha(140),
                )
            };
            // Every shape shares one caret element so mode switches keep the motion.
            // Selections (core or Vim visual) are painted immediately, so the caret
            // must not lag behind their edge.
            code = code.child(super::caret::render(
                cursor_rect,
                cell,
                cursor_style,
                super::caret::CaretContext {
                    pane,
                    document: id,
                    viewport: rect,
                    scroll: (scroll_x, scroll_y),
                },
                s.smooth_caret
                    && !(s.dragging && s.caret_dragging)
                    && selections.is_empty()
                    && s.preedit_for(pane, id).is_none(),
            ));
            if let Some(session) = s.preedit_for(pane, id) {
                let width = layout_line(&session.text, 0.0, 0.0, metrics.content_w)
                    .map_or(80.0, |l| l.width)
                    .max(2.0);
                let r = UiRect::new(
                    cursor_x,
                    cursor_y,
                    cursor_x + width + 4.0,
                    cursor_y + fonts::editor().line_h,
                );
                code = code.child(
                    panel(r, VisualStyle::filled(theme::c().surface))
                        .child(text(
                            r,
                            session.text.clone(),
                            fonts::editor().style(theme::c().text_bright),
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
        if s.peek_visible {
            if let Some(overlay) = state.overlay(PeekPlacement {
                rect: UiRect::new(rect.left, code_top, rect.right, rect.bottom),
                code_left,
                scroll_y,
                metrics,
            }) {
                root = root.child(overlay);
            }
        }

        let emphasized = s.editor_hovered == Some(pane)
            || (focused_pane
                && (s.editor_vertical_scrollbar_dragging
                    || s.editor_horizontal_scrollbar_dragging));
        if !s.overview_marks.is_empty() {
            root = root.child(overview_ruler(
                rect,
                metrics,
                has_horizontal,
                fonts::editor().line_h,
                s.overview_marks
                    .clone()
                    .into_iter()
                    .map(|mark| (mark.first, mark.lines, mark.change)),
            ));
        }
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
        if has_vertical && has_horizontal {
            root = root.child(scrollbar_corner(pane, rect));
        }
    }

    let st_input = state.clone();
    root = root.on_input(move |ctx, input| {
        let input = input.to_string();
        st_input.send(Event::Text(input));
        ctx.stop_propagation();
    });

    let st_ime = state.clone();
    root = root.on_composition_update(move |_, value, cursor| {
        st_ime.send(Event::Composition {
            text: value.to_owned(),
            cursor,
        })
    });
    let st_ime_end = state.clone();
    root = root.on_composition_end(move |_| st_ime_end.send(Event::CompositionEnd));

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

    root.on_focus({
        let state = state.clone();
        move |_| state.send(Event::Focus)
    })
    .on_blur(move |_| state.send(Event::Blur))
}

/// Git change marks along the editor's right edge, independent of the
/// scrollbar, which draws over them. Lines map to the ruler as they do to
/// the scroll range, so a file that fits shows each mark beside its line.
pub(crate) fn overview_ruler(
    rect: UiRect,
    metrics: ScrollMetrics,
    has_horizontal: bool,
    row_height: f32,
    marks: impl IntoIterator<Item = (usize, usize, LineChange)>,
) -> Element {
    let corner = if has_horizontal { SCROLLBAR_SIZE } else { 0.0 };
    let lane = UiRect::new(
        rect.right - SCROLLBAR_SIZE + 1.0,
        rect.top,
        rect.right - SCROLLBAR_SIZE + 5.0,
        rect.bottom - corner,
    );
    let scale = lane.height() / metrics.content_h.max(metrics.viewport_h);
    let mut ruler = group(lane);
    let mut last: Option<(f32, f32, LineChange)> = None;
    let flush = |ruler: Element, (top, bottom, change): (f32, f32, LineChange)| {
        let color = match change {
            LineChange::Added => theme::c().diff.add_mark,
            LineChange::Modified => theme::c().diff.mod_mark,
            LineChange::Deleted => theme::c().diff.del_mark,
        };
        ruler.child(panel(
            UiRect::new(lane.left, top, lane.right, bottom),
            VisualStyle::filled(color),
        ))
    };
    for (first, lines, change) in marks {
        let top = (lane.top + first as f32 * row_height * scale).min(lane.bottom - MIN_MARK_LENGTH);
        let bottom =
            (lane.top + (first + lines) as f32 * row_height * scale).max(top + MIN_MARK_LENGTH);
        // Marks closer than a pixel draw as one, so huge files stay cheap.
        match &mut last {
            Some((_, end, previous_change)) if *previous_change == change && top <= *end + 1.0 => {
                *end = end.max(bottom)
            }
            _ => {
                if let Some(previous) = last.replace((top, bottom, change)) {
                    ruler = flush(ruler, previous);
                }
            }
        }
    }
    if let Some(previous) = last {
        ruler = flush(ruler, previous);
    }
    ruler
}

pub(crate) fn vertical_scrollbar(
    pane: PaneId,
    rect: UiRect,
    metrics: ScrollMetrics,
    scroll: f32,
    has_horizontal: bool,
    emphasized: bool,
    state: Binding,
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
            st_track_down.send(Event::ScrollbarDown {
                vertical: true,
                offset,
                position: Some(next),
            });
        })
        .on_pointer_drag(move |_cx, pointer| {
            st_track_move.send(Event::ScrollbarDrag {
                vertical: true,
                pointer: pointer.point.y,
                start: track.top,
                travel,
                max: metrics.max_y,
            });
        })
        .on_pointer_up(move |_cx, _pointer| {
            st_track_up.send(Event::ScrollbarUp { vertical: true });
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
            st_thumb_down.send(Event::ScrollbarDown {
                vertical: true,
                offset: pointer.point.y - geometry.start,
                position: None,
            });
        })
        .on_pointer_drag(move |_cx, pointer| {
            st_thumb_move.send(Event::ScrollbarDrag {
                vertical: true,
                pointer: pointer.point.y,
                start: track.top,
                travel,
                max: metrics.max_y,
            });
        })
        .on_pointer_up(move |_cx, _pointer| {
            st_thumb_up.send(Event::ScrollbarUp { vertical: true });
        });

    group(track)
        .key(format!("editor-vertical-scrollbar-{}", pane.get()))
        .child(track_hit)
        .child(thumb)
}

/// The square where both scrollbars meet. It keeps the arrow cursor over the
/// content that runs beneath it and swallows clicks there.
pub(crate) fn scrollbar_corner(pane: PaneId, rect: UiRect) -> Element {
    scrollbar_track(UiRect::new(
        rect.right - SCROLLBAR_SIZE,
        rect.bottom - SCROLLBAR_SIZE,
        rect.right,
        rect.bottom,
    ))
    .key(format!("editor-scrollbar-corner-{}", pane.get()))
    .event_policy(EventPolicy::INTERACTIVE)
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
    state: Binding,
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
            st_track_down.send(Event::ScrollbarDown {
                vertical: false,
                offset,
                position: Some(next),
            });
        })
        .on_pointer_drag(move |_cx, pointer| {
            st_track_move.send(Event::ScrollbarDrag {
                vertical: false,
                pointer: pointer.point.x,
                start: track.left,
                travel,
                max: metrics.max_x,
            });
        })
        .on_pointer_up(move |_cx, _pointer| {
            st_track_up.send(Event::ScrollbarUp { vertical: false });
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
            st_thumb_down.send(Event::ScrollbarDown {
                vertical: false,
                offset: pointer.point.x - geometry.start,
                position: None,
            });
        })
        .on_pointer_drag(move |_cx, pointer| {
            st_thumb_move.send(Event::ScrollbarDrag {
                vertical: false,
                pointer: pointer.point.x,
                start: track.left,
                travel,
                max: metrics.max_x,
            });
        })
        .on_pointer_up(move |_cx, _pointer| {
            st_thumb_up.send(Event::ScrollbarUp { vertical: false });
        });

    group(track)
        .key(format!("editor-horizontal-scrollbar-{}", pane.get()))
        .child(track_hit)
        .child(thumb)
}

/// Keep long-line shaping bounded horizontally as well as vertically.
pub(crate) fn line_window(
    buffer: &TextBuffer,
    line: usize,
    scroll_x: f32,
    width: f32,
) -> Range<usize> {
    if buffer.line(line).len() <= 4096 {
        return buffer.line_range(line);
    }
    let char_w = fonts::editor().char_w.max(1.0);
    let left = (scroll_x / char_w).floor() as usize;
    let right = ((scroll_x + width) / char_w).ceil() as usize;
    buffer
        .display_column_range(line, left.saturating_sub(32), right.saturating_add(32))
        .unwrap_or_else(|| {
            let end = buffer.line_end(line);
            end..end
        })
}

pub(crate) fn document_caret(
    buffer: &TextBuffer,
    line: usize,
    column: usize,
    left: f32,
    top: f32,
    right: f32,
) -> (f32, Option<LineLayout>) {
    if buffer.line(line).len() > 4096 {
        let offset = buffer.offset_at_char_column(line, column);
        let visual = buffer.display_column(offset);
        let range = buffer
            .display_column_range(line, visual.saturating_sub(32), visual.saturating_add(32))
            .unwrap_or(offset..offset);
        let initial = buffer.display_column(range.start);
        let origin = left + initial as f32 * fonts::editor().char_w;
        let layout =
            layout_line_at_column(&buffer.text()[range.clone()], origin, top, right, initial);
        let chars = buffer.text()[range.start..offset].chars().count();
        return (
            caret_x(layout.as_ref(), chars)
                .unwrap_or(left + visual as f32 * fonts::editor().char_w),
            None,
        );
    }
    let layout = layout_line(buffer.line(line), left, top, right);
    let x =
        caret_x(layout.as_ref(), column).unwrap_or(left + column as f32 * fonts::editor().char_w);
    (x, layout)
}

pub(crate) fn scroll_metrics(rect: UiRect, code_left: f32, buffer: &TextBuffer) -> ScrollMetrics {
    let mut viewport_w = (rect.right - code_left).max(1.0);
    let mut viewport_h = rect.height().max(1.0);
    let natural_w =
        buffer.max_line_display_width() as f32 * fonts::editor().char_w + TRAILING_CODE_SPACE;
    let content_h =
        (buffer.line_count() as f32 * fonts::editor().line_h).max(fonts::editor().line_h);
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

pub(crate) fn visual_columns(line: &str) -> usize {
    loom_editor_core::text::display_width(line)
}

pub(crate) fn visible_line_range(
    scroll_y: f32,
    viewport_h: f32,
    line_count: usize,
) -> Range<usize> {
    let first = ((scroll_y / fonts::editor().line_h).floor() as usize).saturating_sub(1);
    let last =
        (((scroll_y + viewport_h) / fonts::editor().line_h).ceil() as usize + 1).min(line_count);
    first.min(last)..last
}

pub(crate) fn thumb_geometry(
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

pub(crate) fn scroll_from_thumb(thumb_start: f32, travel: f32, max_scroll: f32) -> f32 {
    if travel <= 0.0 {
        0.0
    } else {
        (thumb_start / travel * max_scroll).clamp(0.0, max_scroll)
    }
}

/// Layout uses expanded tabs, while all buffer positions remain source character indices.
#[derive(Clone)]
pub(crate) struct LineLayout {
    pub(crate) layout: TextLayout,
    pub(crate) source_to_display: Vec<usize>,
    pub(crate) width: f32,
}

type LayoutKey = (u32, usize, u32, u32, u32, usize);
#[derive(Default)]
struct LayoutCache {
    entries: HashMap<LayoutKey, HashMap<String, LineLayout>>,
    len: usize,
}
thread_local! {
    static LINE_LAYOUTS: RefCell<LayoutCache> = RefCell::new(LayoutCache::default());
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
pub(crate) fn layout_line(line: &str, left: f32, top: f32, right: f32) -> Option<LineLayout> {
    layout_line_at_column(line, left, top, right, 0)
}

pub(crate) fn layout_line_at_column(
    line: &str,
    left: f32,
    top: f32,
    right: f32,
    column: usize,
) -> Option<LineLayout> {
    let font = fonts::editor();
    let key = (
        font.size.to_bits(),
        font.families.as_ptr() as usize,
        left.to_bits(),
        top.to_bits(),
        right.to_bits(),
        column % 4,
    );
    if let Some(layout) = LINE_LAYOUTS.with(|cache| {
        cache
            .borrow()
            .entries
            .get(&key)
            .and_then(|entries| entries.get(line))
            .cloned()
    }) {
        return Some(layout);
    }
    let (display, source_to_display, _) = expand_tabs(line, column);
    let bounds = UiRect::new(
        left,
        top,
        right.max(left + 1.0),
        top + fonts::editor().line_h,
    );
    let mut request = TextLayoutRequest::single_line(&display, bounds, fonts::editor().size, 400);
    request.font_families = fonts::editor().families;
    let layout = text::layout(&request).map(|layout| LineLayout {
        width: layout.width,
        layout,
        source_to_display,
    });
    if line.len() <= 8192
        && let Some(layout) = &layout
    {
        LINE_LAYOUTS.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.len >= 256 {
                *cache = LayoutCache::default();
            }
            cache
                .entries
                .entry(key)
                .or_default()
                .insert(line.to_owned(), layout.clone());
            cache.len += 1;
        });
    }
    layout
}
pub(crate) fn caret_x(layout: Option<&LineLayout>, char_index: usize) -> Option<f32> {
    let layout = layout?;
    let index = *layout
        .source_to_display
        .get(char_index)
        .or_else(|| layout.source_to_display.last())?;
    layout.layout.caret_rect(index).map(|rect| rect.left)
}
pub(crate) fn hit_column(layout: &LineLayout, x: f32, y: f32) -> usize {
    let index = layout.layout.hit_test(x, y).index;
    layout
        .source_to_display
        .iter()
        .enumerate()
        .min_by_key(|(_, i)| i.abs_diff(index))
        .map_or(0, |(i, _)| i)
}
pub(crate) fn fallback_hit_column(line: &str, x: f32) -> usize {
    let mut column = 0;
    let mut chars = 0;
    for g in line.graphemes(true) {
        let width = if g == "\t" {
            4 - column % 4
        } else {
            UnicodeWidthStr::width(g)
        };
        if x < (column as f32 + width as f32 / 2.0) * fonts::editor().char_w {
            return chars;
        }
        column += width;
        chars += g.chars().count();
    }
    chars
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cursor_offset_from_point(
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
    if line_text.len() > 4096 {
        let column =
            ((content_x - code_left).max(0.0) / fonts::editor().char_w.max(1.0)).round() as usize;
        let approximate = buffer.offset_at_display_column(line, column);
        let range = buffer
            .display_column_range(line, column.saturating_sub(32), column.saturating_add(32))
            .unwrap_or(approximate..approximate);
        let initial = buffer.display_column(range.start);
        let left = code_left + initial as f32 * fonts::editor().char_w;
        let layout = layout_line_at_column(
            &buffer.text()[range.clone()],
            left,
            rect.top,
            code_left + metrics.content_w,
            initial,
        );
        return layout
            .map(|layout| {
                let local = hit_column(&layout, content_x, point_y);
                let offset = buffer.text()[range.clone()]
                    .char_indices()
                    .nth(local)
                    .map_or(range.end, |(byte, _)| range.start + byte);
                buffer.grapheme_floor(offset)
            })
            .unwrap_or(approximate);
    }
    let content_y = point_y + scroll_y;
    let line_top = rect.top + line as f32 * fonts::editor().line_h;
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

pub(crate) fn line_index_from_point(
    point_y: f32,
    viewport_top: f32,
    scroll_y: f32,
    line_count: usize,
) -> usize {
    if line_count == 0 {
        return 0;
    }
    (((point_y - viewport_top + scroll_y) / fonts::editor().line_h)
        .floor()
        .max(0.0) as usize)
        .min(line_count - 1)
}

pub(crate) fn ime_cursor_rect(app: &ViewState, rect: UiRect) -> UiRect {
    let Some(document) = &app.document else {
        return rect;
    };
    let buffer = document.editor();
    let (line, col) = buffer.line_col();
    let (sx, sy) = document.scroll;
    let left = rect.left + fonts::editor().gutter_w() + theme::CODE_PAD;
    let top = rect.top + line as f32 * fonts::editor().line_h - sy;
    let mut x = document_caret(&buffer, line, col, left, top, left + 100000.0).0 - sx;
    let pane = app.pane;
    if let Some(session) = app.preedit_for(pane, document.id) {
        let index = session
            .cursor
            .as_ref()
            .map_or(session.text.len(), |r| r.start)
            .min(session.text.len());
        let prefix = session.text.get(..index).unwrap_or("");
        x += layout_line(prefix, 0.0, 0.0, 100000.0).map_or(0.0, |l| l.width);
    }
    let x = x.clamp(left, rect.right.max(left));
    let top = top.clamp(
        rect.top,
        (rect.bottom - fonts::editor().line_h).max(rect.top),
    );
    UiRect::new(x, top, x + 2.0, top + fonts::editor().line_h)
}

/// Lines in one page of an editor of this size, for page movements.
pub(crate) fn page_lines(rect: UiRect) -> usize {
    (rect.height() / fonts::editor().line_h).floor().max(1.0) as usize
}
