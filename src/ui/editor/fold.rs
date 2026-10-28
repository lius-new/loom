//! Fold controls keep source focus and send versioned, value-only intents.
use crate::{
    domain::{document::DocumentStamp, identity::ViewId},
    ui::{
        editor_binding::{Binding, Event},
        fonts, theme,
    },
};
use lgui::{
    core::{EventPolicy, SemanticRole, Semantics, UiFocusHandle},
    prelude::{Element, UiRect, VisualStyle, panel, text},
};
#[allow(clippy::too_many_arguments)]
pub fn button(
    rect: UiRect,
    label: String,
    glyph: String,
    state: Binding,
    focus: UiFocusHandle,
    view: ViewId,
    stamp: DocumentStamp,
    start: usize,
) -> Element {
    panel(rect, VisualStyle::filled(theme::c().active_line))
        .event_policy(EventPolicy {
            focus: false,
            ..EventPolicy::INTERACTIVE
        })
        .cursor(lgui::core::CursorIcon::Pointer)
        .semantics(Semantics::new(SemanticRole::Button).name(label))
        .on_pointer_down(|cx, _| cx.stop_propagation())
        .on_click(move || {
            focus.focus();
            state.send(Event::Fold { view, stamp, start });
        })
        .child(text(
            rect,
            glyph,
            fonts::editor().style(theme::c().text_dim),
        ))
}
