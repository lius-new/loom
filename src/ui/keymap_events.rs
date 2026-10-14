//! Decode GUI key events into pure key facts.
use crate::features::keymap::keystroke::{KeyInput, Keystroke, Modifiers};
use lgui::core::{KeyState, KeyboardEvent, LogicalKey, NamedKey};
pub fn from_event(event: &KeyboardEvent) -> Option<Keystroke> {
    if event.state != KeyState::Down {
        return None;
    }
    let modifiers = Modifiers {
        ctrl: event.modifiers.ctrl(),
        alt: event.modifiers.alt(),
        shift: event.modifiers.shift(),
        platform: event.modifiers.meta(),
    };
    let input = match &event.key {
        LogicalKey::Named(name) => KeyInput::Named(named_key(name)?),
        LogicalKey::Character(value) => KeyInput::Character(value),
    };
    Keystroke::from_key(input, modifiers)
}
fn named_key(named: &NamedKey) -> Option<&'static str> {
    Some(match named {
        NamedKey::Escape => "escape",
        NamedKey::Enter => "enter",
        NamedKey::Tab => "tab",
        NamedKey::Backspace => "backspace",
        NamedKey::Delete => "delete",
        NamedKey::Insert => "insert",
        NamedKey::ArrowLeft => "left",
        NamedKey::ArrowRight => "right",
        NamedKey::ArrowUp => "up",
        NamedKey::ArrowDown => "down",
        NamedKey::Home => "home",
        NamedKey::End => "end",
        NamedKey::PageUp => "pageup",
        NamedKey::PageDown => "pagedown",
        NamedKey::ContextMenu => "menu",
        NamedKey::F1 => "f1",
        NamedKey::F2 => "f2",
        NamedKey::F3 => "f3",
        NamedKey::F4 => "f4",
        NamedKey::F5 => "f5",
        NamedKey::F6 => "f6",
        NamedKey::F7 => "f7",
        NamedKey::F8 => "f8",
        NamedKey::F9 => "f9",
        NamedKey::F10 => "f10",
        NamedKey::F11 => "f11",
        NamedKey::F12 => "f12",
        _ => return None,
    })
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

    fn character(value: &str, modifiers: KeyModifiers) -> KeyboardEvent {
        event(LogicalKey::Character(value.into()), modifiers)
    }

    #[test]
    fn events_normalize_letter_case_and_keep_shift() {
        let typed = crate::ui::keymap_events::from_event(&character(
            "G",
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ))
        .unwrap();
        assert_eq!(typed, Keystroke::parse("ctrl-shift-g").unwrap());
        let typed =
            crate::ui::keymap_events::from_event(&character("b", KeyModifiers::CONTROL)).unwrap();
        assert!(typed.should_match(&Keystroke::parse("ctrl-b").unwrap()));
    }

    #[test]
    fn shifted_punctuation_drops_the_shift_modifier() {
        let typed =
            crate::ui::keymap_events::from_event(&character("(", KeyModifiers::SHIFT)).unwrap();
        assert_eq!(typed, Keystroke::parse("(").unwrap());
    }

    #[test]
    fn named_keys_keep_shift() {
        let typed = crate::ui::keymap_events::from_event(&event(
            LogicalKey::Named(NamedKey::Tab),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ))
        .unwrap();
        assert_eq!(typed, Keystroke::parse("ctrl-shift-tab").unwrap());
    }

    #[test]
    fn lone_modifiers_and_releases_are_ignored() {
        assert!(
            crate::ui::keymap_events::from_event(&event(
                LogicalKey::Named(NamedKey::Shift),
                KeyModifiers::SHIFT
            ))
            .is_none()
        );
        let mut release = character("a", KeyModifiers::CONTROL);
        release.state = KeyState::Up;
        assert!(crate::ui::keymap_events::from_event(&release).is_none());
    }

    #[test]
    fn altgr_characters_match_their_bare_binding() {
        let typed = crate::ui::keymap_events::from_event(&character(
            "@",
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ))
        .unwrap();
        assert!(typed.should_match(&Keystroke::parse("@").unwrap()));
        assert!(typed.should_match(&Keystroke::parse("ctrl-alt-@").unwrap()));
        assert!(!typed.should_match(&Keystroke::parse("ctrl-@").unwrap()));
    }
}
