//! A vector bulb in its own gutter lane, independent of font emoji support.
use crate::ui::theme;
use lgui::core::{
    CursorIcon, EventPolicy, PathStyle, Point, SemanticRole, Semantics, Stroke, UiElement, UiId,
    UiPath, UiPathCommand, ellipse, path,
};
use lgui::prelude::{Element, UiRect, VisualStyle};

pub(crate) fn render(
    rect: UiRect,
    id: UiId,
    activate: impl Fn() + Send + Sync + 'static,
) -> Element {
    let x = (rect.left + rect.right) / 2.0;
    let y = (rect.top + rect.bottom) / 2.0;
    let colour = theme::c().warning;
    let head = UiRect::new(x - 4.0, y - 6.0, x + 4.0, y + 2.0);
    let lines = [(-2.5, 2.0), (-2.5, 4.0), (2.5, 4.0), (2.5, 2.0)];
    let commands: Vec<_> = lines
        .into_iter()
        .enumerate()
        .map(|(i, (dx, dy))| {
            let p = Point::new(x + dx, y + dy);
            if i == 0 {
                UiPathCommand::MoveTo(p)
            } else {
                UiPathCommand::LineTo(p)
            }
        })
        .chain([
            UiPathCommand::MoveTo(Point::new(x - 1.5, y + 6.0)),
            UiPathCommand::LineTo(Point::new(x + 1.5, y + 6.0)),
        ])
        .collect();
    Element::new(move |cx| {
        UiElement::panel(id.clone(), rect, VisualStyle::default()).children(cx.children)
    })
    .key("language-code-actions")
    .event_policy(EventPolicy {
        focus: false,
        ..EventPolicy::INTERACTIVE
    })
    .cursor(CursorIcon::Pointer)
    .semantics(Semantics::new(SemanticRole::Button).name("Code actions (Ctrl+.)"))
    .on_pointer_down_with_button(|cx, _, _| cx.stop_propagation())
    .on_click(move |cx: &mut lgui::core::UiEventContext| {
        cx.stop_propagation();
        activate();
    })
    .child(ellipse(head, VisualStyle::filled(colour)))
    .child(path(
        rect,
        UiPath::new(commands),
        PathStyle {
            fill: None,
            fill_alpha: 0,
            stroke: Some(Stroke::new(colour, 1.0, 255)),
        },
    ))
}
