//! The text field that replaces a session tab's name while it is renamed.

use std::sync::Arc;

use lgui::core::{EventPolicy, SemanticRole, Semantics};
use lgui::prelude::{Element, UiRect, VisualStyle, component, panel};

use super::bindings::{Binding, Command};
use super::state::TerminalUiState;
use crate::ui::components::input::{
    self, AccessInputOptions, InputBinding, InputState, InputStyle,
};
use crate::ui::theme;

/// Narrowest a tab grows to while its name is edited, so there is room to type.
pub(super) const MIN_TAB_W: f32 = 160.0;

/// The field, drawn inside `rect`. Enter and clicking elsewhere keep the typed
/// name, Escape drops it; either way the terminal takes the keyboard back.
pub(super) fn input(
    rect: UiRect,
    state: Binding,
    terminal_focus: lgui::core::UiFocusHandle,
) -> Element {
    let field = rect;
    let commit = state.clone();
    let cancel = state.clone();
    let blur = state.clone();
    let (commit_focus, cancel_focus) = (terminal_focus.clone(), terminal_focus);
    // Keys and text typed here belong to the field, never to the shell behind it.
    // No box of its own: the tab's pill is the frame.
    panel(rect, VisualStyle::default())
        .key("terminal-rename-field")
        .event_policy(EventPolicy::INTERACTIVE)
        .on_input(|cx, _| cx.stop_propagation())
        .on_key_down(|cx, _| cx.stop_propagation())
        .child(
            component(field, move |cx, &field| {
                let id = cx.use_stable_id();
                let focus = cx.focus_handle(id.clone());
                let mounted = focus.clone();
                cx.use_effect((), move || mounted.focus());
                let read: fn(&TerminalUiState) -> &InputState = TerminalUiState::rename_input;
                let write: fn(&mut TerminalUiState) -> &mut InputState =
                    TerminalUiState::rename_input_mut;
                let (commit, cancel, blur) = (commit.clone(), cancel.clone(), blur.clone());
                let (commit_focus, cancel_focus) = (commit_focus.clone(), cancel_focus.clone());
                input::render_access(
                    field,
                    id,
                    focus,
                    InputBinding::new_access(state.ui.clone(), read, write),
                    AccessInputOptions {
                        label: "Session name",
                        placeholder: "Automatic name",
                        style: InputStyle {
                            text: theme::mono(theme::c().text, theme::UI_SIZE),
                            placeholder: theme::mono(theme::c().text_faint, theme::UI_SIZE),
                            caret: theme::c().text,
                            selection: theme::c().accent,
                        },
                        on_submit: Some(Arc::new(move || {
                            commit.run(Command::CommitRename);
                            commit_focus.focus();
                        })),
                        on_cancel: Some(Arc::new(move || {
                            cancel.run(Command::CancelRename);
                            cancel_focus.focus();
                        })),
                        on_blur: Some(Arc::new(move || {
                            blur.run(Command::CommitRename);
                        })),
                    },
                )
            })
            .key("terminal-rename-input")
            .semantics(Semantics::new(SemanticRole::TextInput).name("Session name")),
        )
}
