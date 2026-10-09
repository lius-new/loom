//! Resize strips with hover cursors and highlights only while resizing.

use lgui::core::{CursorIcon, EventPolicy, UiElement, UiId, UiScale};
use lgui::prelude::{Element, UiRect, VisualStyle};

use crate::theme;

pub fn render(
    key: impl Into<String>,
    hit: UiRect,
    line: UiRect,
    cursor: CursorIcon,
    active: bool,
) -> Element {
    let key = key.into();
    let line_id = UiId::owned(format!("resize-line-{key}"));
    Element::new(move |cx| {
        let line = pixel_aligned_line(line, cursor, cx.context.scale());
        let color = if active {
            theme::c().accent
        } else {
            theme::c().border
        };
        let mut children = cx.children;
        children.push(UiElement::panel(line_id, line, VisualStyle::filled(color)));
        UiElement::panel(cx.id, hit, VisualStyle::default()).children(children)
    })
    .key(key)
    .event_policy(EventPolicy::INTERACTIVE)
    .cursor(cursor)
}

// Filled rectangles on half-pixel edges paint a translucent extra row/column.
// Snap only the painted divider, leaving its wider pointer hit strip unchanged.
fn pixel_aligned_line(line: UiRect, cursor: CursorIcon, scale: UiScale) -> UiRect {
    let factor = scale.factor();
    let snap = |position| scale.physical_value(position) as f32 / factor;
    let width = scale.physical_length(1.0) as f32 / factor;
    match cursor {
        CursorIcon::ResizeHorizontal => {
            let left = snap(line.left);
            UiRect::new(left, snap(line.top), left + width, snap(line.bottom))
        }
        CursorIcon::ResizeVertical => {
            let top = snap(line.top);
            UiRect::new(snap(line.left), top, snap(line.right), top + width)
        }
        _ => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgui::application::{AppView, ApplicationContext};
    use lgui::core::{
        InputEvent, Point, PointerButton, PointerData, UiScale, dispatch_runtime_output,
    };
    use lgui::prelude::panel;
    use lgui::session::UiSession;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn horizontal_resize_cursor_changes_on_hover_and_highlights_only_while_resizing() {
        hover_cursor_and_resize_highlight(CursorIcon::ResizeHorizontal);
    }

    #[test]
    fn vertical_resize_cursor_changes_on_hover_and_highlights_only_while_resizing() {
        hover_cursor_and_resize_highlight(CursorIcon::ResizeVertical);
    }

    #[test]
    fn resize_lines_are_crisp_at_fractional_positions_and_display_scales() {
        use lgui::core::PhysicalRect;
        use lgui::render_skia::SkiaSoftwareSurface;
        use lgui::renderer::{FrameInfo, FrameReason};

        let viewport = UiRect::new(0.0, 0.0, 128.0, 128.0);
        for factor in [1.0_f32, 1.25, 1.5, 2.0] {
            let scale = UiScale::new(factor);
            let size = (128.0 * factor).round() as usize;
            for cursor in [CursorIcon::ResizeHorizontal, CursorIcon::ResizeVertical] {
                for position in [50.0, 50.25, 50.75] {
                    for active in [false, true] {
                        let (hit, line) = if cursor == CursorIcon::ResizeHorizontal {
                            (
                                UiRect::new(position - 4.0, 0.0, position + 4.0, 128.0),
                                UiRect::new(position - 0.5, 0.0, position + 0.5, 128.0),
                            )
                        } else {
                            (
                                UiRect::new(0.0, position - 4.0, 128.0, position + 4.0),
                                UiRect::new(0.0, position - 0.5, 128.0, position + 0.5),
                            )
                        };
                        let view: AppView =
                            Arc::new(move |_| render("pixel-test", hit, line, cursor, active));
                        let mut session = UiSession::new();
                        session.render_view(&view, viewport, scale);
                        let frame = FrameInfo::new(
                            PhysicalRect::new(0, 0, size as i32, size as i32),
                            &[],
                            scale,
                            FrameReason::SceneChange,
                            true,
                        );
                        let mut surface = SkiaSoftwareSurface::new(1024 * 1024);
                        surface
                            .draw(&session.tree().scene().project_to_physical(scale), &frame)
                            .unwrap();
                        let sample = (16.0 * factor).round() as usize;
                        let alphas = (0..size)
                            .map(|offset| {
                                let (x, y) = if cursor == CursorIcon::ResizeHorizontal {
                                    (offset, sample)
                                } else {
                                    (sample, offset)
                                };
                                surface.pixels()[(y * size + x) * 4 + 3]
                            })
                            .filter(|alpha| *alpha > 0)
                            .collect::<Vec<_>>();
                        assert_eq!(
                            alphas.len(),
                            factor.round() as usize,
                            "scale={factor}, cursor={cursor:?}, position={position}, active={active}, alpha={alphas:?}"
                        );
                        assert!(
                            alphas.iter().all(|alpha| *alpha == 255),
                            "line edges must not produce a translucent halo: {alphas:?}"
                        );
                    }
                }
            }
        }
    }

    fn hover_cursor_and_resize_highlight(cursor: CursorIcon) {
        let viewport = UiRect::new(0.0, 0.0, 300.0, 300.0);
        let (hit, line, points) = if cursor == CursorIcon::ResizeHorizontal {
            (
                UiRect::new(96.0, 0.0, 104.0, 300.0),
                UiRect::new(99.5, 0.0, 100.5, 300.0),
                [
                    Point::new(96.5, 120.0),
                    Point::new(100.0, 120.0),
                    Point::new(103.5, 120.0),
                ],
            )
        } else {
            (
                UiRect::new(0.0, 96.0, 300.0, 104.0),
                UiRect::new(0.0, 99.5, 300.0, 100.5),
                [
                    Point::new(120.0, 96.5),
                    Point::new(120.0, 100.0),
                    Point::new(120.0, 103.5),
                ],
            )
        };
        let starts = Arc::new(AtomicUsize::new(0));
        let start_count = starts.clone();
        let view: AppView = Arc::new(move |cx| {
            let resizing = cx.state_with(|| false);
            let down = resizing.clone();
            let up = resizing.clone();
            let starts = start_count.clone();
            panel(viewport, VisualStyle::default())
                .cursor(CursorIcon::Text)
                .on_pointer_move(|_, _| {})
                .child(
                    render("hover-test", hit, line, cursor, resizing.get())
                        .on_pointer_down_with_button(move |_, _, button| {
                            if button == PointerButton::Left {
                                starts.fetch_add(1, Ordering::SeqCst);
                                down.update(|active| *active = true);
                            }
                        })
                        .on_pointer_drag(|_, _| {})
                        .on_pointer_up(move |_, _| up.update(|active| *active = false)),
                )
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let line_id = UiId::owned("resize-line-hover-test");
        assert_eq!(
            session.tree().node(&line_id).unwrap().style.fill,
            Some(theme::c().border)
        );
        let context = ApplicationContext::empty(Default::default());
        let send = |session: &mut UiSession, input| {
            dispatch_runtime_output(
                session.handle_input(input),
                &context,
                &lgui::window::WindowId::new("resize-hover-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
            session.render_view(&view, viewport, UiScale::ONE);
        };
        let outside = Point::new(200.0, 200.0);
        for point in points {
            assert_eq!(session.tree().cursor_at(point), Some(cursor));
            send(
                &mut session,
                InputEvent::PointerMove(PointerData::mouse(point)),
            );
            assert_eq!(
                session.tree().node(&line_id).unwrap().style.fill,
                Some(theme::c().border)
            );
            assert_eq!(starts.load(Ordering::SeqCst), 0);
            send(
                &mut session,
                InputEvent::PointerMove(PointerData::mouse(outside)),
            );
            assert_eq!(
                session.tree().node(&line_id).unwrap().style.fill,
                Some(theme::c().border)
            );
            assert_eq!(session.tree().cursor_at(outside), Some(CursorIcon::Text));
        }
        let point = points[1];
        // A right press is not a resize operation and must not highlight the line.
        send(
            &mut session,
            InputEvent::PointerDown {
                pointer: PointerData::mouse(point),
                button: PointerButton::Right,
            },
        );
        assert_eq!(
            session.tree().node(&line_id).unwrap().style.fill,
            Some(theme::c().border)
        );
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        send(
            &mut session,
            InputEvent::PointerUp {
                pointer: PointerData::mouse(point),
                button: PointerButton::Right,
            },
        );
        let handle_id = session.tree().hit_test(point).unwrap().id;
        send(
            &mut session,
            InputEvent::PointerDown {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            },
        );
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(
            session.tree().node(&line_id).unwrap().style.fill,
            Some(theme::c().accent)
        );
        send(
            &mut session,
            InputEvent::PointerMove(PointerData::mouse(outside)),
        );
        assert_eq!(
            session.tree().node(&line_id).unwrap().style.fill,
            Some(theme::c().accent)
        );
        assert_eq!(session.tree().cursor_at_id(&handle_id), Some(cursor));
        send(
            &mut session,
            InputEvent::PointerUp {
                pointer: PointerData::mouse(outside),
                button: PointerButton::Left,
            },
        );
        assert_eq!(
            session.tree().node(&line_id).unwrap().style.fill,
            Some(theme::c().border)
        );
    }
}
