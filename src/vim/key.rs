//! Keys as the Vim layer sees them, and Vim's `<Esc>`-style key notation.

use std::fmt;

use crate::input::keystroke::Keystroke;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A printable character, including space.
    Char(char),
    /// Ctrl with a letter or punctuation key, e.g. `<C-r>`.
    Ctrl(char),
    Esc,
    Enter,
    Tab,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
}

impl Key {
    /// The Vim key for a keystroke, or `None` for chords Vim leaves alone
    /// (Alt, Cmd, shifted named keys).
    pub fn from_keystroke(keystroke: &Keystroke) -> Option<Self> {
        let modifiers = keystroke.modifiers;
        if modifiers.alt || modifiers.platform {
            return None;
        }
        if modifiers.ctrl {
            let mut chars = keystroke.key.chars();
            let character = chars.next()?;
            if chars.next().is_some() || modifiers.shift {
                return None;
            }
            return Some(if character == '[' {
                Key::Esc
            } else {
                Key::Ctrl(character)
            });
        }
        if let Some(text) = keystroke.text() {
            let mut chars = text.chars();
            let character = chars.next()?;
            return chars.next().is_none().then_some(Key::Char(character));
        }
        if modifiers.shift {
            return None;
        }
        Some(match keystroke.key.as_str() {
            "escape" => Key::Esc,
            "enter" => Key::Enter,
            "tab" => Key::Tab,
            "backspace" => Key::Backspace,
            "delete" => Key::Delete,
            "left" => Key::Left,
            "right" => Key::Right,
            "up" => Key::Up,
            "down" => Key::Down,
            "home" => Key::Home,
            "end" => Key::End,
            "pageup" => Key::PageUp,
            "pagedown" => Key::PageDown,
            "insert" => Key::Insert,
            _ => return None,
        })
    }

    /// Parse Vim key notation such as `d2w`, `ciw<Esc>` or `<C-r>`.
    pub fn parse(source: &str) -> Vec<Key> {
        let mut keys = Vec::new();
        let mut rest = source;
        while let Some(character) = rest.chars().next() {
            if character == '<'
                && let Some(end) = rest.find('>')
                && let Some(key) = Self::named(&rest[1..end])
            {
                keys.push(key);
                rest = &rest[end + 1..];
                continue;
            }
            keys.push(Key::Char(character));
            rest = &rest[character.len_utf8()..];
        }
        keys
    }

    fn named(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("c-") {
            let mut chars = rest.chars();
            let character = chars.next()?;
            return chars.next().is_none().then_some(if character == '[' {
                Key::Esc
            } else {
                Key::Ctrl(character)
            });
        }
        Some(match lower.as_str() {
            "esc" => Key::Esc,
            "cr" | "enter" | "return" => Key::Enter,
            "tab" => Key::Tab,
            "bs" => Key::Backspace,
            "del" => Key::Delete,
            "left" => Key::Left,
            "right" => Key::Right,
            "up" => Key::Up,
            "down" => Key::Down,
            "home" => Key::Home,
            "end" => Key::End,
            "pageup" => Key::PageUp,
            "pagedown" => Key::PageDown,
            "insert" => Key::Insert,
            "space" => Key::Char(' '),
            "lt" => Key::Char('<'),
            "bar" => Key::Char('|'),
            "bslash" => Key::Char('\\'),
            _ => return None,
        })
    }
}

impl Key {
    /// Notation that `Key::parse` reads back, for recorded macros.
    pub fn notation(&self) -> String {
        match self {
            Key::Ctrl(character) => format!("<C-{character}>"),
            Key::Char(' ') => " ".to_owned(),
            other => other.to_string(),
        }
    }
}

/// Vim notation, used to show pending keys.
impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Char(' ') => f.write_str("<Space>"),
            Key::Char('<') => f.write_str("<lt>"),
            Key::Char(character) => write!(f, "{character}"),
            Key::Ctrl(character) => write!(f, "^{}", character.to_ascii_uppercase()),
            Key::Esc => f.write_str("<Esc>"),
            Key::Enter => f.write_str("<CR>"),
            Key::Tab => f.write_str("<Tab>"),
            Key::Backspace => f.write_str("<BS>"),
            Key::Delete => f.write_str("<Del>"),
            Key::Left => f.write_str("<Left>"),
            Key::Right => f.write_str("<Right>"),
            Key::Up => f.write_str("<Up>"),
            Key::Down => f.write_str("<Down>"),
            Key::Home => f.write_str("<Home>"),
            Key::End => f.write_str("<End>"),
            Key::PageUp => f.write_str("<PageUp>"),
            Key::PageDown => f.write_str("<PageDown>"),
            Key::Insert => f.write_str("<Insert>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notation_parses_characters_and_named_keys() {
        assert_eq!(
            Key::parse("d2w<Esc><C-r><lt>x<CR>"),
            [
                Key::Char('d'),
                Key::Char('2'),
                Key::Char('w'),
                Key::Esc,
                Key::Ctrl('r'),
                Key::Char('<'),
                Key::Char('x'),
                Key::Enter,
            ]
        );
        assert_eq!(
            Key::parse("a<b"),
            [Key::Char('a'), Key::Char('<'), Key::Char('b')]
        );
    }

    #[test]
    fn keystrokes_map_to_vim_keys() {
        let key = |source: &str| Key::from_keystroke(&Keystroke::parse(source).unwrap());
        assert_eq!(key("G"), Some(Key::Char('G')));
        assert_eq!(key("$"), Some(Key::Char('$')));
        assert_eq!(key("ctrl-r"), Some(Key::Ctrl('r')));
        assert_eq!(key("ctrl-["), Some(Key::Esc));
        assert_eq!(key("escape"), Some(Key::Esc));
        assert_eq!(key("space"), Some(Key::Char(' ')));
        assert_eq!(key("alt-x"), None);
        assert_eq!(key("shift-left"), None);
    }
}
