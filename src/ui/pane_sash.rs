//! Draggable dividers between editor panes.

use lgui::core::{CursorIcon, PointerButton};
use lgui::prelude::{Element, State, UiRect};

use crate::ui::components::resize_handle;

use crate::model::pane_layout::{Axis, MinSize, Sash};
use crate::state::AppState;

/// Smallest pane a sash drag may produce.
pub const PANE_MIN: MinSize = MinSize {
    width: 200.0,
    height: 120.0,
};
/// Width of the invisible hit strip centred on the 1px divider.
const HIT: f32 = 4.0;

pub fn split_position(left: f32, right: f32, position: f32) -> f32 {
    let minimum = PANE_MIN.width.min((right - left).max(0.0) / 2.0);
    position.clamp(left + minimum, right - minimum)
}

/// Shared pointer capture and feedback for horizontal content splits.
pub fn horizontal(
    key: String,
    position: f32,
    span: (f32, f32),
    active: bool,
    start: impl Fn() + Send + Sync + 'static,
    drag: impl Fn(f32) + Send + Sync + 'static,
    end: impl Fn() + Send + Sync + 'static,
) -> Element {
    resize_handle::render(
        key,
        UiRect::new(position - HIT / 2.0, span.0, position + HIT / 2.0, span.1),
        UiRect::new(position - 0.5, span.0, position + 0.5, span.1),
        CursorIcon::ResizeHorizontal,
        active,
    )
    .on_pointer_down_with_button(move |cx, _, button| {
        if button == PointerButton::Left {
            start();
            cx.stop_propagation();
        }
    })
    .on_pointer_drag(move |cx, pointer| {
        drag(pointer.point.x);
        cx.stop_propagation();
    })
    .on_pointer_up(move |cx, _| {
        end();
        cx.stop_propagation();
    })
}

pub fn render(sash: Sash, state: State<AppState>) -> Element {
    let s = state.get();
    let active = s
        .sash_drag
        .as_ref()
        .is_some_and(|drag| same_sash(drag, &sash));
    let (line, hit, cursor) = match sash.axis {
        Axis::Horizontal => (
            UiRect::new(
                sash.position - 0.5,
                sash.span.0,
                sash.position + 0.5,
                sash.span.1,
            ),
            UiRect::new(
                sash.position - HIT / 2.0,
                sash.span.0,
                sash.position + HIT / 2.0,
                sash.span.1,
            ),
            CursorIcon::ResizeHorizontal,
        ),
        Axis::Vertical => (
            UiRect::new(
                sash.span.0,
                sash.position - 0.5,
                sash.span.1,
                sash.position + 0.5,
            ),
            UiRect::new(
                sash.span.0,
                sash.position - HIT / 2.0,
                sash.span.1,
                sash.position + HIT / 2.0,
            ),
            CursorIcon::ResizeVertical,
        ),
    };
    let key = format!(
        "pane-sash-{}-{}",
        sash.path
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join("."),
        sash.index
    );
    let st_down = state.clone();
    let st_move = state.clone();
    let st_up = state;
    let pressed = sash.clone();
    resize_handle::render(key, hit, line, cursor, active)
        .on_pointer_down_with_button(move |cx, _pointer, button| {
            if button == PointerButton::Left {
                let sash = pressed.clone();
                st_down.update(move |app| app.sash_drag = Some(sash));
                cx.stop_propagation();
            }
        })
        .on_pointer_drag(move |_cx, pointer| {
            st_move.try_update(move |app| {
                let Some(drag) = app.sash_drag.clone() else {
                    return false;
                };
                let position = match drag.axis {
                    Axis::Horizontal => pointer.point.x,
                    Axis::Vertical => pointer.point.y,
                };
                app.workspace.layout_mut().resize(&drag, position, PANE_MIN)
            });
        })
        .on_pointer_up(move |_cx, _pointer| {
            st_up.update(|app| app.sash_drag = None);
        })
}

/// The same boundary of the same split, regardless of where it currently is.
fn same_sash(a: &Sash, b: &Sash) -> bool {
    a.path == b.path && a.index == b.index && a.axis == b.axis
}
