//! Command palette overlay (Cmd/Ctrl+P): currently open file picker.

use lgui::core::{EventPolicy, UiFocusHandle};
use lgui::prelude::{panel, text, Element, State, UiRect, VisualStyle};

use crate::state::AppState;
use crate::theme;

pub fn render(rect: UiRect, state: State<AppState>, editor_focus: UiFocusHandle) -> Element {
    let s = state.get();

    let st = state.clone();
    let mut overlay = panel(rect, VisualStyle::filled(theme::SIDEBAR).alpha(180))
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move || st.update(|app| app.show_palette = false));

    let card_w = 480.0;
    let card_h = 300.0;
    let cx = rect.left + (rect.right - rect.left) / 2.0;
    let cy = rect.top + (rect.bottom - rect.top) / 2.0 - 60.0;
    let card = UiRect::new(cx - card_w / 2.0, cy, cx + card_w / 2.0, cy + card_h);
    overlay = overlay.child(panel(
        card,
        VisualStyle::filled(theme::SURFACE).radius(10.0),
    ));

    let input = UiRect::new(
        card.left + 14.0,
        card.top + 14.0,
        card.right - 14.0,
        card.top + 40.0,
    );
    overlay = overlay.child(panel(input, VisualStyle::filled(theme::BG).radius(6.0)));
    overlay = overlay.child(text(
        UiRect::new(
            input.left + 12.0,
            card.top + 19.0,
            input.right - 12.0,
            card.top + 35.0,
        ),
        "Open files",
        theme::mono(theme::ZINC_500, theme::SMALL),
    ));

    let mut y = card.top + 54.0;
    if s.workspace.open_files().is_empty() {
        overlay = overlay.child(text(
            UiRect::new(card.left + 18.0, y + 8.0, card.right - 18.0, y + 28.0),
            "No files open — select one in the file tree",
            theme::mono(theme::ZINC_500, theme::UI_SIZE),
        ));
        return overlay;
    }

    for &id in s.workspace.open_files() {
        let Some(meta) = s.workspace.meta(id) else {
            continue;
        };
        let name = meta.name.clone();
        let directory = meta.directory_label();
        let language = meta.lang;
        let row = UiRect::new(card.left + 8.0, y, card.right - 8.0, y + 30.0);
        let st = state.clone();
        let focus = editor_focus.clone();
        let row_style = if Some(id) == s.workspace.active() {
            VisualStyle::filled(theme::ACTIVE_LINE)
        } else {
            VisualStyle::default()
        };
        let row_element = panel(row, row_style)
            .event_policy(EventPolicy::INTERACTIVE)
            .on_click(move || {
                st.update(move |app| {
                    app.workspace.set_active(id);
                    app.show_palette = false;
                });
                focus.focus();
            })
            .child(text(
                UiRect::new(row.left + 10.0, y + 6.0, row.left + 42.0, y + 24.0),
                language.badge(),
                theme::mono_bold(language.badge_color(), theme::SMALL),
            ))
            .child(text(
                UiRect::new(row.left + 42.0, y + 6.0, row.right - 120.0, y + 24.0),
                name,
                theme::mono(theme::ZINC_200, theme::UI_SIZE),
            ))
            .child(text(
                UiRect::new(row.right - 110.0, y + 6.0, row.right - 10.0, y + 24.0),
                directory,
                theme::mono(theme::ZINC_500, theme::SMALL),
            ));
        overlay = overlay.child(row_element);
        y += 30.0;
    }

    overlay
}
