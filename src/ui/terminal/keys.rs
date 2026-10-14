//! Translate framework events; protocol encoding belongs to the terminal feature.
use crate::features::terminal::input::{self, Modifiers, TerminalKey};
use lgui::core::{KeyModifiers, KeyState, KeyboardEvent, LogicalKey, NamedKey};

pub(crate) fn modifiers(value: KeyModifiers) -> Modifiers {
    let mut result = Modifiers::empty();
    if value.shift() {
        result = result | Modifiers::SHIFT;
    }
    if value.alt() {
        result = result | Modifiers::ALT;
    }
    if value.ctrl() {
        result = result | Modifiers::CONTROL;
    }
    result
}
pub(crate) fn key_bytes(event: &KeyboardEvent, application_cursor: bool) -> Option<Vec<u8>> {
    if event.state != KeyState::Down {
        return None;
    }
    let key = match &event.key {
        LogicalKey::Character(value) => TerminalKey::Character(value.as_ref()),
        LogicalKey::Named(named) => TerminalKey::Named(match named {
            NamedKey::ArrowUp => input::NamedKey::ArrowUp,
            NamedKey::ArrowDown => input::NamedKey::ArrowDown,
            NamedKey::ArrowRight => input::NamedKey::ArrowRight,
            NamedKey::ArrowLeft => input::NamedKey::ArrowLeft,
            NamedKey::Home => input::NamedKey::Home,
            NamedKey::End => input::NamedKey::End,
            NamedKey::Insert => input::NamedKey::Insert,
            NamedKey::Delete => input::NamedKey::Delete,
            NamedKey::PageUp => input::NamedKey::PageUp,
            NamedKey::PageDown => input::NamedKey::PageDown,
            NamedKey::Backspace => input::NamedKey::Backspace,
            NamedKey::Enter => input::NamedKey::Enter,
            NamedKey::Tab => input::NamedKey::Tab,
            NamedKey::Escape => input::NamedKey::Escape,
            _ => return None,
        }),
    };
    input::key_bytes(key, modifiers(event.modifiers), application_cursor)
}
