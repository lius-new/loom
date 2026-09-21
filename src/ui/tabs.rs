//! Editor tab strip: buffer tabs with language badges, a close button, and
//! the terminal control on the right.

use lgui::core::{Color, EventPolicy, IconStyle, UiElement, UiId, precompiled};
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
/// Left padding of the tab cluster. The strip itself has no padding — each
/// cluster (tabs / controls) applies its own, so parent padding never leaks
/// out as an outer margin.
const TABS_PAD: f32 = 8.0;
/// Vertical inset of each tab pill inside the strip.
const PILL_INSET: f32 = 4.0;

/// Natural text width measured with the renderer's text system, falling back
/// to a per-character estimate when no text system is installed.
fn measure(s: &str, size: f32, weight: i32) -> f32 {
    let bounds = UiRect::new(0.0, 0.0, 10_000.0, size);
    measure_width(s, bounds, size, weight).unwrap_or_else(|| {
        s.chars().count() as f32 * theme::CHAR_W * (size / theme::CODE_SIZE)
    })
}

/// A color-tinted, resolution-independent SVG icon (rasterized at physical pixels).
fn icon(id: &'static str, key: &'static str, rect: UiRect, color: Color) -> Element {
    precompiled(UiElement::icon(UiId::new(id), rect, key).icon_style(IconStyle::new(color)))
}

pub fn render(rect: UiRect, state: State<AppState>) -> Element {
    let s = state.get();
    let active = s.workspace.active();

    let mut bar = panel(rect, VisualStyle::filled(theme::SIDEBAR));

    // Tabs are content-sized rounded pills, inset PILL_INSET vertically,
    // separated by TAB_GAP. The strip has no padding; this cluster's left
    // padding is TABS_PAD.
    let mut x = rect.left + TABS_PAD;
    for &id in s.workspace.open_files() {
        let m = meta(id);
        let badge_w = measure(m.lang.badge(), theme::SMALL, 700);
        let name_w = measure(m.name, theme::UI_SIZE, 400);
        let close_w = measure("✕", theme::SMALL, 400);
        let tab_w = PAD + badge_w + GAP + name_w + GAP + close_w + PAD;

        let pill = UiRect::new(x, rect.top + PILL_INSET, x + tab_w, rect.bottom - PILL_INSET);
        let name_style = if Some(id) == active {
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
        let mut tab_el = if Some(id) == active {
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

        // Close button
        {
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

    // Right control cluster: the icon-only terminal toggle sits in a
    // right-aligned cluster bounded by a 1px divider (left) and the strip
    // edge (right). The cluster applies its own padding (CLUSTER_PAD on the
    // right), so the icon's left gap (divider -> icon) and right gap
    // (icon -> strip edge) are equal. The divider spans the full strip height.
    const CLUSTER_PAD: f32 = 8.0;
    const ICON: f32 = 14.0;

    let icon_r = UiRect::new(
        rect.right - CLUSTER_PAD - ICON,
        rect.top + 7.0,
        rect.right - CLUSTER_PAD,
        rect.bottom - 7.0,
    );

    // Left border of the cluster: a full-height 1px vertical divider.
    let divider = UiRect::new(
        icon_r.left - CLUSTER_PAD - 1.0,
        rect.top,
        icon_r.left - CLUSTER_PAD,
        rect.bottom,
    );
    bar = bar.child(panel(divider, VisualStyle::filled(theme::BORDER)));

    // Click target spans the whole cluster, full height, with an opaque
    // SIDEBAR fill (matching the strip) so any tab pills scrolling underneath
    // stay hidden behind the control cluster.
    let st = state.clone();
    let btn = UiRect::new(
        icon_r.left - CLUSTER_PAD,
        rect.top,
        rect.right,
        rect.bottom,
    );
    let tt_btn = panel(btn, VisualStyle::filled(theme::SIDEBAR))
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move || st.update(|app| app.show_terminal = !app.show_terminal))
        .child(icon("tabs.terminal", "terminal", icon_r, theme::ZINC_400));
    bar = bar.child(tt_btn);

    // Hairline bottom border (matches the title bar).
    bar = bar.child(panel(
        UiRect::new(rect.left, rect.bottom - 1.0, rect.right, rect.bottom),
        VisualStyle::filled(theme::BORDER),
    ));

    bar
}
