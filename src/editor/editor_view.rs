//! The central code viewport: CodeLens ribbon, gutter, syntax-highlighted
//! source, cursor, ghost completion and the inline AI diff card.
//!
//! This is the only component that mutates the buffer — through `State<AppState>`
//! updates — so editing logic stays co-located and the rest of the app only
//! reads buffers.

use lgui::core::{EventPolicy, KeyState, KeyboardEvent, LogicalKey, NamedKey};
use lgui::prelude::{group, panel, text, Element, State, UiRect, VisualStyle};

use crate::editor::gutter;
use crate::editor::syntax;
use crate::model::document::{meta, FileId};
use crate::state::AppState;
use crate::theme;

/// Render the active file's editor surface into `rect`.
pub fn render(rect: UiRect, state: State<AppState>) -> Element {
    // Snapshot the state for this frame (State<T>.get() clones).
    let s = state.get();
    let id = s.workspace.active();
    let m = meta(id);
    let buf = s.workspace.active_buffer();
    let (cline, ccol) = buf.line_col();
    let n_lines = buf.line_count();

    let code_top = rect.top + theme::LINE_H; // one row reserved for CodeLens
    let code_left = rect.left + theme::GUTTER_W + theme::CODE_PAD;
    let code_width = rect.right - code_left;

    // ---- Root interactive surface -------------------------------------
    let mut root = panel(rect, VisualStyle::filled(theme::BG));
    root = root.event_policy(EventPolicy::INTERACTIVE);

    // ---- CodeLens ribbon ----------------------------------------------
    let tests = if m.has_refs {
        format!("✨ AI Refactor • {} • 2 references", m.tests)
    } else {
        format!("✨ AI Refactor • {}", m.tests)
    };
    let lens_rect = UiRect::new(code_left, rect.top, rect.right - 8.0, rect.top + theme::LINE_H);
    root = root.child(text(lens_rect, tests, theme::mono(theme::ZINC_500, theme::SMALL)));

    // ---- Gutter --------------------------------------------------------
    let gutter_rect = UiRect::new(rect.left, code_top, rect.left + theme::GUTTER_W, rect.bottom);
    root = root.child(gutter::render(
        gutter_rect,
        n_lines,
        if id == FileId::Wallet { Some(5) } else { None },
    ));

    // ---- Active-line highlight ----------------------------------------
    let hl = UiRect::new(
        rect.left + theme::GUTTER_W,
        code_top + cline as f32 * theme::LINE_H,
        rect.right,
        code_top + (cline as f32 + 1.0) * theme::LINE_H,
    );
    root = root.child(panel(hl, VisualStyle::filled(theme::ACTIVE_LINE)));

    // ---- Source lines (syntax highlighted) ----------------------------
    let mut lines = group(UiRect::new(code_left, code_top, rect.right, rect.bottom));
    for i in 0..n_lines {
        let y = code_top + i as f32 * theme::LINE_H;
        let mut x = code_left;
        for (span, color) in syntax::highlight_line(buf.line(i), m.lang) {
            let w = span.chars().count() as f32 * theme::CHAR_W;
            let r = UiRect::new(x, y, x + w + 4.0, y + theme::LINE_H);
            lines = lines.child(text(r, span, theme::mono(color, theme::CODE_SIZE)));
            x += w;
        }
    }

    // Ghost autocomplete (WalletService.cs only)
    if id == FileId::Wallet {
        let gx = code_left + 460.0;
        let gy = code_top + 10.0 * theme::LINE_H;
        lines = lines.child(text(
            UiRect::new(gx, gy, rect.right - 8.0, gy + theme::LINE_H),
            "// [Tab] to insert auto-rollback audit logging",
            theme::mono(theme::GHOST, theme::CODE_SIZE),
        ));
    }

    root = root.child(lines);

    // ---- Cursor --------------------------------------------------------
    let cx = code_left + ccol as f32 * theme::CHAR_W;
    let cy = code_top + cline as f32 * theme::LINE_H;
    let cursor_rect = UiRect::new(cx, cy + 3.0, cx + 2.0, cy + theme::LINE_H - 3.0);
    if s.focused {
        root = root.child(panel(cursor_rect, VisualStyle::filled(theme::ACCENT)));
    } else {
        root = root.child(panel(
            cursor_rect,
            VisualStyle::default().stroked(theme::hairline(theme::ACCENT)),
        ));
    }

    // ---- Inline AI diff card (WalletService.cs only) ------------------
    if id == FileId::Wallet {
        root = root.child(diff_card(UiRect::new(
            code_left + 24.0,
            code_top + 7.0 * theme::LINE_H,
            code_left + 24.0 + 400.0,
            code_top + 14.0 * theme::LINE_H,
        )));
    }

    let _ = code_width;

    // ---- Editing handlers ---------------------------------------------
    let st = state.clone();
    root = root.on_input(move |_ctx, s_text| {
        let t = s_text.to_string();
        st.update(move |app| app.workspace.active_buffer_mut().insert(&t));
    });

    let st = state.clone();
    root = root.on_key_down(move |_ctx, ev| handle_key(&st, ev));

    let st = state.clone();
    root = root.on_focus(move |_ctx| st.update(|app| app.focused = true));

    let st = state.clone();
    root = root.on_blur(move |_ctx| st.update(|app| app.focused = false));

    root
}

fn handle_key(state: &State<AppState>, ev: &KeyboardEvent) {
    if ev.state != KeyState::Down {
        return;
    }
    match &ev.key {
        LogicalKey::Named(NamedKey::Backspace) => {
            state.update(|app| app.workspace.active_buffer_mut().backspace())
        }
        LogicalKey::Named(NamedKey::Delete) => {
            state.update(|app| app.workspace.active_buffer_mut().delete_forward())
        }
        LogicalKey::Named(NamedKey::Enter) => {
            state.update(|app| app.workspace.active_buffer_mut().insert("\n"))
        }
        LogicalKey::Named(NamedKey::Tab) => {
            state.update(|app| app.workspace.active_buffer_mut().insert("  "))
        }
        LogicalKey::Named(NamedKey::ArrowLeft) => {
            state.update(|app| app.workspace.active_buffer_mut().move_left())
        }
        LogicalKey::Named(NamedKey::ArrowRight) => {
            state.update(|app| app.workspace.active_buffer_mut().move_right())
        }
        LogicalKey::Named(NamedKey::ArrowUp) => {
            state.update(|app| app.workspace.active_buffer_mut().move_up())
        }
        LogicalKey::Named(NamedKey::ArrowDown) => {
            state.update(|app| app.workspace.active_buffer_mut().move_down())
        }
        LogicalKey::Named(NamedKey::Home) => {
            state.update(|app| app.workspace.active_buffer_mut().move_home())
        }
        LogicalKey::Named(NamedKey::End) => {
            state.update(|app| app.workspace.active_buffer_mut().move_end())
        }
        _ => {}
    }
}

/// The static inline AI diff widget (title + Accept/Reject + diff rows).
fn diff_card(rect: UiRect) -> Element {
    let mut card = panel(rect, VisualStyle::filled(theme::SURFACE));
    card = card.event_policy(EventPolicy::INTERACTIVE);

    let title = UiRect::new(rect.left + 12.0, rect.top + 8.0, rect.right - 12.0, rect.top + 30.0);
    card = card.child(text(
        title,
        "AI Diff: Distributed Redis Idempotency Lock",
        theme::sans_semibold(theme::ZINC_100, theme::SMALL),
    ));

    let accept = UiRect::new(rect.right - 160.0, rect.top + 8.0, rect.right - 108.0, rect.top + 26.0);
    card = card.child(text(
        accept,
        "Accept (Tab)",
        theme::mono_bold(theme::EMERALD_400, theme::SMALL),
    ));
    let reject = UiRect::new(rect.right - 100.0, rect.top + 8.0, rect.right - 16.0, rect.top + 26.0);
    card = card.child(text(
        reject,
        "Reject (Esc)",
        theme::mono_bold(theme::ZINC_500, theme::SMALL),
    ));

    // Diff rows: [del, add, del, add]
    let rows: [(&str, bool); 4] = [
        ("-   var senderBal = _cache.GetBalanceAsync(sender);", true),
        ("+   var senderBal = await _cache.GetBalanceAsync(sender);", false),
        ("-   await _cache.TransferAsync(sender, receiver, amount);", true),
        ("+   return await _cache.CommitTransferAsync(sender, receiver, amount);", false),
    ];
    for (i, (content, is_del)) in rows.iter().enumerate() {
        let y = rect.top + 32.0 + i as f32 * 20.0;
        let row = UiRect::new(rect.left, y, rect.right, y + 20.0);
        let (bg, fg) = if *is_del {
            (theme::DIFF_DEL_BG, theme::DIFF_DEL_FG)
        } else {
            (theme::DIFF_ADD_BG, theme::DIFF_ADD_FG)
        };
        card = card.child(panel(row, VisualStyle::filled(bg)));
        card = card.child(text(
            UiRect::new(rect.left + 12.0, y, rect.right - 12.0, y + 20.0),
            *content,
            theme::mono(fg, theme::CODE_SIZE),
        ));
    }

    card
}
