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
        ComponentState, EventPolicy, PathStyle, Point, SemanticRole, Semantics, Stroke, UiElement,
        UiFocusHandle, UiId, UiPath, UiPathCommand, UiRenderContext,
    },
    prelude::{Element, UiRect, VisualStyle, panel, text},
};
use std::sync::Arc;

pub enum Content {
    Chevron {
        collapsed: bool,
        column: Column,
        start: usize,
    },
    Placeholder(String),
}

/// Immutable control identities keep gutter hover local to one editor view.
#[derive(Clone)]
pub struct Column {
    id: UiId,
    view: ViewId,
    buttons: Arc<[UiId]>,
}

// A stationary pointer can still name a button scrolled out of the viewport.
// Retain only that one hover identity until input moves it or leaves the window.
#[derive(Clone, Default)]
struct ColumnHover {
    target: Option<UiId>,
}
impl ComponentState for ColumnHover {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl Column {
    pub fn new(view: ViewId, starts: impl IntoIterator<Item = usize>) -> Self {
        let mut buttons: Vec<_> = starts
            .into_iter()
            .map(|start| Self::id_for(view, start))
            .collect();
        buttons.push(UiId::owned(format!("editor-code-actions-{view:?}")));
        buttons.push(UiId::owned(format!("editor-gutter-change-{view:?}")));
        Self {
            id: UiId::owned(format!("editor-fold-column-{view:?}")),
            view,
            buttons: buttons.into(),
        }
    }

    pub fn actions_id(&self) -> UiId {
        UiId::owned(format!("editor-code-actions-{:?}", self.view))
    }

    pub fn change_id(&self) -> UiId {
        UiId::owned(format!("editor-gutter-change-{:?}", self.view))
    }

    fn id_for(view: ViewId, start: usize) -> UiId {
        UiId::owned(format!("editor-fold-button-{view:?}-{start}"))
    }

    fn button_id(&self, start: usize) -> UiId {
        Self::id_for(self.view, start)
    }

    fn hovered(&self, context: &UiRenderContext<'_>) -> bool {
        context.component_state_mut::<ColumnHover, _>(&self.id, |hover| {
            if let Some(id) = std::iter::once(&self.id)
                .chain(self.buttons.iter())
                .find(|id| context.interaction_flags(id).hovered)
            {
                hover.target = Some(id.clone());
                true
            } else if hover
                .target
                .as_ref()
                .is_some_and(|id| context.interaction_flags(id).hovered)
            {
                true
            } else {
                hover.target = None;
                false
            }
        })
    }

    pub fn render(&self, rect: UiRect, fold_left: f32) -> Element {
        let column = self.clone();
        Element::new(move |cx| {
            let mut children = cx.children;
            if column.hovered(cx.context) {
                let target = cx
                    .context
                    .component_state_mut::<ColumnHover, _>(&column.id, |hover| {
                        hover.target.clone()
                    });
                if let Some(target) = target
                    && target != column.id
                    && !contains_control(&children, &target)
                {
                    // Virtualization can remove the hovered button before the next
                    // pointer event. Retain its identity outside the clip until input
                    // updates hover, without keeping its glyph or click handler alive.
                    children.push(
                        UiElement::panel(
                            target,
                            UiRect::new(-1.0, -1.0, -1.0, -1.0),
                            VisualStyle::default(),
                        )
                        .event_policy(EventPolicy {
                            hover: true,
                            press: false,
                            focus: false,
                        }),
                    );
                }
            }
            UiElement::panel(column.id.clone(), rect, VisualStyle::default()).children(children)
        })
        .key(self.id.as_str())
        .event_policy(EventPolicy {
            focus: false,
            ..EventPolicy::INTERACTIVE
        })
        .semantics(Semantics::new(SemanticRole::Group).name("Editor gutter"))
        // All of the gutter reveals the arrows. Keep ordinary line-number
        // clicks routed to the editor, but do not move the caret in the fold lane.
        .on_pointer_move(|_, _| {})
        .on_pointer_down(move |cx, pointer| {
            if pointer.point.x >= fold_left {
                cx.stop_propagation();
            }
        })
    }
}

fn contains_control(children: &[UiElement], id: &UiId) -> bool {
    children
        .iter()
        .any(|child| &child.node().id == id || contains_control(child.children_ref(), id))
}

fn chevron(rect: UiRect, collapsed: bool, column: Column, start: usize) -> Element {
    let x = (rect.left + rect.right) / 2.0;
    let y = (rect.top + rect.bottom) / 2.0;
    let points = if collapsed {
        [(x - 3.0, y - 4.0), (x + 3.0, y), (x - 3.0, y + 4.0)]
    } else {
        [(x - 4.0, y - 2.5), (x, y + 2.5), (x + 4.0, y - 2.5)]
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
        let visible = column.hovered(cx.context);
        UiElement::path(
            column.button_id(start),
            rect,
            arrow.clone(),
            PathStyle {
                fill: None,
                fill_alpha: 0,
                stroke: visible.then(|| Stroke::new(theme::c().text_dim, 1.5, 255)),
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
        Content::Chevron {
            collapsed,
            column,
            start,
        } => ("gutter", chevron(rect, collapsed, column, start)),
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

    #[test]
    fn larger_chevrons_show_only_on_hover_without_flicker_or_background_at_fractional_scale() {
        let viewport = UiRect::new(0.0, 0.0, 32.0, 32.0);
        let rect = UiRect::new(6.0, 4.0, 16.0, 28.0);
        for collapsed in [false, true] {
            let view: AppView = Arc::new(move |_| {
                chevron(rect, collapsed, Column::new(ViewId::new(1), [0]), 0)
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
                        ((bounds.0 + bounds.2 + 1) as f32 / 2.0 - 11.0 * factor).abs() <= 1.0,
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
    #[test]
    fn column_hover_reveals_all_its_arrows_and_keeps_other_views_hidden() {
        let viewport = UiRect::new(0.0, 0.0, 80.0, 80.0);
        let scrolled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let scroll = scrolled.clone();
        let view: AppView = Arc::new(move |_| {
            let mut root = lgui::prelude::group(viewport);
            for (view, x) in [(ViewId::new(1), 6.0), (ViewId::new(2), 30.0)] {
                let starts = if view == ViewId::new(1)
                    && scroll.load(std::sync::atomic::Ordering::Relaxed)
                {
                    [2, 3]
                } else {
                    [0, 1]
                };
                let column = Column::new(view, starts);
                let mut controls = column.render(UiRect::new(x, 0.0, x + 10.0, 80.0), x);
                for (start, y) in starts.into_iter().zip([0.0, 48.0]) {
                    controls = controls.child(
                        chevron(
                            UiRect::new(x, y, x + 10.0, y + 24.0),
                            false,
                            column.clone(),
                            start,
                        )
                        .event_policy(EventPolicy::INTERACTIVE)
                        .cursor(CursorIcon::Pointer)
                        .on_click(|| {}),
                    );
                }
                root = root.child(controls);
            }
            root
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let check = |session: &UiSession, first, second| {
            for (view, visible) in [(ViewId::new(1), first), (ViewId::new(2), second)] {
                let starts = if view == ViewId::new(1)
                    && scrolled.load(std::sync::atomic::Ordering::Relaxed)
                {
                    [2, 3]
                } else {
                    [0, 1]
                };
                for start in starts {
                    assert_eq!(
                        session
                            .tree()
                            .node(&Column::id_for(view, start))
                            .unwrap()
                            .path_style
                            .stroke
                            .is_some(),
                        visible,
                        "view={view:?}, start={start}, scrolled={}",
                        scrolled.load(std::sync::atomic::Ordering::Relaxed)
                    );
                }
            }
        };
        check(&session, false, false);
        for (point, first, second, cursor) in [
            (Point::new(11.0, 36.0), true, false, CursorIcon::Default),
            (Point::new(11.0, 12.0), true, false, CursorIcon::Pointer),
            (Point::new(11.0, 60.0), true, false, CursorIcon::Pointer),
            (Point::new(35.0, 36.0), false, true, CursorIcon::Default),
            (Point::new(60.0, 36.0), false, false, CursorIcon::Default),
        ] {
            session.handle_input(InputEvent::PointerMove(PointerData::mouse(point)));
            for _ in 0..3 {
                session.render_view(&view, viewport, UiScale::ONE);
            }
            check(&session, first, second);
            assert_eq!(
                session
                    .tree()
                    .cursor_at(point)
                    .unwrap_or(CursorIcon::Default),
                cursor
            );
        }
        session.handle_input(InputEvent::PointerMove(PointerData::mouse(Point::new(
            11.0, 12.0,
        ))));
        session.render_view(&view, viewport, UiScale::ONE);
        check(&session, true, false);
        scrolled.store(true, std::sync::atomic::Ordering::Relaxed);
        for _ in 0..3 {
            session.render_view(&view, viewport, UiScale::ONE);
        }
        check(&session, true, false);
        session.handle_input(InputEvent::PointerMove(PointerData::mouse(Point::new(
            35.0, 12.0,
        ))));
        session.render_view(&view, viewport, UiScale::ONE);
        check(&session, false, true);
        session.handle_input(InputEvent::PointerLeave(PointerData::mouse(Point::new(
            35.0, 12.0,
        ))));
        session.render_view(&view, viewport, UiScale::ONE);
        check(&session, false, false);
    }
}
