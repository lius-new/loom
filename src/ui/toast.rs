//! Toast notification pill.

use lgui::prelude::{Element, State, UiRect, VisualStyle, panel, text};

use crate::state::AppState;
use crate::theme;

pub fn render(rect: UiRect, state: State<AppState>) -> Element {
    let s = state.get();
    let msg = match &s.toast {
        Some(m) => m.clone(),
        None => {
            return panel(
                UiRect::new(0.0, 0.0, 0.0, 0.0),
                VisualStyle::filled(theme::c().bg),
            );
        }
    };

    let cx = rect.left + (rect.right - rect.left) / 2.0;
    let w = 320.0;
    let pill = UiRect::new(
        cx - w / 2.0,
        rect.bottom - 40.0,
        cx + w / 2.0,
        rect.bottom - 16.0,
    );
    let mut p = panel(pill, VisualStyle::filled(theme::c().surface).radius(12.0));
    p = p.child(text(
        UiRect::new(
            pill.left + 14.0,
            pill.top + 4.0,
            pill.left + 34.0,
            pill.bottom,
        ),
        "✦",
        theme::sans(theme::c().accent, theme::SMALL),
    ));
    p = p.child(text(
        UiRect::new(
            pill.left + 34.0,
            pill.top + 4.0,
            pill.right - 14.0,
            pill.bottom,
        ),
        msg,
        theme::sans(theme::c().text, theme::SMALL),
    ));
    p
}
