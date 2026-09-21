//! Editor tab strip: buffer tabs with language badges, a close button, and
//! the Cmd+K / terminal controls on the right.

use lgui::core::EventPolicy;
use lgui::prelude::{panel, text, Element, State, UiRect, VisualStyle};
use lgui::text::measure_width;

use crate::model::document::meta;
use crate::state::AppState;
use crate::theme;

/// Horizontal padding inside each tab pill (px-3 in the mockup).
const PAD: f32 = 12.0;
/// Gap between badge, name and close icon (gap-2 in the mockup).
const GAP: f32 = 8.0;
/// Gap between adjacent tab pills.
const TAB_GAP: f32 = 4.0;
/// Extra right margin inside text rects so the last glyph is not clipped.
const TEXT_MARGIN: f32 = 6.0;

/// Natural text width measured with the renderer's text system, falling back
/// to a per-character estimate when no text system is installed.
fn measure(s: &str, size: f32, weight: i32) -> f32 {
    let bounds = UiRect::new(0.0, 0.0, 10_000.0, size);
    measure_width(s, bounds, size, weight).unwrap_or_else(|| {
        s.chars().count() as f32 * theme::CHAR_W * (size / theme::CODE_SIZE)
    })
}

pub fn render(rect: UiRect, state: State<AppState>) -> Element {
    let s = state.get();
    let active = s.workspace.active();

    let mut bar = panel(rect, VisualStyle::filled(theme::SIDEBAR));

    // Tabs are content-sized rounded pills, inset 2px vertically, separated
    // by a 4px gap, with a hairline bottom border under the whole strip.
    let mut x = rect.left + 8.0;
    for &id in s.workspace.open_files() {
        let m = meta(id);
        let badge_w = measure(m.lang.badge(), theme::SMALL, 700);
        let name_w = measure(m.name, theme::UI_SIZE, 400);
        let close_w = measure("✕", theme::SMALL, 400);
        let tab_w = PAD + badge_w + GAP + name_w + GAP + close_w + PAD;

        let pill = UiRect::new(x, rect.top + 4.0, x + tab_w, rect.bottom - 4.0);
        let name_style = if id == active {
            theme::mono(theme::ZINC_100, theme::UI_SIZE)
        } else {
            theme::mono(theme::ZINC_400, theme::UI_SIZE)
        };

        let badge_left = pill.left + PAD;
        let name_left = badge_left + badge_w + GAP;
        let close_left = pill.right - PAD - close_w;

        let st = state.clone();
        // Active tab gets a crisp 1px border; see theme::bordered for why a
        // filled ring is used instead of a stroked outline.
        let mut tab_el = if id == active {
            theme::bordered(pill, theme::BG, theme::BORDER, 2.0, 1.0)
        } else {
            panel(pill, VisualStyle::default().radius(2.0))
        }
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move || st.update(move |app| app.workspace.set_active(id)));

        tab_el = tab_el.child(text(
            UiRect::new(badge_left, pill.top, badge_left + badge_w + TEXT_MARGIN, pill.bottom),
            m.lang.badge(),
            theme::mono_bold(m.lang.badge_color(), theme::SMALL),
        ));
        tab_el = tab_el.child(text(
            UiRect::new(name_left, pill.top, name_left + name_w + TEXT_MARGIN, pill.bottom),
            m.name,
            name_style,
        ));

        // Close button (last remaining tab cannot be closed)
        if s.workspace.open_files().len() > 1 {
            let st = state.clone();
            tab_el = tab_el.child(
                panel(
                    UiRect::new(close_left, pill.top, pill.right - PAD, pill.bottom),
                    VisualStyle::default(),
                )
                .event_policy(EventPolicy::INTERACTIVE)
                .on_click(move || st.update(move |app| app.workspace.close(id)))
                .child(text(
                    UiRect::new(close_left, pill.top, close_left + close_w + TEXT_MARGIN, pill.bottom),
                    "✕",
                    theme::mono(theme::ZINC_500, theme::SMALL),
                )),
            );
        }

        bar = bar.child(tab_el);
        x += tab_w + TAB_GAP;
    }

    // Right controls: Cmd+K sparkles + terminal toggle
    let st = state.clone();
    let ck = UiRect::new(rect.right - 140.0, rect.top + 4.0, rect.right - 78.0, rect.bottom - 4.0);
    let ck_btn = panel(ck, VisualStyle::default())
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move || st.update(|app| app.show_cmdk = true))
        .child(text(
            UiRect::new(ck.left + 6.0, rect.top + 4.0, ck.right, rect.bottom - 4.0),
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
            UiRect::new(tt.left + 6.0, rect.top + 4.0, tt.right, rect.bottom - 4.0),
            "❯_",
            theme::mono(theme::ZINC_400, theme::SMALL),
        ));
    bar = bar.child(tt_btn);

    // Hairline bottom border (matches the title bar).
    bar = bar.child(panel(
        UiRect::new(rect.left, rect.bottom - 1.0, rect.right, rect.bottom),
        VisualStyle::filled(theme::BORDER),
    ));

    bar
}
