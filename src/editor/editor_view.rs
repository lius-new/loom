//! The central code viewport: gutter, syntax-highlighted source and cursor.
//!
//! This is the only component that mutates the buffer — through `State<AppState>`
//! updates — so editing logic stays co-located and the rest of the app only
//! reads buffers.

use lgui::core::{EventPolicy, KeyState, KeyboardEvent, LogicalKey, NamedKey, UiElement, UiId};
use lgui::prelude::{Element, State, UiRect, VisualStyle, group, panel, text};
use lgui::text::{self, TextLayout, TextLayoutRequest};

use crate::editor::gutter;
use crate::editor::syntax;
use crate::state::AppState;
use crate::theme;

/// Render the active file's editor surface into `rect`.
pub fn render(rect: UiRect, state: State<AppState>, editor_id: UiId) -> Element {
    // Snapshot the state for this frame (State<T>.get() clones).
    let s = state.get();

    // ---- Root interactive surface -------------------------------------
    let mut root = Element::new(move |cx| {
        UiElement::panel(editor_id, rect, VisualStyle::filled(theme::BG)).children(cx.children)
    });
    root = root.event_policy(EventPolicy::INTERACTIVE);

    if let Some(id) = s.workspace.active() {
        let m = s
            .workspace
            .meta(id)
            .expect("active document metadata exists");
        let buf = s.workspace.active_buffer().expect("active buffer exists");
        let (cline, ccol) = buf.line_col();
        let n_lines = buf.line_count();

        let code_top = rect.top;
        let code_left = rect.left + theme::GUTTER_W + theme::CODE_PAD;

        // ---- Gutter ----------------------------------------------------
        let gutter_rect = UiRect::new(
            rect.left,
            code_top,
            rect.left + theme::GUTTER_W,
            rect.bottom,
        );
        root = root.child(gutter::render(gutter_rect, n_lines, None));

        // ---- Active-line highlight ------------------------------------
        let hl = UiRect::new(
            rect.left + theme::GUTTER_W,
            code_top + cline as f32 * theme::LINE_H,
            rect.right,
            code_top + (cline as f32 + 1.0) * theme::LINE_H,
        );
        root = root.child(panel(hl, VisualStyle::filled(theme::ACTIVE_LINE)));

        // ---- Source lines (syntax highlighted) ------------------------
        let mut lines = group(UiRect::new(code_left, code_top, rect.right, rect.bottom));
        for i in 0..n_lines {
            let y = code_top + i as f32 * theme::LINE_H;
            let line = buf.line(i);
            let layout = layout_line(line, code_left, y, rect.right);
            let mut x = code_left;
            let mut char_offset = 0;
            for (span, color) in syntax::highlight_line(line, m.lang) {
                char_offset += span.chars().count();
                let next_x = caret_x(layout.as_ref(), char_offset)
                    .unwrap_or_else(|| x + span.chars().count() as f32 * theme::CHAR_W);
                let r = UiRect::new(x, y, next_x.max(x) + 4.0, y + theme::LINE_H);
                lines = lines.child(text(r, span, theme::mono(color, theme::CODE_SIZE)));
                x = next_x;
            }
        }

        root = root.child(lines);

        // ---- Cursor ----------------------------------------------------
        let cy = code_top + cline as f32 * theme::LINE_H;
        let cursor_layout = layout_line(buf.line(cline), code_left, cy, rect.right);
        let cx = caret_x(cursor_layout.as_ref(), ccol)
            .unwrap_or(code_left + ccol as f32 * theme::CHAR_W);
        let cursor_rect = UiRect::new(cx, cy + 3.0, cx + 2.0, cy + theme::LINE_H - 3.0);
        if s.focused {
            root = root.child(panel(cursor_rect, VisualStyle::filled(theme::ACCENT)));
        } else {
            root = root.child(panel(
                cursor_rect,
                VisualStyle::default().stroked(theme::hairline(theme::ACCENT)),
            ));
        }
    } else {
        // Empty state: no file is open.
        let cy = rect.top + (rect.bottom - rect.top) / 2.0;
        root = root.child(text(
            UiRect::new(rect.left + 16.0, cy - 10.0, rect.right - 16.0, cy + 10.0),
            "No file open — pick one from the file tree",
            theme::sans(theme::ZINC_500, theme::SMALL),
        ));
    }

    // ---- Editing handlers ---------------------------------------------
    let st = state.clone();
    root = root.on_input(move |_ctx, s_text| {
        let t = s_text.to_string();
        st.update(move |app| {
            if let Some(buf) = app.workspace.active_buffer_mut() {
                buf.insert(&t);
            }
        });
    });

    let st = state.clone();
    root = root.on_key_down(move |_ctx, ev| handle_key(&st, ev));

    let st = state.clone();
    root = root.on_focus(move |_ctx| st.update(|app| app.focused = true));

    let st = state.clone();
    root = root.on_blur(move |_ctx| st.update(|app| app.focused = false));

    root
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

fn handle_key(state: &State<AppState>, ev: &KeyboardEvent) {
    if ev.state != KeyState::Down {
        return;
    }
    match &ev.key {
        LogicalKey::Named(NamedKey::Backspace) => state.update(|app| {
            if let Some(buf) = app.workspace.active_buffer_mut() {
                buf.backspace()
            }
        }),
        LogicalKey::Named(NamedKey::Delete) => state.update(|app| {
            if let Some(buf) = app.workspace.active_buffer_mut() {
                buf.delete_forward()
            }
        }),
        LogicalKey::Named(NamedKey::Enter) => state.update(|app| {
            if let Some(buf) = app.workspace.active_buffer_mut() {
                buf.insert("\n")
            }
        }),
        LogicalKey::Named(NamedKey::Tab) => state.update(|app| {
            if let Some(buf) = app.workspace.active_buffer_mut() {
                buf.insert("  ")
            }
        }),
        LogicalKey::Named(NamedKey::ArrowLeft) => state.update(|app| {
            if let Some(buf) = app.workspace.active_buffer_mut() {
                buf.move_left()
            }
        }),
        LogicalKey::Named(NamedKey::ArrowRight) => state.update(|app| {
            if let Some(buf) = app.workspace.active_buffer_mut() {
                buf.move_right()
            }
        }),
        LogicalKey::Named(NamedKey::ArrowUp) => state.update(|app| {
            if let Some(buf) = app.workspace.active_buffer_mut() {
                buf.move_up()
            }
        }),
        LogicalKey::Named(NamedKey::ArrowDown) => state.update(|app| {
            if let Some(buf) = app.workspace.active_buffer_mut() {
                buf.move_down()
            }
        }),
        LogicalKey::Named(NamedKey::Home) => state.update(|app| {
            if let Some(buf) = app.workspace.active_buffer_mut() {
                buf.move_home()
            }
        }),
        LogicalKey::Named(NamedKey::End) => state.update(|app| {
            if let Some(buf) = app.workspace.active_buffer_mut() {
                buf.move_end()
            }
        }),
        _ => {}
    }
}
