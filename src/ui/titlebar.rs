//! Custom frameless title bar with native window controls on the far right.
//!
//! The bar is marked as a native window drag region; interactive children
//! (the buttons) are excluded automatically by hit testing.

use lgui::core::{
    Color, IconStyle, PhysicalRect, UiElement, UiEventContext, UiId, UiScale, precompiled,
};
use lgui::prelude::{Element, State, UiRect, VisualStyle, panel};

use crate::input::action::Action;
use crate::state::AppState;
use crate::theme;
use crate::window_geometry;

/// A color-tinted, resolution-independent SVG icon (rasterized at physical pixels).
fn icon(id: &'static str, key: &'static str, rect: UiRect, color: Color) -> Element {
    precompiled(UiElement::icon(UiId::new(id), rect, key).icon_style(IconStyle::new(color)))
}

/// Keep the SVG bitmap and its destination on the same physical pixel grid.
/// Auto scaling can shrink the UI below 100%; caption glyphs still need at least
/// their native 12px canvas, or a 1px edge becomes a faint fractional pixel.
fn window_icon(id: &'static str, key: &'static str, button: UiRect) -> Element {
    Element::new(move |cx| {
        let rect = window_icon_rect(button, cx.context.scale());
        UiElement::icon(UiId::new(id), rect, key).icon_style(IconStyle::new(theme::c().text_soft))
    })
}

fn window_icon_rect(button: UiRect, scale: UiScale) -> UiRect {
    let size = scale.physical_length(12.0).max(12);
    let left = scale.physical_value((button.left + button.right) / 2.0) - size / 2;
    let top = scale.physical_value((button.top + button.bottom) / 2.0) - size / 2;
    scale.logical_rect(PhysicalRect::new(left, top, left + size, top + size))
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
            theme::icon_rect(settings_r),
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
        .child(window_icon("titlebar.minimize", "window-minimize", min_r));
    bar = bar.child(min_btn);

    // Maximize / restore (toggles between maximized and windowed). On Windows 11
    // the OS owns this button so hovering it shows Snap Layouts; the click
    // handler covers other platforms.
    let max_btn = panel(max_r, VisualStyle::default())
        .event_policy(lgui::core::EventPolicy::INTERACTIVE)
        .window_maximize_button()
        .on_click(|ctx: &mut UiEventContext| {
            let _ = ctx.window().toggle_maximize();
        })
        .child(window_icon(
            "titlebar.maximize",
            if window_geometry::is_maximized() {
                "window-restore"
            } else {
                "window-maximize"
            },
            max_r,
        ));
    bar = bar.child(max_btn);

    // Close
    let close_btn = panel(close_r, VisualStyle::default())
        .event_policy(lgui::core::EventPolicy::INTERACTIVE)
        .on_click(|ctx: &mut UiEventContext| {
            let _ = ctx.window().request_close();
        })
        .child(window_icon("titlebar.close", "window-close", close_r));
    bar = bar.child(close_btn);

    // Hairline bottom border
    bar = bar.child(panel(
        UiRect::new(rect.left, rect.bottom - 1.0, rect.right, rect.bottom),
        VisualStyle::filled(theme::c().border),
    ));

    bar
}
