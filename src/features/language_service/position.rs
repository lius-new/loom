//! Positions between the editor's byte offsets and the protocol's
//! `{ line, character }`, in whichever encoding the server chose.
//!
//! The functions take a line's text (without its break), so they stay free of the
//! buffer type; `TextBuffer` callers pass `buffer.line(n)`.
use super::capabilities::PositionEncoding;

/// A zero-based protocol position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

/// The `character` of a byte offset inside a line. An offset inside a character
/// counts as the start of that character; one past the end counts as the end.
pub fn column_of(line: &str, byte: usize, encoding: PositionEncoding) -> u32 {
    let mut byte = byte.min(line.len());
    while !line.is_char_boundary(byte) {
        byte -= 1;
    }
    let column = match encoding {
        PositionEncoding::Utf8 => byte,
        PositionEncoding::Utf16 => line[..byte].encode_utf16().count(),
    };
    u32::try_from(column).unwrap_or(u32::MAX)
}

/// The byte offset of a `character` inside a line. A column past the end is the
/// end of the line, as the protocol asks; one inside a character (or between the
/// halves of a surrogate pair) rounds down to that character's start.
pub fn byte_of(line: &str, column: u32, encoding: PositionEncoding) -> usize {
    let column = column as usize;
    match encoding {
        PositionEncoding::Utf8 => {
            let mut byte = column.min(line.len());
            while !line.is_char_boundary(byte) {
                byte -= 1;
            }
            byte
        }
        PositionEncoding::Utf16 => {
            let mut units = 0;
            for (at, ch) in line.char_indices() {
                let next = units + ch.len_utf16();
                if next > column {
                    return at;
                }
                units = next;
            }
            line.len()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PositionEncoding::{Utf8, Utf16};
    use super::*;

    // 'a' 1 byte / 1 unit, 'é' 2 / 1, '中' 3 / 1, '😀' 4 / 2.
    const LINE: &str = "aé中😀z";

    #[test]
    fn columns_count_in_the_chosen_encoding() {
        let at = |needle: &str| LINE.find(needle).unwrap();
        for (needle, utf8, utf16) in [
            ("a", 0, 0),
            ("é", 1, 1),
            ("中", 3, 2),
            ("😀", 6, 3),
            ("z", 10, 5),
        ] {
            assert_eq!(column_of(LINE, at(needle), Utf8), utf8, "{needle}");
            assert_eq!(column_of(LINE, at(needle), Utf16), utf16, "{needle}");
            assert_eq!(byte_of(LINE, utf8, Utf8), at(needle), "{needle}");
            assert_eq!(byte_of(LINE, utf16, Utf16), at(needle), "{needle}");
        }
        assert_eq!(column_of(LINE, LINE.len(), Utf8), 11);
        assert_eq!(column_of(LINE, LINE.len(), Utf16), 6);
    }

    #[test]
    fn offsets_inside_a_character_round_down() {
        assert_eq!(column_of(LINE, 2, Utf8), 1, "inside é");
        assert_eq!(column_of(LINE, 8, Utf16), 3, "inside the emoji");
        assert_eq!(byte_of(LINE, 2, Utf8), 1);
        // Between the two halves of the emoji's surrogate pair.
        assert_eq!(byte_of(LINE, 4, Utf16), 6);
        assert_eq!(byte_of(LINE, 8, Utf8), 6);
    }

    #[test]
    fn columns_past_the_end_clamp_to_the_end() {
        assert_eq!(byte_of(LINE, 999, Utf8), LINE.len());
        assert_eq!(byte_of(LINE, 999, Utf16), LINE.len());
        assert_eq!(byte_of("", 3, Utf16), 0);
        assert_eq!(column_of(LINE, 999, Utf16), 6);
        assert_eq!(column_of("", 0, Utf8), 0);
    }

    #[test]
    fn every_boundary_round_trips() {
        for encoding in [Utf8, Utf16] {
            for (byte, _) in LINE.char_indices().chain([(LINE.len(), ' ')]) {
                let column = column_of(LINE, byte, encoding);
                assert_eq!(byte_of(LINE, column, encoding), byte, "{encoding:?} {byte}");
            }
        }
    }
}
