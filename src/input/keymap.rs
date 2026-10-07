//! Maps raw keyboard events to high-level editor actions.
//!
//! Keeps the concrete `lgui` key types out of the rest of the app: the UI and
//! state layers only ever see the `Action` enum.

use lgui::core::{KeyState, KeyboardEvent, LogicalKey, NamedKey};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    CloneRepository,
    OpenSourceControl,
    OpenExplorer,
    ToggleDrawer,
    ToggleTerminal,
    CloseOverlay,
    NextFile,
    PrevFile,
    SelectFile(usize),
    SelectLastFile,
    CloseActiveFile,
}

/// Convert a key event into an action, or `None` if it is not a shortcut.
///
/// Shortcuts (Cmd/Ctrl is accepted so it works on macOS and Linux/Windows):
/// * Cmd/Ctrl+Shift+G — source control
/// * Cmd/Ctrl+B — toggle file drawer
/// * Cmd/Ctrl+W — close the active tab (dirty files ask first)
/// * Cmd/Ctrl+1..8, Cmd/Ctrl+9 — select a tab by position, or the last tab
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
                "b" | "B" => Some(Action::ToggleDrawer),
                "g" | "G" if ev.modifiers.shift() => Some(Action::OpenSourceControl),
                "`" => Some(Action::ToggleTerminal),
                "w" | "W" => Some(Action::CloseActiveFile),
                "1" => Some(Action::SelectFile(0)),
                "2" => Some(Action::SelectFile(1)),
                "3" => Some(Action::SelectFile(2)),
                "4" => Some(Action::SelectFile(3)),
                "5" => Some(Action::SelectFile(4)),
                "6" => Some(Action::SelectFile(5)),
                "7" => Some(Action::SelectFile(6)),
                "8" => Some(Action::SelectFile(7)),
                "9" => Some(Action::SelectLastFile),
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

#[cfg(test)]
mod tests {
    use super::*;
    use lgui::core::KeyModifiers;

    fn event(value: &str) -> KeyboardEvent {
        KeyboardEvent {
            state: KeyState::Down,
            key: LogicalKey::Character(value.into()),
            modifiers: KeyModifiers::CONTROL,
            ..Default::default()
        }
    }

    #[test]
    fn number_shortcuts_select_positions_and_nine_selects_the_last_tab() {
        assert_eq!(action_for(&event("1")), Some(Action::SelectFile(0)));
        assert_eq!(action_for(&event("8")), Some(Action::SelectFile(7)));
        assert_eq!(action_for(&event("9")), Some(Action::SelectLastFile));
    }

    #[test]
    fn control_w_closes_the_active_tab() {
        assert_eq!(action_for(&event("w")), Some(Action::CloseActiveFile));
        assert_eq!(action_for(&event("W")), Some(Action::CloseActiveFile));
    }

    #[test]
    fn control_p_is_not_reserved_by_the_tab_experience() {
        assert_eq!(action_for(&event("p")), None);
        assert_eq!(action_for(&event("P")), None);
    }
}
