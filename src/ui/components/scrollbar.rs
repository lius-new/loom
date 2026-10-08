//! Shared overlay scrollbar colors and dimensions for editors and drawers.

use lgui::core::{CursorIcon, UiElement};
use lgui::prelude::{Element, UiRect, VisualStyle};

use crate::theme;

pub const SIZE: f32 = 12.0;
pub const INSET: f32 = 2.0;
pub const MIN_THUMB_LENGTH: f32 = 28.0;

pub fn track(rect: UiRect) -> Element {
    Element::new(move |cx| {
        let flags = cx.context.interaction_flags(&cx.id);
        let style = if flags.hovered || flags.pressed {
            VisualStyle::filled(theme::c().text_dim).alpha(18)
        } else {
            VisualStyle::default()
        };
        UiElement::panel(cx.id, rect, style).children(cx.children)
    })
    .cursor(CursorIcon::Default)
}

pub fn thumb(rect: UiRect, alpha: u8) -> Element {
    Element::new(move |cx| {
        let flags = cx.context.interaction_flags(&cx.id);
        let style = if flags.hovered || flags.pressed {
            VisualStyle::filled(theme::c().text_soft).radius(2.0)
        } else {
            VisualStyle::filled(theme::c().text_dim)
                .alpha(alpha)
                .radius(2.0)
        };
        UiElement::panel(cx.id, rect, style).children(cx.children)
    })
    .cursor(CursorIcon::Default)
}
