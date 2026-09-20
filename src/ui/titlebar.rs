//! Custom (frameless) title bar: project drawer toggle, git pill, command
//! palette trigger, test-run pill, model selector, assistant toggle, and the
//! three window controls (minimize / maximize / close) on the far right.
//!
//! The bar is marked as a native window drag region; interactive children
//! (the buttons) are excluded automatically by hit testing.

use lgui::core::{ellipse, Color, IconStyle, UiElement, UiEventContext, UiId, precompiled};
use lgui::prelude::{panel, text, Element, State, TextVerticalAlign, UiRect, VisualStyle, WindowMode};

use crate::state::AppState;
use crate::theme;

/// A color-tinted, resolution-independent SVG icon (rasterized at physical pixels).
fn icon(id: &'static str, key: &'static str, rect: UiRect, color: Color) -> Element {
    precompiled(UiElement::icon(UiId::new(id), rect, key).icon_style(IconStyle::new(color)))
}

pub fn render(rect: UiRect, state: State<AppState>) -> Element {
    let mut bar = panel(rect, VisualStyle::filled(theme::SIDEBAR)).window_drag_region();

    // ---- Left cluster --------------------------------------------------
    // Project drawer toggle
    let st = state.clone();
    let proj = UiRect::new(rect.left + 14.0, rect.top + 8.0, rect.left + 86.0, rect.top + 32.0);
    let proj_btn = panel(proj, VisualStyle::default())
        .event_policy(lgui::core::EventPolicy::INTERACTIVE)
        .on_click(move || st.update(|app| app.show_drawer = !app.show_drawer))
        .child(icon(
            "titlebar.project",
            "panel-left",
            UiRect::new(proj.left + 7.0, rect.top + 14.0, proj.left + 19.0, rect.top + 26.0),
            theme::ACCENT,
        ))
        .child(text(
            UiRect::new(proj.left + 26.0, rect.top + 10.0, proj.right, rect.top + 30.0),
            "Project",
            theme::sans_semibold(theme::ACCENT, theme::UI_SIZE)
                .vertical_align(TextVerticalAlign::XCenter),
        ));
    bar = bar.child(proj_btn);

    // Git pill
    let pill = UiRect::new(rect.left + 94.0, rect.top + 10.0, rect.left + 174.0, rect.top + 30.0);
    bar = bar.child(panel(pill, VisualStyle::filled(theme::SURFACE).radius(12.0)));
    bar = bar.child(icon(
        "titlebar.git",
        "git-branch",
        UiRect::new(pill.left + 8.0, rect.top + 14.0, pill.left + 20.0, rect.top + 26.0),
        theme::ZINC_300,
    ));
    bar = bar.child(text(
        UiRect::new(pill.left + 24.0, rect.top + 11.0, pill.right, rect.top + 29.0),
        "main*  +24 -3",
        theme::mono(theme::ZINC_300, theme::SMALL).vertical_align(TextVerticalAlign::XCenter),
    ));

    // ---- Center: command palette trigger ------------------------------
    let st = state.clone();
    let cx = rect.left + (rect.right - rect.left) / 2.0;
    let trig = UiRect::new(cx - 172.0, rect.top + 7.0, cx + 118.0, rect.top + 33.0);
    let trig_btn = panel(trig, VisualStyle::filled(theme::SURFACE).radius(8.0))
        .event_policy(lgui::core::EventPolicy::INTERACTIVE)
        .on_click(move || st.update(|app| app.show_palette = true))
        .child(icon(
            "titlebar.search",
            "search",
            UiRect::new(trig.left + 10.0, rect.top + 14.0, trig.left + 22.0, rect.top + 26.0),
            theme::ZINC_400,
        ))
        .child(text(
            UiRect::new(trig.left + 28.0, rect.top + 10.0, trig.left + 220.0, rect.top + 30.0),
            "src > Services > WalletService.cs",
            theme::mono(theme::ZINC_400, theme::SMALL).vertical_align(TextVerticalAlign::XCenter),
        ))
        .child(icon(
            "titlebar.command",
            "command",
            UiRect::new(trig.right - 34.0, rect.top + 14.0, trig.right - 22.0, rect.top + 26.0),
            theme::ZINC_500,
        ));
    bar = bar.child(trig_btn);

    // ---- Right cluster (anchored to the window controls) --------------
    let right = rect.right - 8.0;

    // Test run pill
    let tp = UiRect::new(right - 312.0, rect.top + 10.0, right - 236.0, rect.top + 30.0);
    bar = bar.child(panel(tp, VisualStyle::filled(theme::SURFACE).radius(12.0)));
    bar = bar.child(icon(
        "titlebar.run",
        "play",
        UiRect::new(tp.left + 8.0, rect.top + 14.0, tp.left + 20.0, rect.top + 26.0),
        theme::EMERALD_400,
    ));
    bar = bar.child(text(
        UiRect::new(tp.left + 24.0, rect.top + 11.0, tp.right, rect.top + 29.0),
        "Test Run",
        theme::mono(theme::EMERALD_400, theme::SMALL).vertical_align(TextVerticalAlign::XCenter),
    ));

    // Model selector (static)
    let ms = UiRect::new(right - 232.0, rect.top + 8.0, right - 160.0, rect.top + 32.0);
    bar = bar.child(panel(ms, VisualStyle::filled(theme::SURFACE).radius(6.0)));
    bar = bar.child(ellipse(
        UiRect::new(ms.left + 10.0, rect.top + 17.0, ms.left + 16.0, rect.top + 23.0),
        VisualStyle::filled(theme::ACCENT),
    ));
    bar = bar.child(text(
        UiRect::new(ms.left + 22.0, rect.top + 11.0, ms.right - 8.0, rect.top + 29.0),
        "Claude 3.7",
        theme::mono(theme::ZINC_300, theme::SMALL).vertical_align(TextVerticalAlign::XCenter),
    ));

    // Assistant toggle
    let st = state.clone();
    let ab = UiRect::new(right - 152.0, rect.top + 10.0, right - 136.0, rect.top + 30.0);
    let ab_btn = panel(ab, VisualStyle::default())
        .event_policy(lgui::core::EventPolicy::INTERACTIVE)
        .on_click(move || st.update(|app| app.show_assistant = !app.show_assistant))
        .child(icon(
            "titlebar.assistant",
            "sparkles",
            UiRect::new(ab.left, rect.top + 11.0, ab.right, rect.top + 29.0),
            theme::ACCENT,
        ));
    bar = bar.child(ab_btn);

    // ---- Window controls (far right) ----------------------------------
    let min_r = UiRect::new(right - 132.0, rect.top, right - 88.0, rect.bottom);
    let max_r = UiRect::new(right - 88.0, rect.top, right - 44.0, rect.bottom);
    let close_r = UiRect::new(right - 44.0, rect.top, right, rect.bottom);

    // Minimize
    let min_btn = panel(min_r, VisualStyle::default())
        .event_policy(lgui::core::EventPolicy::INTERACTIVE)
        .on_click(|ctx: &mut UiEventContext| {
            let _ = ctx.window().minimize();
        })
        .child(icon(
            "titlebar.minimize",
            "minus",
            UiRect::new(min_r.left + 14.0, rect.top + 12.0, min_r.right - 14.0, rect.top + 28.0),
            theme::ZINC_400,
        ));
    bar = bar.child(min_btn);

    // Maximize / restore (toggles fullscreen)
    let st = state.clone();
    let max_btn = panel(max_r, VisualStyle::default())
        .event_policy(lgui::core::EventPolicy::INTERACTIVE)
        .on_click(move |ctx: &mut UiEventContext| {
            let maximized = st.get().window_maximized;
            let _ = ctx
                .window()
                .set_mode(if maximized { WindowMode::Windowed } else { WindowMode::Fullscreen });
            st.update(|app| app.window_maximized = !app.window_maximized);
        })
        .child(icon(
            "titlebar.maximize",
            "square",
            UiRect::new(max_r.left + 14.0, rect.top + 12.0, max_r.right - 14.0, rect.top + 28.0),
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
            UiRect::new(close_r.left + 14.0, rect.top + 12.0, close_r.right - 14.0, rect.top + 28.0),
            theme::ZINC_400,
        ));
    bar = bar.child(close_btn);

    bar
}
