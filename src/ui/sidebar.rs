//! File drawer: filter input, workspace files and dependencies.

use lgui::core::{ellipse, EventPolicy};
use lgui::prelude::{panel, text, Element, State, UiRect, VisualStyle};

use crate::model::document::{meta, FILE_ORDER};
use crate::state::AppState;
use crate::theme;

pub fn render(rect: UiRect, state: State<AppState>) -> Element {
    let s = state.get();
    let active = s.workspace.active();

    let mut bar = panel(rect, VisualStyle::filled(theme::SIDEBAR));

    // Filter input (decorative)
    let flt = UiRect::new(rect.left + 12.0, rect.top + 12.0, rect.right - 12.0, rect.top + 38.0);
    bar = bar.child(panel(flt, VisualStyle::filled(theme::SURFACE).radius(6.0)));
    bar = bar.child(text(
        UiRect::new(flt.left + 10.0, rect.top + 17.0, flt.right, rect.top + 33.0),
        "⌕ Filter files...",
        theme::mono(theme::ZINC_500, theme::SMALL),
    ));

    let mut y = rect.top + 52.0;

    // Active Workspace
    bar = bar.child(text(
        UiRect::new(rect.left + 14.0, y, rect.right, y + 18.0),
        "ACTIVE WORKSPACE",
        theme::sans_semibold(theme::ZINC_500, theme::SMALL),
    ));
    y += 24.0;

    for id in FILE_ORDER {
        let m = meta(id);
        let row = UiRect::new(rect.left, y, rect.right, y + 26.0);
        let bg = if active == id {
            theme::SURFACE
        } else {
            theme::SIDEBAR
        };
        let st = state.clone();
        let row_el = panel(row, VisualStyle::filled(bg))
            .event_policy(EventPolicy::INTERACTIVE)
            .on_click(move || st.update(move |app| app.workspace.set_active(id)));

        let mut row_el = row_el.child(text(
            UiRect::new(rect.left + 14.0, y + 5.0, rect.left + 40.0, y + 23.0),
            m.lang.badge(),
            theme::mono_bold(m.lang.badge_color(), theme::SMALL),
        ));
        row_el = row_el.child(text(
            UiRect::new(rect.left + 40.0, y + 5.0, rect.right - 24.0, y + 23.0),
            m.name,
            theme::mono(theme::ZINC_300, theme::UI_SIZE),
        ));
        if let Some(dot) = m.git_dot {
            row_el = row_el.child(ellipse(
                UiRect::new(rect.right - 22.0, y + 10.0, rect.right - 16.0, y + 16.0),
                VisualStyle::filled(dot.color()),
            ));
        }
        bar = bar.child(row_el);
        y += 26.0;
    }

    // Dependencies
    y += 10.0;
    bar = bar.child(text(
        UiRect::new(rect.left + 14.0, y, rect.right, y + 18.0),
        "DEPENDENCIES",
        theme::sans_semibold(theme::ZINC_500, theme::SMALL),
    ));
    y += 24.0;
    bar = bar.child(text(
        UiRect::new(rect.left + 14.0, y + 5.0, rect.left + 40.0, y + 23.0),
        "cs",
        theme::mono_bold(theme::ZINC_400, theme::SMALL),
    ));
    bar = bar.child(text(
        UiRect::new(rect.left + 40.0, y + 5.0, rect.right - 12.0, y + 23.0),
        "GameServer.csproj",
        theme::mono(theme::ZINC_600, theme::UI_SIZE),
    ));

    bar
}
