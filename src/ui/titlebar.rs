//! Custom frameless title bar with native window controls on the far right.
//!
//! The bar is marked as a native window drag region; interactive children
//! (the buttons) are excluded automatically by hit testing.

use lgui::core::{Color, IconStyle, UiElement, UiEventContext, UiId, precompiled};
use lgui::prelude::{Element, State, UiRect, VisualStyle, panel};

use crate::input::action::Action;
use crate::state::AppState;
use crate::theme;

/// A color-tinted, resolution-independent SVG icon (rasterized at physical pixels).
fn icon(id: &'static str, key: &'static str, rect: UiRect, color: Color) -> Element {
    precompiled(UiElement::icon(UiId::new(id), rect, key).icon_style(IconStyle::new(color)))
}

pub fn render(rect: UiRect, state: State<AppState>) -> Element {
    let mut bar = panel(rect, VisualStyle::filled(theme::c().sidebar)).window_drag_region();

    // ---- Right cluster (anchored to the window controls) --------------
    let right = rect.right - 4.0;

    // Settings, just left of the window controls.
    let settings_r = UiRect::new(right - 148.0, rect.top, right - 116.0, rect.bottom);
    let settings_open = state.get().workspace.active_is_settings();
    let settings_btn = panel(settings_r, VisualStyle::default())
        .event_policy(lgui::core::EventPolicy::INTERACTIVE)
        .cursor(lgui::core::CursorIcon::Pointer)
        .on_click(move || state.update(|app| app.apply(Action::OpenSettings)))
        .child(icon(
            "titlebar.settings",
            "settings",
            UiRect::new(
                settings_r.left + 9.0,
                rect.top + 9.0,
                settings_r.right - 9.0,
                rect.top + 23.0,
            ),
            if settings_open {
                theme::c().accent
            } else {
                theme::c().text_muted
            },
        ));
    bar = bar.child(settings_btn);

    // ---- Window controls (far right) ----------------------------------
    let min_r = UiRect::new(right - 108.0, rect.top, right - 72.0, rect.bottom);
    let max_r = UiRect::new(right - 72.0, rect.top, right - 36.0, rect.bottom);
    let close_r = UiRect::new(right - 36.0, rect.top, right, rect.bottom);

    // Minimize
    let min_btn = panel(min_r, VisualStyle::default())
        .event_policy(lgui::core::EventPolicy::INTERACTIVE)
        .on_click(|ctx: &mut UiEventContext| {
            let _ = ctx.window().minimize();
        })
        .child(icon(
            "titlebar.minimize",
            "minus",
            UiRect::new(
                min_r.left + 12.0,
                rect.top + 10.0,
                min_r.right - 12.0,
                rect.top + 22.0,
            ),
            theme::c().text_muted,
        ));
    bar = bar.child(min_btn);

    // Maximize / restore (toggles between maximized and windowed)
    let max_btn = panel(max_r, VisualStyle::default())
        .event_policy(lgui::core::EventPolicy::INTERACTIVE)
        .on_click(|ctx: &mut UiEventContext| {
            let _ = ctx.window().toggle_maximize();
        })
        .child(icon(
            "titlebar.maximize",
            "square",
            UiRect::new(
                max_r.left + 12.0,
                rect.top + 10.0,
                max_r.right - 12.0,
                rect.top + 22.0,
            ),
            theme::c().text_muted,
        ));
    bar = bar.child(max_btn);

    // Close
    let close_btn = panel(close_r, VisualStyle::default())
        .event_policy(lgui::core::EventPolicy::INTERACTIVE)
        .on_click(|ctx: &mut UiEventContext| {
            let _ = ctx.window().request_close();
        })
        .child(icon(
            "titlebar.close",
            "close",
            UiRect::new(
                close_r.left + 12.0,
                rect.top + 10.0,
                close_r.right - 12.0,
                rect.top + 22.0,
            ),
            theme::c().text_muted,
        ));
    bar = bar.child(close_btn);

    // Hairline bottom border
    bar = bar.child(panel(
        UiRect::new(rect.left, rect.bottom - 1.0, rect.right, rect.bottom),
        VisualStyle::filled(theme::c().border),
    ));

    bar
}
