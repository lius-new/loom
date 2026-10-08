//! Shared keyboard and context-menu editing commands.
use super::normal::{self, Movement};
use crate::model::text::EditorMut;
use lgui::core::{KeyState, KeyboardEvent, LogicalKey, NamedKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Move(Movement, bool),
    Delete(bool, bool),
    Enter,
    Indent(bool),
    SelectAll,
    Escape,
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
}
pub fn command_for(ev: &KeyboardEvent, page: usize) -> Option<Command> {
    if ev.state != KeyState::Down || ev.modifiers.alt() {
        return None;
    }
    let ctrl = ev.modifiers.ctrl() || ev.modifiers.meta();
    let shift = ev.modifiers.shift();
    if ctrl && let LogicalKey::Character(c) = &ev.key {
        return match c.to_lowercase().as_str() {
            "a" => Some(Command::SelectAll),
            "c" => Some(Command::Copy),
            "x" => Some(Command::Cut),
            "v" => Some(Command::Paste),
            "z" => Some(if shift { Command::Redo } else { Command::Undo }),
            "y" => Some(Command::Redo),
            _ => None,
        };
    }
    let movement = match &ev.key {
        LogicalKey::Named(NamedKey::ArrowLeft) => {
            if ctrl {
                Movement::WordLeft
            } else {
                Movement::Left
            }
        }
        LogicalKey::Named(NamedKey::ArrowRight) => {
            if ctrl {
                Movement::WordRight
            } else {
                Movement::Right
            }
        }
        LogicalKey::Named(NamedKey::ArrowUp) if !ctrl => Movement::Up,
        LogicalKey::Named(NamedKey::ArrowDown) if !ctrl => Movement::Down,
        LogicalKey::Named(NamedKey::Home) => {
            if ctrl {
                Movement::Start
            } else {
                Movement::Home
            }
        }
        LogicalKey::Named(NamedKey::End) => {
            if ctrl {
                Movement::Finish
            } else {
                Movement::End
            }
        }
        LogicalKey::Named(NamedKey::PageUp) if !ctrl => Movement::PageUp(page),
        LogicalKey::Named(NamedKey::PageDown) if !ctrl => Movement::PageDown(page),
        LogicalKey::Named(NamedKey::Backspace) => return Some(Command::Delete(false, ctrl)),
        LogicalKey::Named(NamedKey::Delete) => return Some(Command::Delete(true, ctrl)),
        LogicalKey::Named(NamedKey::Enter) if !ctrl => return Some(Command::Enter),
        LogicalKey::Named(NamedKey::Tab) if !ctrl => return Some(Command::Indent(shift)),
        LogicalKey::Named(NamedKey::Escape) => return Some(Command::Escape),
        _ => return None,
    };
    Some(Command::Move(movement, shift))
}
pub fn apply(editor: &mut EditorMut<'_>, command: Command) {
    match command {
        Command::Move(movement, extend) => normal::navigate(editor, movement, extend),
        Command::Delete(forward, word) => normal::erase(editor, forward, word),
        Command::Enter => normal::enter(editor),
        Command::Indent(outdent) => normal::indent(editor, outdent),
        Command::SelectAll => editor.select_all(),
        Command::Escape => editor.clear_selection(),
        Command::Undo => {
            editor.undo();
        }
        Command::Redo => {
            editor.redo();
        }
        Command::Copy | Command::Cut | Command::Paste => {}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use lgui::core::KeyModifiers;
    fn event(key: LogicalKey, modifiers: KeyModifiers) -> KeyboardEvent {
        KeyboardEvent {
            state: KeyState::Down,
            key,
            modifiers,
            ..Default::default()
        }
    }
    #[test]
    fn modifiers_distinguish_editor_shortcuts_and_global_tab_switching() {
        assert_eq!(
            command_for(
                &event(LogicalKey::Named(NamedKey::Tab), KeyModifiers::CONTROL),
                20
            ),
            None
        );
        assert_eq!(
            command_for(
                &event(
                    LogicalKey::Named(NamedKey::Home),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT
                ),
                20
            ),
            Some(Command::Move(Movement::Start, true))
        );
        assert_eq!(
            command_for(
                &event(
                    LogicalKey::Character("z".into()),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT
                ),
                20
            ),
            Some(Command::Redo)
        );
        assert_eq!(
            command_for(
                &event(LogicalKey::Named(NamedKey::PageDown), KeyModifiers::SHIFT),
                20
            ),
            Some(Command::Move(Movement::PageDown(20), true))
        );
    }
}
