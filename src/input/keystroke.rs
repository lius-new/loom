//! Keystrokes: parsing keymap syntax, normalizing keyboard events and
//! formatting shortcut labels.
//!
//! The syntax follows Zed: modifiers joined with `-` before the key
//! (`ctrl-shift-g`), with `secondary-` meaning Cmd on macOS and Ctrl elsewhere.
//! `shift-` only describes letters; punctuation typed with Shift (`(`, `!`) is
//! written as the produced character and carries no Shift modifier.

use std::fmt;

use lgui::core::{KeyState, KeyboardEvent, LogicalKey, NamedKey};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Cmd on macOS, the Windows key on Windows, Super on Linux.
    pub platform: bool,
}

impl Modifiers {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn is_empty(self) -> bool {
        self == Self::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Keystroke {
    pub modifiers: Modifiers,
    /// Normalized key name: a lowercase letter, a single character, or a
    /// named key such as `tab`, `pageup` or `f5`.
    pub key: String,
    /// The character an AltGr chord produced on Windows (Ctrl+Alt+Q → `@`),
    /// which may also match a binding for that bare character.
    pub key_char: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeystrokeError(pub String);

impl fmt::Display for KeystrokeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KeystrokeError {}

/// Named keys accepted in keymap files, with their display labels.
const NAMED_KEYS: &[(&str, &str)] = &[
    ("escape", "Esc"),
    ("enter", "Enter"),
    ("tab", "Tab"),
    ("space", "Space"),
    ("backspace", "Backspace"),
    ("delete", "Delete"),
    ("insert", "Insert"),
    ("left", "Left"),
    ("right", "Right"),
    ("up", "Up"),
    ("down", "Down"),
    ("home", "Home"),
    ("end", "End"),
    ("pageup", "PageUp"),
    ("pagedown", "PageDown"),
    ("menu", "Menu"),
    ("f1", "F1"),
    ("f2", "F2"),
    ("f3", "F3"),
    ("f4", "F4"),
    ("f5", "F5"),
    ("f6", "F6"),
    ("f7", "F7"),
    ("f8", "F8"),
    ("f9", "F9"),
    ("f10", "F10"),
    ("f11", "F11"),
    ("f12", "F12"),
];

fn canonical_key_name(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    let alias = match lower.as_str() {
        "esc" => "escape",
        "return" => "enter",
        "del" => "delete",
        "pgup" => "pageup",
        "pgdn" => "pagedown",
        other => other,
    };
    NAMED_KEYS
        .iter()
        .find(|(key, _)| *key == alias)
        .map(|(key, _)| *key)
}

impl Keystroke {
    /// Parse one keystroke such as `secondary-shift-g`, `ctrl--` or `f5`.
    pub fn parse(source: &str) -> Result<Self, KeystrokeError> {
        let mut modifiers = Modifiers::default();
        let mut rest = source;
        while let Some((head, tail)) = rest.split_once('-') {
            // `ctrl--` binds the minus key: the final component is the key.
            if tail.is_empty() && head.is_empty() {
                break;
            }
            match head.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => modifiers.ctrl = true,
                "alt" | "option" => modifiers.alt = true,
                "shift" => modifiers.shift = true,
                "cmd" | "super" | "win" => modifiers.platform = true,
                "secondary" => {
                    if cfg!(target_os = "macos") {
                        modifiers.platform = true;
                    } else {
                        modifiers.ctrl = true;
                    }
                }
                _ if tail.is_empty() => break,
                other => {
                    return Err(KeystrokeError(format!(
                        "unknown modifier `{other}` in `{source}`"
                    )));
                }
            }
            rest = tail;
        }
        if rest.is_empty() {
            return Err(KeystrokeError(format!("missing key in `{source}`")));
        }
        let mut chars = rest.chars();
        let first = chars.next().expect("rest is not empty");
        let key = if chars.next().is_none() {
            if first == ' ' {
                "space".to_owned()
            } else if first.is_uppercase() {
                // `G` is shorthand for `shift-g`.
                modifiers.shift = true;
                first.to_lowercase().collect()
            } else {
                first.to_string()
            }
        } else {
            canonical_key_name(rest)
                .ok_or_else(|| KeystrokeError(format!("unknown key `{rest}` in `{source}`")))?
                .to_owned()
        };
        Ok(Self {
            modifiers,
            key,
            key_char: None,
        })
    }

    /// Parse a space-separated keystroke sequence such as `ctrl-k ctrl-s`.
    pub fn parse_sequence(source: &str) -> Result<Vec<Self>, KeystrokeError> {
        let sequence = source
            .split_whitespace()
            .map(Self::parse)
            .collect::<Result<Vec<_>, _>>()?;
        if sequence.is_empty() {
            return Err(KeystrokeError("empty keystroke".to_owned()));
        }
        Ok(sequence)
    }

    /// Normalize a key-down event. Returns `None` for key releases, lone
    /// modifier keys and keys that can not be bound.
    pub fn from_event(event: &KeyboardEvent) -> Option<Self> {
        if event.state != KeyState::Down {
            return None;
        }
        let mut modifiers = Modifiers {
            ctrl: event.modifiers.ctrl(),
            alt: event.modifiers.alt(),
            shift: event.modifiers.shift(),
            platform: event.modifiers.meta(),
        };
        let mut key_char = None;
        let key = match &event.key {
            LogicalKey::Named(named) => named_key(named)?.to_owned(),
            LogicalKey::Character(value) => {
                let mut chars = value.chars();
                let character = chars.next()?;
                if chars.next().is_some() {
                    return None;
                }
                if character == ' ' {
                    "space".to_owned()
                } else if character.is_alphabetic()
                    && character.is_uppercase() != character.is_lowercase()
                {
                    character.to_lowercase().collect()
                } else {
                    // The character already reflects Shift (`!` rather than `shift-1`).
                    modifiers.shift = false;
                    if modifiers.ctrl && modifiers.alt && !character.is_ascii_alphanumeric() {
                        key_char = Some(character.to_string());
                    }
                    character.to_string()
                }
            }
        };
        Some(Self {
            modifiers,
            key,
            key_char,
        })
    }

    /// Whether this typed keystroke satisfies `target`, a keystroke from the keymap.
    pub fn should_match(&self, target: &Keystroke) -> bool {
        if let Some(key_char) = &self.key_char
            && target.modifiers.is_empty()
            && &target.key == key_char
        {
            return true;
        }
        self.modifiers == target.modifiers && self.key == target.key
    }

    /// Whether typing this keystroke would insert text when no binding claims it.
    pub fn is_printable(&self) -> bool {
        !self.modifiers.ctrl
            && !self.modifiers.alt
            && !self.modifiers.platform
            && (self.key == "space" || self.key.chars().count() == 1)
    }

    /// The text this keystroke types, for replaying an abandoned sequence.
    pub fn text(&self) -> Option<String> {
        if !self.is_printable() {
            return None;
        }
        Some(if self.key == "space" {
            " ".to_owned()
        } else if self.modifiers.shift {
            self.key.to_uppercase()
        } else {
            self.key.clone()
        })
    }

    /// Platform-native label such as `Ctrl+Shift+G` or `⌘⇧G`.
    pub fn label(&self) -> String {
        let key = key_label(&self.key);
        if cfg!(target_os = "macos") {
            let mut label = String::new();
            if self.modifiers.ctrl {
                label.push('⌃');
            }
            if self.modifiers.alt {
                label.push('⌥');
            }
            if self.modifiers.shift {
                label.push('⇧');
            }
            if self.modifiers.platform {
                label.push('⌘');
            }
            label.push_str(&key);
            label
        } else {
            let mut parts = Vec::new();
            if self.modifiers.ctrl {
                parts.push("Ctrl".to_owned());
            }
            if self.modifiers.platform {
                parts.push(if cfg!(windows) { "Win" } else { "Super" }.to_owned());
            }
            if self.modifiers.alt {
                parts.push("Alt".to_owned());
            }
            if self.modifiers.shift {
                parts.push("Shift".to_owned());
            }
            parts.push(key);
            parts.join("+")
        }
    }

    /// Label for a whole sequence, e.g. `Ctrl+K Ctrl+S`.
    pub fn sequence_label(sequence: &[Keystroke]) -> String {
        sequence
            .iter()
            .map(Keystroke::label)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Keymap syntax (`ctrl-shift-g`); round-trips through [`Keystroke::parse`].
impl fmt::Display for Keystroke {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.modifiers.ctrl {
            f.write_str("ctrl-")?;
        }
        if self.modifiers.alt {
            f.write_str("alt-")?;
        }
        if self.modifiers.shift {
            f.write_str("shift-")?;
        }
        if self.modifiers.platform {
            f.write_str("cmd-")?;
        }
        f.write_str(&self.key)
    }
}

fn key_label(key: &str) -> String {
    if let Some((_, label)) = NAMED_KEYS.iter().find(|(name, _)| *name == key) {
        return (*label).to_owned();
    }
    key.to_uppercase()
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

    fn mods(ctrl: bool, alt: bool, shift: bool, platform: bool) -> Modifiers {
        Modifiers {
            ctrl,
            alt,
            shift,
            platform,
        }
    }

    #[test]
    fn parses_modifiers_keys_and_aliases() {
        let stroke = Keystroke::parse("ctrl-shift-g").unwrap();
        assert_eq!(stroke.modifiers, mods(true, false, true, false));
        assert_eq!(stroke.key, "g");

        assert_eq!(
            Keystroke::parse("cmd-,").unwrap().modifiers,
            mods(false, false, false, true)
        );
        assert_eq!(Keystroke::parse("esc").unwrap().key, "escape");
        assert_eq!(Keystroke::parse("PageDown").unwrap().key, "pagedown");
        assert_eq!(Keystroke::parse("ctrl--").unwrap().key, "-");
        assert_eq!(Keystroke::parse("-").unwrap().key, "-");
        assert_eq!(Keystroke::parse("alt-`").unwrap().key, "`");
    }

    #[test]
    fn uppercase_letters_imply_shift() {
        assert_eq!(
            Keystroke::parse("G").unwrap(),
            Keystroke::parse("shift-g").unwrap()
        );
    }

    #[test]
    fn secondary_follows_the_platform_convention() {
        let stroke = Keystroke::parse("secondary-s").unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(stroke.modifiers, mods(false, false, false, true));
        } else {
            assert_eq!(stroke.modifiers, mods(true, false, false, false));
        }
    }

    #[test]
    fn rejects_unknown_modifiers_and_keys() {
        assert!(Keystroke::parse("hyper-a").is_err());
        assert!(Keystroke::parse("ctrl-banana").is_err());
        assert!(Keystroke::parse("ctrl-").is_err());
        assert!(Keystroke::parse_sequence("   ").is_err());
    }

    #[test]
    fn parses_sequences() {
        let sequence = Keystroke::parse_sequence("ctrl-k  ctrl-s").unwrap();
        assert_eq!(sequence.len(), 2);
        assert_eq!(sequence[1], Keystroke::parse("ctrl-s").unwrap());
    }

    #[test]
    fn events_normalize_letter_case_and_keep_shift() {
        let typed =
            Keystroke::from_event(&character("G", KeyModifiers::CONTROL | KeyModifiers::SHIFT))
                .unwrap();
        assert_eq!(typed, Keystroke::parse("ctrl-shift-g").unwrap());
        let typed = Keystroke::from_event(&character("b", KeyModifiers::CONTROL)).unwrap();
        assert!(typed.should_match(&Keystroke::parse("ctrl-b").unwrap()));
    }

    #[test]
    fn shifted_punctuation_drops_the_shift_modifier() {
        let typed = Keystroke::from_event(&character("(", KeyModifiers::SHIFT)).unwrap();
        assert_eq!(typed, Keystroke::parse("(").unwrap());
    }

    #[test]
    fn named_keys_keep_shift() {
        let typed = Keystroke::from_event(&event(
            LogicalKey::Named(NamedKey::Tab),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ))
        .unwrap();
        assert_eq!(typed, Keystroke::parse("ctrl-shift-tab").unwrap());
    }

    #[test]
    fn lone_modifiers_and_releases_are_ignored() {
        assert!(
            Keystroke::from_event(&event(
                LogicalKey::Named(NamedKey::Shift),
                KeyModifiers::SHIFT
            ))
            .is_none()
        );
        let mut release = character("a", KeyModifiers::CONTROL);
        release.state = KeyState::Up;
        assert!(Keystroke::from_event(&release).is_none());
    }

    #[test]
    fn altgr_characters_match_their_bare_binding() {
        let typed =
            Keystroke::from_event(&character("@", KeyModifiers::CONTROL | KeyModifiers::ALT))
                .unwrap();
        assert!(typed.should_match(&Keystroke::parse("@").unwrap()));
        assert!(typed.should_match(&Keystroke::parse("ctrl-alt-@").unwrap()));
        assert!(!typed.should_match(&Keystroke::parse("ctrl-@").unwrap()));
    }

    #[test]
    fn printable_keystrokes_replay_as_text() {
        assert_eq!(
            Keystroke::parse("shift-j").unwrap().text().as_deref(),
            Some("J")
        );
        assert_eq!(
            Keystroke::parse("space").unwrap().text().as_deref(),
            Some(" ")
        );
        assert_eq!(Keystroke::parse("ctrl-j").unwrap().text(), None);
        assert_eq!(Keystroke::parse("enter").unwrap().text(), None);
    }

    #[test]
    fn labels_and_display_round_trip() {
        let stroke = Keystroke::parse("ctrl-shift-g").unwrap();
        assert_eq!(Keystroke::parse(&stroke.to_string()).unwrap(), stroke);
        if !cfg!(target_os = "macos") {
            assert_eq!(stroke.label(), "Ctrl+Shift+G");
            assert_eq!(
                Keystroke::parse("ctrl-pageup").unwrap().label(),
                "Ctrl+PageUp"
            );
            assert_eq!(
                Keystroke::sequence_label(&Keystroke::parse_sequence("ctrl-k ctrl-s").unwrap()),
                "Ctrl+K Ctrl+S"
            );
        }
    }
}
