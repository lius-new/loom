//! Compact text actions shared by Git section headers and branch rows.
use crate::ui::theme;
use lgui::core::{
    CursorIcon, EventPolicy, PointerButton, SemanticRole, Semantics, UiElement, UiEventContext,
};
use lgui::prelude::{Element, TextAlign, UiRect, VisualStyle, text};

pub(crate) fn width(label: &str) -> f32 {
    crate::ui::tabs::measure(label, theme::SMALL, 400) + 16.0
}

pub(crate) fn render(
    rect: UiRect,
    key: String,
    label: &'static str,
    name: String,
    on_click: impl Fn() + Send + Sync + 'static,
) -> Element {
    let mut style = theme::sans(theme::c().text_soft, theme::SMALL);
    style.align = TextAlign::Center;
    Element::new(move |cx| {
        let hovered = cx.context.interaction_flags(&cx.id).hovered;
        UiElement::panel(
            cx.id,
            rect,
            VisualStyle::filled(if hovered {
                theme::c().surface
            } else {
                theme::c().active_line
            })
            .radius(3.0),
        )
        .children(cx.children)
    })
    .key(key)
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(Semantics::new(SemanticRole::Button).name(name))
    .cursor(CursorIcon::Pointer)
    .on_pointer_down_with_button(|cx, _, button| {
        if button == PointerButton::Left {
            cx.stop_propagation();
        }
    })
    .on_click(move |cx: &mut UiEventContext| {
        cx.stop_propagation();
        on_click();
    })
    .child(text(rect, label, style))
}
