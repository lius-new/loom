//! Read-only inline diff editor shown for Source Control change tabs.

use lgui::core::{EventPolicy, WheelUnit, clip};
use lgui::prelude::{Element, State, UiRect, VisualStyle, panel, text};

use crate::model::diff_document::DiffRowKind;
use crate::state::AppState;
use crate::theme;

const ROW_H: f32 = 20.0;
const NUMBER_W: f32 = 44.0;
const MARKER_W: f32 = 20.0;
const SCROLLBAR_W: f32 = 4.0;

pub fn render(rect: UiRect, state: State<AppState>) -> Element {
    let app = state.get();
    let Some(document) = app.workspace.active_diff() else {
        return panel(rect, VisualStyle::filled(theme::BG));
    };

    let rows = &document.rows;
    let binary = document.binary;
    let label = format!("{} — {}", document.path.display(), document.source_label());
    let text_left = rect.left + NUMBER_W * 2.0 + MARKER_W;
    let text_viewport = UiRect::new(text_left, rect.top, rect.right, rect.bottom);
    let content_h = rows.len() as f32 * ROW_H;
    let content_w = rows
        .iter()
        .map(|row| row.text.chars().count())
        .max()
        .unwrap_or_default() as f32
        * theme::CHAR_W
        + theme::CODE_PAD * 2.0;
    let max_y = (content_h - rect.height()).max(0.0);
    let max_x = (content_w - text_viewport.width()).max(0.0);
    let (stored_x, stored_y) = app.workspace.active_scroll();
    let scroll_x = stored_x.clamp(0.0, max_x);
    let scroll_y = stored_y.clamp(0.0, max_y);
    let start = (scroll_y / ROW_H).floor() as usize;
    let end = (((scroll_y + rect.height()) / ROW_H).ceil() as usize + 1).min(rows.len());

    let wheel_state = state.clone();
    let mut root = panel(rect, VisualStyle::filled(theme::BG))
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

    if rows.is_empty() {
        let message = if binary {
            format!("Binary file changed: {label}")
        } else {
            format!("No textual changes: {label}")
        };
        return root.child(text(
            UiRect::new(
                rect.left + 24.0,
                rect.top + 24.0,
                rect.right - 24.0,
                rect.top + 64.0,
            ),
            message,
            theme::mono(theme::ZINC_500, theme::UI_SIZE),
        ));
    }

    let mut body = clip(rect, 0.0, 0.0);
    for (index, row) in rows.iter().enumerate().take(end).skip(start) {
        let top = rect.top + index as f32 * ROW_H - scroll_y;
        let row_rect = UiRect::new(rect.left, top, rect.right, top + ROW_H);
        let (background, foreground, marker) = match row.kind {
            DiffRowKind::Context => (None, theme::ZINC_300, ""),
            DiffRowKind::Addition => (Some(theme::DIFF_ADD_BG), theme::DIFF_ADD_FG, "+"),
            DiffRowKind::Deletion => (Some(theme::DIFF_DEL_BG), theme::DIFF_DEL_FG, "−"),
            DiffRowKind::Hunk => (Some(theme::SURFACE), theme::ZINC_500, ""),
        };
        if let Some(background) = background {
            body = body.child(panel(row_rect, VisualStyle::filled(background)));
        }

        if row.kind == DiffRowKind::Hunk {
            body = body.child(text(
                UiRect::new(rect.left + 10.0, top, rect.right - 10.0, top + ROW_H),
                row.text.clone(),
                theme::mono(theme::ZINC_500, theme::SMALL),
            ));
            continue;
        }

        let old_number = row
            .old_line
            .map(|line| line.to_string())
            .unwrap_or_default();
        let new_number = row
            .new_line
            .map(|line| line.to_string())
            .unwrap_or_default();
        body = body
            .child(text(
                UiRect::new(rect.left, top, rect.left + NUMBER_W - 8.0, top + ROW_H),
                old_number,
                theme::mono_right(theme::ZINC_600, theme::SMALL),
            ))
            .child(text(
                UiRect::new(
                    rect.left + NUMBER_W,
                    top,
                    rect.left + NUMBER_W * 2.0 - 8.0,
                    top + ROW_H,
                ),
                new_number,
                theme::mono_right(theme::ZINC_600, theme::SMALL),
            ))
            .child(text(
                UiRect::new(rect.left + NUMBER_W * 2.0, top, text_left, top + ROW_H),
                marker,
                theme::mono(foreground, theme::CODE_SIZE),
            ));

        let text_width =
            (row.text.chars().count() as f32 * theme::CHAR_W + 16.0).max(text_viewport.width());
        body = body.child(clip(text_viewport, 0.0, 0.0).child(text(
            UiRect::new(
                text_left + theme::CODE_PAD - scroll_x,
                top,
                text_left + theme::CODE_PAD - scroll_x + text_width,
                top + ROW_H,
            ),
            row.text.clone(),
            theme::mono(foreground, theme::CODE_SIZE),
        )));
    }

    body = body
        .child(panel(
            UiRect::new(
                rect.left + NUMBER_W - 1.0,
                rect.top,
                rect.left + NUMBER_W,
                rect.bottom,
            ),
            VisualStyle::filled(theme::BORDER),
        ))
        .child(panel(
            UiRect::new(text_left - 1.0, rect.top, text_left, rect.bottom),
            VisualStyle::filled(theme::BORDER),
        ));
    root = root.child(body);

    if max_y > 0.0 {
        let thumb_h = (rect.height() * rect.height() / content_h).clamp(24.0, rect.height());
        let travel = rect.height() - thumb_h;
        let thumb_top = rect.top + travel * (scroll_y / max_y);
        root = root.child(panel(
            UiRect::new(
                rect.right - SCROLLBAR_W,
                thumb_top,
                rect.right,
                thumb_top + thumb_h,
            ),
            VisualStyle::filled(theme::ZINC_600).radius(2.0),
        ));
    }

    root
}
