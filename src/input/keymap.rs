//! Maps raw keyboard events to high-level editor actions.
//!
//! Keeps the concrete `lgui` key types out of the rest of the app: the UI and
//! state layers only ever see the `Action` enum.

use lgui::core::{KeyState, KeyboardEvent, LogicalKey, NamedKey};

use crate::model::document::FileId;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    OpenPalette,
    OpenCmdK,
    ToggleDrawer,
    ToggleAssistant,
    ToggleTerminal,
    CloseOverlay,
    OpenFile(FileId),
    CloseFile(FileId),
    NextFile,
    PrevFile,
}

/// Convert a key event into an action, or `None` if it is not a shortcut.
///
/// Shortcuts (Cmd/Ctrl is accepted so it works on macOS and Linux/Windows):
/// * Cmd/Ctrl+P — command palette
/// * Cmd/Ctrl+K — inline AI prompt
/// * Cmd/Ctrl+B — toggle file drawer
/// * Cmd/Ctrl+L — toggle assistant
/// * Esc         — close overlays
pub fn action_for(ev: &KeyboardEvent) -> Option<Action> {
    if ev.state != KeyState::Down {
        return None;
    }

    let cmd = ev.modifiers.ctrl() || ev.modifiers.meta();
    if cmd {
        if let LogicalKey::Named(NamedKey::Tab) = &ev.key {
            return Some(if ev.modifiers.shift() {
                Action::PrevFile
            } else {
                Action::NextFile
            });
        }
        return match &ev.key {
            LogicalKey::Character(c) => match c.as_str() {
                "p" | "P" => Some(Action::OpenPalette),
                "k" | "K" => Some(Action::OpenCmdK),
                "b" | "B" => Some(Action::ToggleDrawer),
                "l" | "L" => Some(Action::ToggleAssistant),
                "`" => Some(Action::ToggleTerminal),
                _ => None,
            },
            _ => None,
        };
    }

    if let LogicalKey::Named(NamedKey::Escape) = &ev.key {
        return Some(Action::CloseOverlay);
    }

    None
}
