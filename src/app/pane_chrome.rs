//! Pane divider state and domain resize coordination.
use crate::app::state::AppState;
use crate::domain::pane_layout::{Axis, Sash};
use crate::ui::chrome::sash_binding::{Binding, Command};
use lgui::prelude::{Element, State};
pub fn binding(state: &State<AppState>) -> Binding {
    let reader = state.clone();
    let writer = state.clone();
    Binding::new(
        move |sash| {
            reader
                .get()
                .editor
                .sash_drag
                .as_ref()
                .is_some_and(|drag| crate::ui::pane_sash::same_sash(drag, sash))
        },
        move |command| match command {
            Command::Start(sash) => writer.update(|app| app.editor.sash_drag = Some(sash)),
            Command::Drag(x, y) => {
                writer.try_update(|app| {
                    let Some(drag) = app.editor.sash_drag.clone() else {
                        return false;
                    };
                    let position = match drag.axis {
                        Axis::Horizontal => x,
                        Axis::Vertical => y,
                    };
                    app.workspace.layout_mut().resize(
                        &drag,
                        position,
                        crate::ui::pane_sash::PANE_MIN,
                    )
                });
            }
            Command::End => writer.update(|app| app.editor.sash_drag = None),
        },
    )
}
pub fn render(sash: Sash, state: State<AppState>) -> Element {
    crate::ui::pane_sash::render(sash, binding(&state))
}
