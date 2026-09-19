//! Cmd/Ctrl+K floating inline-AI prompt (overlay).

use lgui::core::EventPolicy;
use lgui::prelude::{panel, text, Element, State, UiRect, VisualStyle};

use crate::state::AppState;
use crate::theme;

pub fn render(rect: UiRect, state: State<AppState>) -> Element {
    // Dim backdrop — Esc handled globally, click also closes.
    let st = state.clone();
    let mut overlay = panel(rect, VisualStyle::filled(theme::SIDEBAR).alpha(200))
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move || st.update(|app| app.show_cmdk = false));

    let card_w = 520.0;
    let card_h = 190.0;
    let cx = rect.left + (rect.right - rect.left) / 2.0;
    let cy = rect.top + (rect.bottom - rect.top) / 2.0 - 120.0;
    let card = UiRect::new(cx - card_w / 2.0, cy, cx + card_w / 2.0, cy + card_h);
    overlay = overlay.child(panel(card, VisualStyle::filled(theme::SURFACE).radius(10.0)));

    // Header
    overlay = overlay.child(text(
        UiRect::new(card.left + 16.0, card.top + 14.0, card.right - 16.0, card.top + 32.0),
        "✦ Inline AI Instruction",
        theme::sans_semibold(theme::ZINC_100, theme::UI_SIZE),
    ));
    overlay = overlay.child(text(
        UiRect::new(card.left + 16.0, card.top + 34.0, card.right - 16.0, card.top + 50.0),
        "Scope: Selection <Lines 10-14>",
        theme::mono(theme::ZINC_500, theme::SMALL),
    ));

    // Prompt input
    let inp = UiRect::new(card.left + 16.0, card.top + 56.0, card.right - 16.0, card.top + 84.0);
    overlay = overlay.child(panel(inp, VisualStyle::filled(theme::BG).radius(6.0)));
    overlay = overlay.child(text(
        UiRect::new(inp.left + 12.0, card.top + 62.0, inp.right - 12.0, card.top + 78.0),
        "e.g., 'Wrap in distributed transaction with CancellationToken'...",
        theme::mono(theme::ZINC_500, theme::SMALL),
    ));

    // Actions
    let st = state.clone();
    let run = UiRect::new(card.right - 150.0, card.top + 140.0, card.right - 16.0, card.top + 168.0);
    let run_btn = panel(run, VisualStyle::filled(theme::ACCENT).radius(6.0))
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move || {
            st.update(|app| {
                app.show_cmdk = false;
                app.toast = Some("Instruction queued (demo)".to_string());
            })
        })
        .child(text(
            UiRect::new(run.left, card.top + 144.0, run.right, card.top + 164.0),
            "Press Enter to run",
            theme::sans_semibold(theme::ZINC_100, theme::SMALL),
        ));
    overlay = overlay.child(run_btn);

    overlay = overlay.child(text(
        UiRect::new(card.left + 16.0, card.top + 144.0, card.left + 100.0, card.top + 164.0),
        "Cancel (Esc)",
        theme::sans(theme::ZINC_500, theme::SMALL),
    ));

    overlay
}
