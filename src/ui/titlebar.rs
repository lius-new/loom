//! Custom (frameless) title bar: git pill on the
//! left, and the three window controls (minimize / maximize / close) on the
//! far right.
//!
//! The bar is marked as a native window drag region; interactive children
//! (the buttons) are excluded automatically by hit testing.

use lgui::core::{Color, IconStyle, UiElement, UiEventContext, UiId, precompiled};
use lgui::prelude::{Element, TextVerticalAlign, UiRect, VisualStyle, panel, text};

use crate::theme;

/// A color-tinted, resolution-independent SVG icon (rasterized at physical pixels).
fn icon(id: &'static str, key: &'static str, rect: UiRect, color: Color) -> Element {
    precompiled(UiElement::icon(UiId::new(id), rect, key).icon_style(IconStyle::new(color)))
}

pub fn render(rect: UiRect) -> Element {
    let mut bar = panel(rect, VisualStyle::filled(theme::SIDEBAR)).window_drag_region();

    // ---- Left cluster --------------------------------------------------
    // Git pill
    let pill = UiRect::new(
        rect.left + 14.0,
        rect.top + 6.0,
        rect.left + 94.0,
        rect.top + 26.0,
    );
    bar = bar.child(panel(
        pill,
        VisualStyle::filled(theme::SURFACE).radius(12.0),
    ));
    bar = bar.child(icon(
        "titlebar.git",
        "git-branch",
        UiRect::new(
            pill.left + 8.0,
            rect.top + 10.0,
            pill.left + 20.0,
            rect.top + 22.0,
        ),
        theme::ZINC_300,
    ));
    bar = bar.child(text(
        UiRect::new(
            pill.left + 24.0,
            rect.top + 7.0,
            pill.right,
            rect.top + 25.0,
        ),
        "main*  +24 -3",
        theme::mono(theme::ZINC_300, theme::SMALL).vertical_align(TextVerticalAlign::XCenter),
    ));

    // ---- Right cluster (anchored to the window controls) --------------
    let right = rect.right - 4.0;

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
            theme::ZINC_400,
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
            theme::ZINC_400,
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
            theme::ZINC_400,
        ));
    bar = bar.child(close_btn);

    // Hairline bottom border
    bar = bar.child(panel(
        UiRect::new(rect.left, rect.bottom - 1.0, rect.right, rect.bottom),
        VisualStyle::filled(theme::BORDER),
    ));

    bar
}
