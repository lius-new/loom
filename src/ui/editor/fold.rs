//! Fold controls keep source focus and send versioned, value-only intents.
use crate::{
    domain::{document::DocumentStamp, identity::ViewId},
    ui::{
        editor_binding::{Binding, Event},
        fonts, theme,
    },
};
use lgui::{
    core::{
        EventPolicy, PathStyle, Point, SemanticRole, Semantics, Stroke, UiElement, UiFocusHandle,
        UiPath, UiPathCommand,
    },
    prelude::{Element, UiRect, VisualStyle, panel, text},
};

pub enum Content {
    Chevron { collapsed: bool },
    Placeholder(String),
}

fn chevron(rect: UiRect, collapsed: bool) -> Element {
    let x = (rect.left + rect.right) / 2.0;
    let y = (rect.top + rect.bottom) / 2.0;
    let points = if collapsed {
        [(x - 1.625, y - 4.5), (x + 1.625, y), (x - 1.625, y + 4.5)]
    } else {
        [(x - 3.25, y - 2.25), (x, y + 2.25), (x + 3.25, y - 2.25)]
    };
    let arrow = UiPath::new(points.into_iter().enumerate().map(|(i, (x, y))| {
        if i == 0 {
            UiPathCommand::MoveTo(Point::new(x, y))
        } else {
            UiPathCommand::LineTo(Point::new(x, y))
        }
    }));
    // The target keeps its identity and bounds when the arrow appears. Drawing
    // on the target itself avoids hover flicker from inserting a hit-test child.
    Element::new(move |cx| {
        let flags = cx.context.interaction_flags(&cx.id);
        UiElement::path(
            cx.id,
            rect,
            arrow.clone(),
            PathStyle {
                fill: None,
                fill_alpha: 0,
                stroke: (flags.hovered || flags.pressed)
                    .then(|| Stroke::new(theme::c().text_soft, 1.5, 255)),
            },
        )
        .children(cx.children)
    })
}

#[allow(clippy::too_many_arguments)]
pub fn button(
    rect: UiRect,
    label: String,
    content: Content,
    state: Binding,
    focus: UiFocusHandle,
    view: ViewId,
    stamp: DocumentStamp,
    start: usize,
) -> Element {
    let (kind, control) = match content {
        Content::Chevron { collapsed } => ("gutter", chevron(rect, collapsed)),
        Content::Placeholder(label) => (
            "inline",
            panel(rect, VisualStyle::default()).child(text(
                rect,
                label,
                fonts::editor().style(theme::c().text_dim),
            )),
        ),
    };
    control
        .key(format!("fold-{view:?}-{start}-{kind}"))
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgui::{
        application::AppView,
        core::{CursorIcon, InputEvent, PhysicalRect, PointerData, UiScale},
        render_skia::SkiaSoftwareSurface,
        renderer::{FrameInfo, FrameReason},
        session::UiSession,
    };
    use std::sync::Arc;

    #[test]
    fn larger_chevrons_show_only_on_hover_without_flicker_or_background_at_fractional_scale() {
        let viewport = UiRect::new(0.0, 0.0, 32.0, 32.0);
        let rect = UiRect::new(6.0, 4.0, 14.0, 28.0);
        for collapsed in [false, true] {
            let view: AppView = Arc::new(move |_| {
                chevron(rect, collapsed)
                    .key("hover-fold")
                    .on_click(|| {})
                    .event_policy(EventPolicy::INTERACTIVE)
                    .cursor(CursorIcon::Pointer)
            });
            for factor in [1.0, 1.25, 1.5, 2.0] {
                let scale = UiScale::new(factor);
                let mut session = UiSession::new();
                session.render_view(&view, viewport, scale);
                let edge = (32.0 * factor) as i32;
                let frame = FrameInfo::new(
                    PhysicalRect::new(0, 0, edge, edge),
                    &[],
                    scale,
                    FrameReason::SceneChange,
                    true,
                );
                let mut surface = SkiaSoftwareSurface::new(1024 * 1024);
                surface
                    .draw(&session.tree().scene().project_to_physical(scale), &frame)
                    .unwrap();
                assert!(
                    surface
                        .pixels()
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .all(|p| p[3] == 0),
                    "hidden before hover"
                );
                let target = session.tree().nodes()[0].id.clone();
                for point in [
                    Point::new(10.0, 16.0),
                    Point::new(7.0, 10.0),
                    Point::new(13.0, 22.0),
                ] {
                    session.handle_input(InputEvent::PointerMove(PointerData::mouse(point)));
                    // Repeated native redraws must preserve the hover target.
                    for _ in 0..3 {
                        session.render_view(&view, viewport, scale);
                    }
                    assert_eq!(session.tree().nodes()[0].id, target);
                    assert_eq!(session.tree().cursor_at(point), Some(CursorIcon::Pointer));
                    surface
                        .draw(&session.tree().scene().project_to_physical(scale), &frame)
                        .unwrap();
                    let mut bounds = (edge, edge, -1, -1);
                    let mut count = 0;
                    for y in 0..edge {
                        for x in 0..edge {
                            if surface.pixels()[((y * edge + x) * 4 + 3) as usize] == 0 {
                                continue;
                            }
                            count += 1;
                            bounds = (
                                bounds.0.min(x),
                                bounds.1.min(y),
                                bounds.2.max(x),
                                bounds.3.max(y),
                            );
                        }
                    }
                    assert!(
                        count > 0 && (count as f32) < 60.0 * factor * factor,
                        "only the arrow is painted: count={count}, bounds={bounds:?}, scale={factor}, collapsed={collapsed}, point={point:?}"
                    );
                    assert!(
                        ((bounds.0 + bounds.2 + 1) as f32 / 2.0 - 10.0 * factor).abs() <= 1.0,
                        "horizontal centering: {bounds:?}"
                    );
                    assert!(
                        ((bounds.1 + bounds.3 + 1) as f32 / 2.0 - 16.0 * factor).abs() <= 1.0,
                        "vertical centering: {bounds:?}"
                    );
                    let major = if collapsed {
                        bounds.3 - bounds.1 + 1
                    } else {
                        bounds.2 - bounds.0 + 1
                    };
                    assert!(
                        (major as f32) >= 7.0 * factor,
                        "larger than the previous five-pixel arrow"
                    );
                    assert!(
                        bounds.1 > (rect.top * factor) as i32
                            && bounds.3 < (rect.bottom * factor) as i32 - 1,
                        "no background block"
                    );
                }
                for outside in [Point::new(20.0, 16.0), Point::new(3.0, 16.0)] {
                    session.handle_input(InputEvent::PointerMove(PointerData::mouse(outside)));
                    session.render_view(&view, viewport, scale);
                    surface
                        .draw(&session.tree().scene().project_to_physical(scale), &frame)
                        .unwrap();
                    assert!(
                        surface
                            .pixels()
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .all(|p| p[3] == 0),
                        "hidden after leaving"
                    );
                }
            }
        }
    }
}
