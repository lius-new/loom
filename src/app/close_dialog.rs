//! Dirty-close command/persistence coordination and modal composition.
use crate::app::context::AppContext;
use crate::app::state::CloseContinuation;
use crate::app::workspace_commands as workspace_actions;
use crate::features::keymap::action::Action;
use crate::ui::chrome::{CloseBinding, CloseCommand, CloseOutcome};
use lgui::core::{UiFocusHandle, UiId};
use lgui::prelude::{Element, UiRect};
use lgui::window::WindowManager;
pub fn message(state: &AppContext) -> Option<String> {
    let snapshot = state.get();
    let request = snapshot.editor.close_request.as_ref()?;
    let dirty_targets = snapshot
        .workspace
        .released_by(&request.targets)
        .into_iter()
        .filter(|id| snapshot.workspace.is_dirty(*id))
        .collect::<Vec<_>>();
    let dirty_count = dirty_targets.len();
    let target_name = dirty_targets
        .first()
        .and_then(|id| snapshot.workspace.meta(*id))
        .map(|meta| meta.name.clone())
        .unwrap_or_else(|| "this file".to_owned());
    let message = if dirty_count == 1 {
        format!("Do you want to save the changes you made to {target_name}?")
    } else {
        format!("Do you want to save changes to {dirty_count} files?")
    };

    Some(message)
}
pub fn run(state: &AppContext, command: CloseCommand) -> CloseOutcome {
    let continuation = match command {
        CloseCommand::Save => workspace_actions::save_close_request(state),
        CloseCommand::Discard => workspace_actions::discard_close_request(state),
        CloseCommand::Cancel => {
            workspace_actions::cancel_close_request(state);
            None
        }
    };
    CloseOutcome {
        exit: continuation == Some(CloseContinuation::ExitApplication)
            && workspace_actions::save_editor_session(state),
        resolved: continuation != Some(CloseContinuation::ExitApplication)
            && state.get().editor.close_request.is_none(),
    }
}
pub fn binding(state: &AppContext) -> CloseBinding {
    let reader = state.clone();
    let writer = state.clone();
    CloseBinding::new(
        move || message(&reader),
        move |command| run(&writer, command),
    )
}
pub fn render(rect: UiRect, state: AppContext, id: UiId, focus: UiFocusHandle) -> Element {
    crate::ui::close_confirmation::render(rect, binding(&state), id, focus)
}
/// Perform a `dialog::` action from the keymap: Confirm saves, Secondary
/// discards ("Don't Save") and Cancel keeps the files open.
pub fn perform(
    state: &AppContext,
    editor_focus: &UiFocusHandle,
    windows: &WindowManager,
    action: Action,
) -> bool {
    let continuation = match action {
        Action::DialogConfirm => workspace_actions::save_close_request(state),
        Action::DialogSecondary => workspace_actions::discard_close_request(state),
        Action::DialogCancel => {
            workspace_actions::cancel_close_request(state);
            None
        }
        _ => return false,
    };
    if continuation == Some(CloseContinuation::ExitApplication) {
        if workspace_actions::save_editor_session(state) {
            windows.close("loom");
        }
    } else if state.get().editor.close_request.is_none() {
        editor_focus.focus();
    }
    true
}
