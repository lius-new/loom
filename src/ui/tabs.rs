//! Editor tab strip: buffer tabs with language badges, a close button, and
//! the terminal control on the right.

use lgui::core::{Color, EventPolicy, IconStyle, UiElement, UiFocusHandle, UiId, precompiled};
use lgui::prelude::{Element, State, UiRect, VisualStyle, panel, text};
use lgui::text::{self, TextLayoutRequest};

use crate::state::AppState;
use crate::terminal_session::{ShellKind, TerminalTabs};
use crate::theme;

/// Horizontal padding inside each tab pill (px-3 in the mockup).
pub(super) const PAD: f32 = 12.0;
/// Gap between badge, name and close icon (gap-2 in the mockup).
pub(super) const GAP: f32 = 8.0;
/// Gap between adjacent tab pills.
pub(super) const TAB_GAP: f32 = 4.0;
/// Extra right margin inside text rects so the last glyph is not clipped.
pub(super) const TEXT_MARGIN: f32 = 6.0;
/// Left padding of the tab cluster. The strip itself has no padding — each
/// cluster (tabs / controls) applies its own, so parent padding never leaks
/// out as an outer margin.
pub(super) const TABS_PAD: f32 = 8.0;
/// Vertical inset of each tab pill inside the strip.
pub(super) const PILL_INSET: f32 = 4.0;
/// VS Code-style upper bound: long names truncate instead of allowing one tab
/// to grow underneath the right-side controls.
pub(super) const MAX_TAB_W: f32 = 220.0;
/// Keeps the badge, an ellipsis and the close button usable when tabs shrink.
pub(super) const MIN_TAB_W: f32 = 96.0;
const CLUSTER_PAD: f32 = 8.0;
const ICON: f32 = 14.0;
const DIRTY_MARK: &str = "●";

/// Natural text width measured with the renderer's text system, falling back
/// to a per-character estimate when no text system is installed.
pub(super) fn measure(s: &str, size: f32, weight: i32) -> f32 {
    let bounds = UiRect::new(0.0, 0.0, 10_000.0, size);
    let mut request = TextLayoutRequest::single_line(s, bounds, size, weight);
    request.font_families = theme::MONO_FAMILIES;
    text::layout(&request)
        .map(|layout| layout.width)
        .unwrap_or_else(|| s.chars().count() as f32 * theme::CHAR_W * (size / theme::CODE_SIZE))
}

pub(super) fn ellipsize(value: &str, max_width: f32, size: f32, weight: i32) -> String {
    if measure(value, size, weight) <= max_width {
        return value.to_owned();
    }

    const ELLIPSIS: &str = "…";
    if max_width <= measure(ELLIPSIS, size, weight) {
        return ELLIPSIS.to_owned();
    }

    let chars: Vec<char> = value.chars().collect();
    let mut low = 0;
    let mut high = chars.len();
    while low < high {
        let middle = (low + high + 1) / 2;
        let candidate = chars[..middle]
            .iter()
            .copied()
            .chain(ELLIPSIS.chars())
            .collect::<String>();
        if measure(&candidate, size, weight) <= max_width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }

    chars[..low]
        .iter()
        .copied()
        .chain(ELLIPSIS.chars())
        .collect()
}

/// A color-tinted, resolution-independent SVG icon (rasterized at physical pixels).
fn icon(id: &'static str, key: &'static str, rect: UiRect, color: Color) -> Element {
    precompiled(UiElement::icon(UiId::new(id), rect, key).icon_style(IconStyle::new(color)))
}

pub fn render(
    rect: UiRect,
    state: State<AppState>,
    editor_focus: UiFocusHandle,
    terminal_focus: UiFocusHandle,
    terminal_tabs: State<TerminalTabs>,
) -> Element {
    let s = state.get();
    let active = s.workspace.active();

    let mut bar = panel(rect, VisualStyle::filled(theme::SIDEBAR));
    let controls_left = rect.right - CLUSTER_PAD - ICON - CLUSTER_PAD - 1.0;
    let tab_count = s.workspace.open_files().len().max(1) as f32;
    let available_tabs_w = (controls_left - rect.left - TABS_PAD).max(MIN_TAB_W);
    let tab_cap =
        ((available_tabs_w - TAB_GAP * (tab_count - 1.0)) / tab_count).clamp(MIN_TAB_W, MAX_TAB_W);

    // Tabs are content-sized rounded pills, inset PILL_INSET vertically,
    // separated by TAB_GAP. The strip has no padding; this cluster's left
    // padding is TABS_PAD.
    let mut x = rect.left + TABS_PAD;
    if s.workspace.open_files().is_empty() {
        let label = "welcome";
        let label_w = measure(label, theme::UI_SIZE, 400);
        let tab_w = (PAD * 2.0 + label_w + TEXT_MARGIN)
            .min(tab_cap)
            .max(MIN_TAB_W);
        let pill = UiRect::new(
            x,
            rect.top + PILL_INSET,
            x + tab_w,
            rect.bottom - PILL_INSET,
        );
        bar = bar.child(
            theme::bordered(pill, theme::BG, theme::BORDER, 2.0, 1.0).child(text(
                UiRect::new(pill.left + PAD, pill.top, pill.right - PAD, pill.bottom),
                label,
                theme::mono(theme::ZINC_100, theme::UI_SIZE),
            )),
        );
    }
    for &id in s.workspace.open_files() {
        let Some(m) = s.workspace.meta(id) else {
            continue;
        };
        let badge_w = measure(m.lang.badge(), theme::SMALL, 700);
        let name_w = measure(&m.name, theme::UI_SIZE, 400);
        let dirty = s.workspace.is_dirty(id);
        let dirty_w = if dirty {
            measure(DIRTY_MARK, theme::SMALL, 400)
        } else {
            0.0
        };
        let close_w = measure("✕", theme::SMALL, 400);
        let dirty_slot_w = if dirty { dirty_w + GAP } else { 0.0 };
        let fixed_w = PAD + badge_w + GAP + GAP + dirty_slot_w + close_w + PAD;
        let tab_w = (fixed_w + name_w).min(tab_cap).max(MIN_TAB_W);

        let pill = UiRect::new(
            x,
            rect.top + PILL_INSET,
            x + tab_w,
            rect.bottom - PILL_INSET,
        );
        let name_style = if Some(id) == active {
            theme::mono(theme::ZINC_100, theme::UI_SIZE)
        } else {
            theme::mono(theme::ZINC_400, theme::UI_SIZE)
        };

        let badge_left = pill.left + PAD;
        let name_left = badge_left + badge_w + GAP;
        let close_left = pill.right - PAD - close_w;
        let dirty_left = close_left - GAP - dirty_w;
        let name_right = if dirty {
            dirty_left - GAP
        } else {
            close_left - GAP
        };
        let name_slot_w = (name_right - name_left).max(0.0);
        let display_name = ellipsize(
            &m.name,
            (name_slot_w - TEXT_MARGIN).max(0.0),
            theme::UI_SIZE,
            400,
        );

        let st = state.clone();
        let focus = editor_focus.clone();
        // Active tab gets a crisp 1px border; see theme::bordered for why a
        // filled ring is used instead of a stroked outline.
        let mut tab_el = if Some(id) == active {
            theme::bordered(pill, theme::BG, theme::BORDER, 2.0, 1.0)
        } else {
            panel(pill, VisualStyle::default().radius(2.0))
        }
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move || {
            st.update(move |app| app.workspace.set_active(id));
            focus.focus();
        });

        tab_el = tab_el.child(text(
            UiRect::new(
                badge_left,
                pill.top,
                badge_left + badge_w + TEXT_MARGIN,
                pill.bottom,
            ),
            m.lang.badge(),
            theme::mono_bold(m.lang.badge_color(), theme::SMALL),
        ));
        tab_el = tab_el.child(text(
            UiRect::new(name_left, pill.top, name_left + name_slot_w, pill.bottom),
            display_name,
            name_style,
        ));
        if dirty {
            tab_el = tab_el.child(text(
                UiRect::new(
                    dirty_left,
                    pill.top,
                    dirty_left + dirty_w + TEXT_MARGIN,
                    pill.bottom,
                ),
                DIRTY_MARK,
                theme::mono(theme::ACCENT, theme::SMALL),
            ));
        }

        // Close button
        {
            let st = state.clone();
            let focus = editor_focus.clone();
            tab_el = tab_el.child(
                panel(
                    UiRect::new(close_left, pill.top, pill.right - PAD, pill.bottom),
                    VisualStyle::default(),
                )
                .event_policy(EventPolicy::INTERACTIVE)
                .on_click(move || {
                    st.update(move |app| app.workspace.close(id));
                    focus.focus();
                })
                .child(text(
                    UiRect::new(
                        close_left,
                        pill.top,
                        close_left + close_w + TEXT_MARGIN,
                        pill.bottom,
                    ),
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
    let terminal_sessions = terminal_tabs;
    let editor_target = editor_focus.clone();
    let terminal_target = terminal_focus;
    let btn = UiRect::new(icon_r.left - CLUSTER_PAD, rect.top, rect.right, rect.bottom);
    let tt_btn = panel(btn, VisualStyle::filled(theme::SIDEBAR))
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move || {
            let opening = !st.get().show_terminal;
            if opening && terminal_sessions.get().is_empty() {
                terminal_sessions.update(|tabs| {
                    tabs.add(ShellKind::default());
                });
            }
            st.update(|app| app.show_terminal = !app.show_terminal);
            if opening {
                terminal_target.focus();
            } else {
                editor_target.focus();
            }
        })
        .child(icon("tabs.terminal", "terminal", icon_r, theme::ZINC_400));
    bar = bar.child(tt_btn);

    // Hairline bottom border (matches the title bar).
    bar = bar.child(panel(
        UiRect::new(rect.left, rect.bottom - 1.0, rect.right, rect.bottom),
        VisualStyle::filled(theme::BORDER),
    ));

    bar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_file_names_end_with_an_ellipsis_and_fit_the_budget() {
        let budget = measure("very-long…", theme::UI_SIZE, 400);
        let result = ellipsize(
            "very-long-file-name-that-keeps-going.rs",
            budget,
            theme::UI_SIZE,
            400,
        );

        assert!(result.ends_with('…'));
        assert!(measure(&result, theme::UI_SIZE, 400) <= budget);
    }

    #[test]
    fn short_file_names_remain_unchanged() {
        assert_eq!(ellipsize("main.rs", 200.0, theme::UI_SIZE, 400), "main.rs");
    }
}
