//! GUI-independent terminal protocol encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Modifiers(u8);
impl Modifiers {
    pub const SHIFT: Self = Self(1);
    pub const ALT: Self = Self(2);
    pub const CONTROL: Self = Self(4);
    pub const fn empty() -> Self {
        Self(0)
    }
    pub fn shift(self) -> bool {
        self.0 & 1 != 0
    }
    pub fn alt(self) -> bool {
        self.0 & 2 != 0
    }
    pub fn ctrl(self) -> bool {
        self.0 & 4 != 0
    }
}
impl std::ops::BitOr for Modifiers {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NamedKey {
    ArrowUp,
    ArrowDown,
    ArrowRight,
    ArrowLeft,
    Home,
    End,
    Insert,
    Delete,
    PageUp,
    PageDown,
    Backspace,
    Enter,
    Tab,
    Escape,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalKey<'a> {
    Named(NamedKey),
    Character(&'a str),
}

pub(crate) fn key_bytes(
    key: TerminalKey<'_>,
    modifiers: Modifiers,
    application_cursor: bool,
) -> Option<Vec<u8>> {
    let shift = modifiers.shift();
    let alt = modifiers.alt();
    let ctrl = modifiers.ctrl();
    // xterm modifier parameter: 1 + Shift(1) + Alt(2) + Ctrl(4).
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    if let TerminalKey::Named(named) = key {
        let cursor_final = match named {
            NamedKey::ArrowUp => Some('A'),
            NamedKey::ArrowDown => Some('B'),
            NamedKey::ArrowRight => Some('C'),
            NamedKey::ArrowLeft => Some('D'),
            NamedKey::Home => Some('H'),
            NamedKey::End => Some('F'),
            _ => None,
        };
        if let Some(final_byte) = cursor_final {
            return Some(
                if modifier > 1 {
                    format!("\x1b[1;{modifier}{final_byte}")
                } else if application_cursor {
                    format!("\x1bO{final_byte}")
                } else {
                    format!("\x1b[{final_byte}")
                }
                .into_bytes(),
            );
        }
        let tilde_code = match named {
            NamedKey::Insert => Some(2),
            NamedKey::Delete => Some(3),
            NamedKey::PageUp => Some(5),
            NamedKey::PageDown => Some(6),
            _ => None,
        };
        if let Some(code) = tilde_code {
            return Some(
                if modifier > 1 {
                    format!("\x1b[{code};{modifier}~")
                } else {
                    format!("\x1b[{code}~")
                }
                .into_bytes(),
            );
        }
        if named == NamedKey::Backspace {
            // Ctrl+Backspace deletes a word (^H), Alt+Backspace sends ESC DEL.
            return Some(match (ctrl, alt) {
                (true, _) => b"\x08".to_vec(),
                (false, true) => b"\x1b\x7f".to_vec(),
                (false, false) => b"\x7f".to_vec(),
            });
        }
    }

    if modifiers.ctrl()
        && let TerminalKey::Character(value) = key
        && let Some(character) = value.chars().next()
        && value.chars().count() == 1
    {
        let lower = character.to_ascii_lowercase();
        // These remain editor-wide shortcuts while the terminal is focused.
        if matches!(lower, 'p' | 'b' | 'g' | '`') {
            return None;
        }
        if lower.is_ascii_lowercase() {
            return Some(vec![(lower as u8) & 0x1f]);
        }
    }

    let bytes: &[u8] = match key {
        TerminalKey::Named(NamedKey::Enter) => b"\r",
        TerminalKey::Named(NamedKey::Tab) if shift => b"\x1b[Z",
        TerminalKey::Named(NamedKey::Tab) => b"\t",
        TerminalKey::Named(NamedKey::Escape) => b"\x1b",
        _ => return None,
    };
    Some(bytes.to_vec())
}

pub(crate) fn paste_bytes(value: &str, bracketed: bool) -> Vec<u8> {
    // Shells submit lines on carriage return; a bare LF would not run them.
    let normalized = value.replace("\r\n", "\r").replace('\n', "\r");
    let mut bytes = Vec::with_capacity(normalized.len() + 12);
    if bracketed {
        bytes.extend_from_slice(b"\x1b[200~");
        // The payload must not be able to end bracketing early.
        bytes.extend_from_slice(normalized.replace("\x1b[201~", "").as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
    } else {
        bytes.extend_from_slice(normalized.as_bytes());
    }
    bytes
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paste_normalizes_newlines_and_honors_bracketed_mode() {
        assert_eq!(paste_bytes("a\r\nb\nc", false), b"a\rb\rc");
        assert_eq!(paste_bytes("ls", true), b"\x1b[200~ls\x1b[201~");
        assert_eq!(paste_bytes("x\x1b[201~y", true), b"\x1b[200~xy\x1b[201~");
    }

    #[test]
    fn shell_control_keys_keep_workspace_shortcuts_and_paste_payload_separate() {
        for ch in ["p", "b", "g", "`", "中文", ""] {
            assert_eq!(
                key_bytes(TerminalKey::Character(ch), Modifiers::CONTROL, false),
                None
            );
        }
        assert_eq!(
            key_bytes(TerminalKey::Character("K"), Modifiers::CONTROL, false),
            Some(vec![11])
        );
        assert_eq!(
            key_bytes(TerminalKey::Character("a"), Modifiers::empty(), false),
            None
        );
        assert_eq!(
            key_bytes(TerminalKey::Named(NamedKey::Tab), Modifiers::SHIFT, false),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(
            key_bytes(
                TerminalKey::Named(NamedKey::Backspace),
                Modifiers::ALT,
                false
            ),
            Some(b"\x1b\x7f".to_vec())
        );
    }
}
