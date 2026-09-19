//! Command palette overlay (Cmd/Ctrl+P): dim backdrop + centered file picker.

use lgui::core::EventPolicy;
use lgui::prelude::{panel, text, Element, State, UiRect, VisualStyle};

use crate::model::document::{meta, FILE_ORDER};
use crate::state::AppState;
use crate::theme;

pub fn render(rect: UiRect, state: State<AppState>) -> Element {
    let s = state.get();

    // Dim backdrop (click closes the palette)
    let st = state.clone();
    let mut overlay = panel(rect, VisualStyle::filled(theme::SIDEBAR).alpha(180))
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move || st.update(|app| app.show_palette = false));

    let card_w = 480.0;
    let card_h = 300.0;
    let cx = rect.left + (rect.right - rect.left) / 2.0;
    let cy = rect.top + (rect.bottom - rect.top) / 2.0 - 60.0;
    let card = UiRect::new(cx - card_w / 2.0, cy, cx + card_w / 2.0, cy + card_h);
    overlay = overlay.child(panel(card, VisualStyle::filled(theme::SURFACE).radius(10.0)));

    // Search input
    let inp = UiRect::new(card.left + 14.0, card.top + 14.0, card.right - 14.0, card.top + 40.0);
    overlay = overlay.child(panel(inp, VisualStyle::filled(theme::BG).radius(6.0)));
    overlay = overlay.child(text(
        UiRect::new(inp.left + 12.0, card.top + 19.0, inp.right - 12.0, card.top + 35.0),
        "Type file name or command (> Format, > Run tests)...",
        theme::mono(theme::ZINC_500, theme::SMALL),
    ));

    // Result rows
    let mut y = card.top + 54.0;
    for id in FILE_ORDER {
        let m = meta(id);
        let row = UiRect::new(card.left + 8.0, y, card.right - 8.0, y + 30.0);
        let st = state.clone();
        let row_el = panel(row, VisualStyle::filled(theme::ACTIVE_LINE))
            .event_policy(EventPolicy::INTERACTIVE)
            .on_click(move || {
                st.update(move |app| {
                    app.workspace.set_active(id);
                    app.show_palette = false;
                })
            })
            .child(text(
                UiRect::new(row.left + 10.0, y + 6.0, row.left + 34.0, y + 24.0),
                m.lang.badge(),
                theme::mono_bold(m.lang.badge_color(), theme::SMALL),
            ))
            .child(text(
                UiRect::new(row.left + 34.0, y + 6.0, row.left + 160.0, y + 24.0),
                m.name,
                theme::mono(theme::ZINC_200, theme::UI_SIZE),
            ))
            .child(text(
                UiRect::new(row.right - 90.0, y + 6.0, row.right - 10.0, y + 24.0),
                m.dir,
                theme::mono(theme::ZINC_500, theme::SMALL),
            ));
        overlay = overlay.child(row_el);
        y += 30.0;
    }

    let _ = s;
    overlay
}
