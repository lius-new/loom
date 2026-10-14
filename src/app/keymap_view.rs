//! Keymap view composition; UI does not discover configuration paths.
use crate::app::context::AppContext;
use crate::features::keymap::bindings;
use crate::ui::components::state_access::StateAccess;
use crate::ui::keymap_binding::{Binding, Command, Frame};
use lgui::core::{UiFocusHandle, UiId};
use lgui::prelude::{Element, UiRect};
pub fn binding(state: &AppContext) -> Binding {
    let reader = state.clone();
    let writer = state.clone();
    Binding::new(
        StateAccess::project(
            state.model(),
            |app| &app.keymap_ui,
            |app| &mut app.keymap_ui,
        ),
        move || {
            let app = reader.get();
            Frame {
                keymap: bindings::current(),
                scroll: app.workspace.active_scroll(),
                location: crate::infrastructure::keymap_filesystem::user_path()
                    .map(|path| {
                        format!(
                            "Customize bindings in {}; saved changes apply immediately.",
                            path.display()
                        )
                    })
                    .unwrap_or_else(|| {
                        "Customize bindings in keymap.json; saved changes apply immediately."
                            .to_owned()
                    }),
            }
        },
        move |command| run(&writer, command),
    )
}
pub fn run(state: &AppContext, command: Command) {
    if matches!(command, Command::OpenUserFile) {
        crate::app::key_actions::open_user_keymap(state);
        return;
    }
    state.update(|app| match command {
        Command::Scroll { delta, max } => {
            let (x, y) = app.workspace.active_scroll();
            app.workspace
                .set_active_scroll(x, (y - delta).clamp(0.0, max));
        }
        Command::ClearSearch => app.keymap_ui.search.clear(),
        Command::OpenUserFile => {}
    });
}
pub fn render(rect: UiRect, state: AppContext, id: UiId, focus: UiFocusHandle) -> Element {
    crate::ui::keymap_page::render(rect, binding(&state), id, focus)
}
