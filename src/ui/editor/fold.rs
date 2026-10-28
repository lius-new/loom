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
        EventPolicy, PathStyle, Point, SemanticRole, Semantics, Stroke, UiFocusHandle, UiPath,
        UiPathCommand, path,
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
        [(x - 1.25, y - 2.5), (x + 1.25, y), (x - 1.25, y + 2.5)]
    } else {
        [(x - 2.5, y - 1.25), (x, y + 1.25), (x + 2.5, y - 1.25)]
    };
    path(
        rect,
        UiPath::new(points.into_iter().enumerate().map(|(i, (x, y))| {
            if i == 0 {
                UiPathCommand::MoveTo(Point::new(x, y))
            } else {
                UiPathCommand::LineTo(Point::new(x, y))
            }
        })),
        PathStyle {
            fill: None,
            fill_alpha: 0,
            stroke: Some(Stroke::new(theme::c().text_dim, 1.0, 255)),
        },
    )
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
    let (kind, glyph) = match content {
        Content::Chevron { collapsed } => ("gutter", chevron(rect, collapsed)),
        Content::Placeholder(label) => (
            "inline",
            text(rect, label, fonts::editor().style(theme::c().text_dim)).into(),
        ),
    };
    panel(rect, VisualStyle::default())
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
        .child(glyph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgui::{
        application::AppView,
        core::{PhysicalRect, UiScale},
        render_skia::SkiaSoftwareSurface,
        renderer::{FrameInfo, FrameReason},
        session::UiSession,
    };
    use std::sync::Arc;

    #[test]
    fn chevrons_have_transparent_background_and_stay_centered_at_fractional_scale() {
        let viewport = UiRect::new(0.0, 0.0, 32.0, 32.0);
        let rect = UiRect::new(8.0, 4.0, 14.0, 28.0);
        for collapsed in [false, true] {
            let view: AppView = Arc::new(move |_| chevron(rect, collapsed));
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
                    count > 0 && count < 40 * factor as i32 * factor as i32,
                    "only the arrow is painted: {count}"
                );
                assert!(
                    ((bounds.0 + bounds.2 + 1) as f32 / 2.0 - 11.0 * factor).abs() <= 1.0,
                    "horizontal centering: {bounds:?}"
                );
                assert!(
                    ((bounds.1 + bounds.3 + 1) as f32 / 2.0 - 16.0 * factor).abs() <= 1.0,
                    "vertical centering: {bounds:?}"
                );
                assert!(
                    bounds.1 > (rect.top * factor) as i32
                        && bounds.3 < (rect.bottom * factor) as i32 - 1,
                    "no background block"
                );
            }
        }
    }
}
