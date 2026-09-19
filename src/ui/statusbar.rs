//! Status bar: cursor position, encoding, language badge, and Copilot /
//! terminal toggles on the right.

use lgui::core::{ellipse, EventPolicy};
use lgui::prelude::{panel, text, Element, State, UiRect, VisualStyle};

use crate::model::document::meta;
use crate::state::AppState;
use crate::theme;

pub fn render(rect: UiRect, state: State<AppState>) -> Element {
    let s = state.get();
    let id = s.workspace.active();
    let m = meta(id);
    let (line, col) = s.workspace.active_buffer().line_col();

    let mut bar = panel(rect, VisualStyle::filled(theme::SIDEBAR));

    let ln = format!("Ln {}, Col {}", line + 1, col + 1);
    bar = bar.child(text(
        UiRect::new(rect.left + 14.0, rect.top + 5.0, rect.left + 96.0, rect.bottom),
        ln,
        theme::mono(theme::ZINC_400, theme::SMALL),
    ));
    bar = bar.child(text(
        UiRect::new(rect.left + 100.0, rect.top + 5.0, rect.left + 160.0, rect.bottom),
        "Spaces: 2",
        theme::mono(theme::ZINC_500, theme::SMALL),
    ));
    bar = bar.child(text(
        UiRect::new(rect.left + 164.0, rect.top + 5.0, rect.left + 206.0, rect.bottom),
        "UTF-8",
        theme::mono(theme::ZINC_500, theme::SMALL),
    ));
    bar = bar.child(text(
        UiRect::new(rect.left + 210.0, rect.top + 5.0, rect.left + 236.0, rect.bottom),
        m.lang.badge(),
        theme::mono_bold(m.lang.badge_color(), theme::SMALL),
    ));

    // Right: Copilot status + terminal toggle
    let st = state.clone();
    let tt = UiRect::new(rect.right - 74.0, rect.top + 4.0, rect.right - 12.0, rect.bottom - 4.0);
    let tt_btn = panel(tt, VisualStyle::default())
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move || st.update(|app| app.show_terminal = !app.show_terminal))
        .child(text(
            UiRect::new(tt.left + 8.0, rect.top + 5.0, tt.right, rect.bottom - 2.0),
            "❯ Terminal",
            theme::mono(theme::ZINC_400, theme::SMALL),
        ));
    bar = bar.child(tt_btn);

    let cop = UiRect::new(rect.right - 260.0, rect.top + 5.0, rect.right - 84.0, rect.bottom);
    bar = bar.child(ellipse(
        UiRect::new(cop.left, rect.top + 10.0, cop.left + 6.0, rect.top + 16.0),
        VisualStyle::filled(theme::ACCENT),
    ));
    bar = bar.child(text(
        UiRect::new(cop.left + 12.0, rect.top + 5.0, cop.right, rect.bottom),
        "Copilot: In-Flow Active",
        theme::mono(theme::ZINC_400, theme::SMALL),
    ));

    bar
}
