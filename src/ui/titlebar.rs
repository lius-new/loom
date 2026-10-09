//! Custom frameless title bar with native window controls on the far right.
//!
//! The bar is marked as a native window drag region; interactive children
//! (the buttons) are excluded automatically by hit testing.

use lgui::core::{
    AnimProperty, AnimationBinding, Color, CursorIcon, EventPolicy, IconStyle, PhysicalRect,
    UiElement, UiEventContext, UiId, UiScale,
};
use lgui::prelude::{Element, State, UiRect, VisualStyle, panel};

use crate::input::action::Action;
use crate::state::AppState;
use crate::theme;
use crate::window_geometry;

/// Windows 11 caption close-button red, the same in light and dark themes.
const CLOSE_HOVER: Color = Color(0xC42B1C);

#[derive(Clone, Copy)]
enum Glyph {
    /// Pixel-snapped 12px window-control silhouette.
    Window,
    /// Regular UI icon at `theme::ICON_SIZE`.
    Ui,
}

#[derive(Clone, Copy)]
struct CaptionIcon {
    id: &'static str,
    key: &'static str,
    color: Color,
    glyph: Glyph,
}

/// A flat title bar button that fades in a hover background and dims while
/// pressed. `close` uses the red close-button treatment.
fn caption_button(
    key: &'static str,
    rect: UiRect,
    icon: CaptionIcon,
    close: bool,
    on_click: impl Fn(&mut UiEventContext) + Send + Sync + 'static,
) -> Element {
    Element::new(move |cx| {
        let hover = cx.animation_value(AnimProperty::Hover).clamp(0.0, 1.0);
        let pressed = cx.animation_value(AnimProperty::Pressed).clamp(0.0, 1.0);
        let c = theme::c();
        let (fill, fill_alpha, icon_color) = if close {
            (
                CLOSE_HOVER,
                hover * (1.0 - 0.1 * pressed),
                mix(icon.color, Color::WHITE, hover),
            )
        } else {
            (c.active_line, hover * (1.0 - 0.4 * pressed), icon.color)
        };
        let icon_rect = match icon.glyph {
            Glyph::Window => window_icon_rect(rect, cx.context.scale()),
            Glyph::Ui => theme::icon_rect(rect),
        };
        let style = VisualStyle {
            fill: Some(fill),
            fill_alpha: (fill_alpha * 255.0).round() as u8,
            ..VisualStyle::default()
        };
        let icon_alpha = (255.0 * (1.0 - 0.3 * pressed)).round() as u8;
        UiElement::panel(cx.id, rect, style).child(
            UiElement::icon(UiId::new(icon.id), icon_rect, icon.key)
                .icon_style(IconStyle::new(icon_color).alpha(icon_alpha)),
        )
    })
    .key(key)
    .event_policy(EventPolicy::INTERACTIVE)
    .animation(AnimationBinding::new(AnimProperty::Hover, 0.0, 1.0))
    .animation(AnimationBinding::new(AnimProperty::Pressed, 0.0, 1.0))
    .on_click(on_click)
}

fn mix(from: Color, to: Color, t: f32) -> Color {
    let channel = |shift: u32| {
        let a = ((from.0 >> shift) & 0xFF) as f32;
        let b = ((to.0 >> shift) & 0xFF) as f32;
        ((a + (b - a) * t).round() as u32) << shift
    };
    Color(channel(16) | channel(8) | channel(0))
}

/// Keep the SVG bitmap and its destination on the same physical pixel grid.
/// Auto scaling can shrink the UI below 100%; caption glyphs still need at least
/// their native 12px canvas, or a 1px edge becomes a faint fractional pixel.
fn window_icon_rect(button: UiRect, scale: UiScale) -> UiRect {
    let size = scale.physical_length(12.0).max(12);
    let left = scale.physical_value((button.left + button.right) / 2.0) - size / 2;
    let top = scale.physical_value((button.top + button.bottom) / 2.0) - size / 2;
    scale.logical_rect(PhysicalRect::new(left, top, left + size, top + size))
}

pub fn render(rect: UiRect, state: State<AppState>) -> Element {
    let mut bar = panel(rect, VisualStyle::filled(theme::c().sidebar)).window_drag_region();

    // ---- Right cluster (anchored to the window controls) --------------
    // Flush with the window edge, like native caption buttons, so the close
    // button's hover fill reaches the corner.
    let right = rect.right;

    // Settings, just left of the window controls.
    let settings_r = UiRect::new(right - 148.0, rect.top, right - 116.0, rect.bottom);
    let settings_open = state.get().workspace.active_is_settings();
    bar = bar.child(
        caption_button(
            "titlebar.settings.button",
            settings_r,
            CaptionIcon {
                id: "titlebar.settings",
                key: "settings",
                color: if settings_open {
                    theme::c().accent
                } else {
                    theme::c().text_muted
                },
                glyph: Glyph::Ui,
            },
            false,
            move |_: &mut UiEventContext| state.update(|app| app.apply(Action::OpenSettings)),
        )
        .cursor(CursorIcon::Pointer),
    );

    // ---- Window controls (far right) ----------------------------------
    let min_r = UiRect::new(right - 108.0, rect.top, right - 72.0, rect.bottom);
    let max_r = UiRect::new(right - 72.0, rect.top, right - 36.0, rect.bottom);
    let close_r = UiRect::new(right - 36.0, rect.top, right, rect.bottom);
    let window_icon = |id, key| CaptionIcon {
        id,
        key,
        color: theme::c().text_soft,
        glyph: Glyph::Window,
    };

    bar = bar.child(caption_button(
        "titlebar.minimize.button",
        min_r,
        window_icon("titlebar.minimize", "window-minimize"),
        false,
        |ctx: &mut UiEventContext| {
            let _ = ctx.window().minimize();
        },
    ));

    // Maximize / restore. On Windows 11 the OS owns this button so hovering it
    // shows Snap Layouts; lgui replays its hover, press and click here.
    let max_key = if window_geometry::is_maximized() {
        "window-restore"
    } else {
        "window-maximize"
    };
    bar = bar.child(
        caption_button(
            "titlebar.maximize.button",
            max_r,
            window_icon("titlebar.maximize", max_key),
            false,
            |ctx: &mut UiEventContext| {
                let _ = ctx.window().toggle_maximize();
            },
        )
        .window_maximize_button(),
    );

    bar = bar.child(caption_button(
        "titlebar.close.button",
        close_r,
        window_icon("titlebar.close", "window-close"),
        true,
        |ctx: &mut UiEventContext| {
            let _ = ctx.window().request_close();
        },
    ));

    // Hairline bottom border
    bar = bar.child(panel(
        UiRect::new(rect.left, rect.bottom - 1.0, rect.right, rect.bottom),
        VisualStyle::filled(theme::c().border),
    ));

    bar
}
