//! Editor tab strip: buffer tabs with language badges, a close button, and
//! the Cmd+K / terminal controls on the right.

use lgui::core::EventPolicy;
use lgui::prelude::{panel, text, Element, State, UiRect, VisualStyle};

use crate::model::document::meta;
use crate::state::AppState;
use crate::theme;

pub fn render(rect: UiRect, state: State<AppState>) -> Element {
    let s = state.get();
    let active = s.workspace.active();

    let mut bar = panel(rect, VisualStyle::filled(theme::SIDEBAR));

    let mut x = rect.left;
    for &id in s.workspace.open_files() {
        let m = meta(id);
        let tab = UiRect::new(x, rect.top, x + 170.0, rect.bottom);
        let bg = if id == active {
            theme::BG
        } else {
            theme::SIDEBAR
        };
        let st = state.clone();
        let mut tab_el = panel(tab, VisualStyle::filled(bg))
            .event_policy(EventPolicy::INTERACTIVE)
            .on_click(move || st.update(move |app| app.workspace.set_active(id)));

        tab_el = tab_el.child(text(
            UiRect::new(tab.left + 12.0, rect.top + 8.0, tab.left + 36.0, rect.bottom),
            m.lang.badge(),
            theme::mono_bold(m.lang.badge_color(), theme::SMALL),
        ));
        tab_el = tab_el.child(text(
            UiRect::new(tab.left + 36.0, rect.top + 8.0, tab.right - 28.0, rect.bottom),
            m.name,
            theme::mono(theme::ZINC_300, theme::UI_SIZE),
        ));

        // Close button (last remaining tab cannot be closed)
        if s.workspace.open_files().len() > 1 {
            let st = state.clone();
            let close = UiRect::new(tab.right - 26.0, rect.top + 8.0, tab.right - 12.0, rect.bottom);
            tab_el = tab_el.child(
                panel(close, VisualStyle::default())
                    .event_policy(EventPolicy::INTERACTIVE)
                    .on_click(move || st.update(move |app| app.workspace.close(id)))
                    .child(text(
                        UiRect::new(close.left, rect.top + 8.0, close.right, rect.bottom),
                        "✕",
                        theme::mono(theme::ZINC_500, theme::SMALL),
                    )),
            );
        }

        bar = bar.child(tab_el);
        x += 170.0;
    }

    // Right controls: Cmd+K sparkles + terminal toggle
    let st = state.clone();
    let ck = UiRect::new(rect.right - 140.0, rect.top + 4.0, rect.right - 78.0, rect.bottom - 4.0);
    let ck_btn = panel(ck, VisualStyle::default())
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move || st.update(|app| app.show_cmdk = true))
        .child(text(
            UiRect::new(ck.left + 6.0, rect.top + 7.0, ck.right, rect.bottom - 4.0),
            "✦ Cmd+K",
            theme::mono(theme::ZINC_400, theme::SMALL),
        ));
    bar = bar.child(ck_btn);

    let st = state.clone();
    let tt = UiRect::new(rect.right - 70.0, rect.top + 4.0, rect.right - 16.0, rect.bottom - 4.0);
    let tt_btn = panel(tt, VisualStyle::default())
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move || st.update(|app| app.show_terminal = !app.show_terminal))
        .child(text(
            UiRect::new(tt.left + 6.0, rect.top + 7.0, tt.right, rect.bottom - 4.0),
            "❯_",
            theme::mono(theme::ZINC_400, theme::SMALL),
        ));
    bar = bar.child(tt_btn);

    bar
}
