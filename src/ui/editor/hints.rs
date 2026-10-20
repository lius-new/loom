//! Inlay hints: text a language server asks to be shown inside a line, such as
//! the type of a variable or the name of a parameter. The hint is not in the
//! document. The line is laid out with the hint text in it so that everything
//! after the hint moves right; the caret, selections, underlines and clicks all
//! follow from that layout, and positions in the document stay as they are.
use std::cell::RefCell;
use std::ops::Range;
use std::sync::Arc;

/// Text shown before the character at `offset` (a byte offset in the document).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Hint {
    pub offset: usize,
    pub text: String,
}

thread_local! {
    /// The hints of the document last drawn, by the identity of its text, so that
    /// finding the caret or the character under the pointer sees the same layout
    /// as drawing did.
    static DRAWN: RefCell<Option<(usize, Arc<[Hint]>)>> = const { RefCell::new(None) };
}

/// Remembers the hints drawn for the document whose text has `identity`.
pub(crate) fn publish(identity: usize, hints: &Arc<[Hint]>) {
    DRAWN.with(|drawn| {
        *drawn.borrow_mut() = (!hints.is_empty()).then(|| (identity, hints.clone()));
    });
}

/// The hints drawn for the text with `identity`, if that is the text last drawn.
pub(crate) fn drawn(identity: usize) -> Option<Arc<[Hint]>> {
    DRAWN.with(|drawn| {
        drawn
            .borrow()
            .as_ref()
            .filter(|(known, _)| *known == identity)
            .map(|(_, hints)| hints.clone())
    })
}

/// The hints that fall in `window` of `text` (hints are sorted by offset), as
/// `(characters into the window, hint text)`. A hint at the window's end counts.
pub(crate) fn in_window<'a>(
    hints: &'a [Hint],
    text: &str,
    window: Range<usize>,
) -> Vec<(usize, &'a str)> {
    let first = hints.partition_point(|hint| hint.offset < window.start);
    hints[first..]
        .iter()
        .take_while(|hint| hint.offset <= window.end)
        .filter(|hint| text.is_char_boundary(hint.offset))
        .map(|hint| {
            (
                text[window.start..hint.offset].chars().count(),
                hint.text.as_str(),
            )
        })
        .collect()
}

/// What `decorate` makes of a line.
pub(crate) struct Decorated {
    pub text: String,
    /// For every character index of the line (and one past its end), where that
    /// character is in `text`, in characters.
    pub positions: Vec<usize>,
    /// Where each hint starts in `text` and how many characters it has.
    pub hints: Vec<(usize, usize)>,
}

/// `line` with each hint inserted before the character it belongs to.
pub(crate) fn decorate(line: &str, hints: &[(usize, &str)]) -> Decorated {
    let count = line.chars().count();
    let mut out =
        String::with_capacity(line.len() + hints.iter().map(|(_, t)| t.len()).sum::<usize>());
    let mut positions = Vec::with_capacity(count + 1);
    let mut starts = Vec::with_capacity(hints.len());
    let mut inserted = 0;
    let mut next = 0;
    let mut chars = line.chars();
    for index in 0..=count {
        while next < hints.len() && hints[next].0 == index {
            let length = hints[next].1.chars().count();
            starts.push((index + inserted, length));
            out.push_str(hints[next].1);
            inserted += length;
            next += 1;
        }
        positions.push(index + inserted);
        if let Some(c) = chars.next() {
            out.push(c);
        }
    }
    Decorated {
        text: out,
        positions,
        hints: starts,
    }
}

/// Splits spans of `(text, style)` so that no span crosses a hint position.
pub(crate) fn split_spans<T: Clone>(
    spans: Vec<(String, T)>,
    hint_positions: &[usize],
) -> Vec<(String, T)> {
    if hint_positions.is_empty() {
        return spans;
    }
    let mut out = Vec::with_capacity(spans.len() + hint_positions.len());
    let mut at = 0;
    for (text, style) in spans {
        let length = text.chars().count();
        let mut start = 0;
        for &position in hint_positions {
            if position > at && position < at + length && position - at > start {
                let cut = position - at;
                out.push((
                    text.chars().skip(start).take(cut - start).collect(),
                    style.clone(),
                ));
                start = cut;
            }
        }
        out.push((text.chars().skip(start).collect(), style));
        at += length;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(offset: usize, text: &str) -> Hint {
        Hint {
            offset,
            text: text.into(),
        }
    }

    #[test]
    fn hints_in_a_window_are_counted_in_characters_from_its_start() {
        let text = "ab\nécd\n";
        let hints = [hint(1, ": T"), hint(5, "x:"), hint(9, "late")];
        // Line 1 is bytes 3..7; the hint at 5 is after 'é' (two bytes): one character.
        assert_eq!(in_window(&hints, text, 3..7), [(1, "x:")]);
        assert_eq!(in_window(&hints, text, 0..2), [(1, ": T")]);
        assert!(in_window(&hints, text, 7..7).is_empty());
        // At the window's end still counts: a hint after the last character.
        assert_eq!(in_window(&[hint(2, "end")], "ab", 0..2), [(2, "end")]);
        // A hint inside a character is ignored, not a panic.
        assert!(in_window(&[hint(4, "bad")], text, 3..7).is_empty());
    }

    #[test]
    fn decorating_puts_the_hint_before_its_character_and_maps_every_position() {
        let made = decorate("abc", &[(1, "[x]"), (3, "!")]);
        assert_eq!(made.text, "a[x]bc!");
        // Characters 0 to 3 and the end: the caret before 'b' is after the hint.
        assert_eq!(made.positions, [0, 4, 5, 7]);
        assert_eq!(made.hints, [(1, 3), (6, 1)]);
        let plain = decorate("abc", &[]);
        assert_eq!(plain.text, "abc");
        assert_eq!(plain.positions, [0, 1, 2, 3]);
        let front = decorate("ab", &[(0, "p:")]);
        assert_eq!(front.text, "p:ab");
        assert_eq!(front.positions, [2, 3, 4]);
    }

    #[test]
    fn two_hints_in_one_place_keep_their_order() {
        let made = decorate("a", &[(1, "x"), (1, "y")]);
        assert_eq!(made.text, "axy");
        assert_eq!(made.positions, [0, 3]);
        assert_eq!(made.hints, [(1, 1), (2, 1)]);
    }

    #[test]
    fn spans_are_cut_where_a_hint_goes() {
        let spans = vec![("hello".to_owned(), 'a'), ("world".to_owned(), 'b')];
        let cut = split_spans(spans.clone(), &[2, 5, 7]);
        let texts: Vec<_> = cut.iter().map(|(t, c)| format!("{t}{c}")).collect();
        assert_eq!(texts, ["hea", "lloa", "wob", "rldb"]);
        assert_eq!(split_spans(spans.clone(), &[]), spans);
        // A hint at a span's own edge needs no cut.
        assert_eq!(split_spans(spans.clone(), &[0, 5, 10]), spans);
    }

    #[test]
    fn the_drawn_hints_belong_to_one_text() {
        let hints: Arc<[Hint]> = vec![hint(0, "x")].into();
        publish(7, &hints);
        assert!(drawn(7).is_some());
        assert!(drawn(8).is_none());
        publish(7, &Arc::from(Vec::new()));
        assert!(drawn(7).is_none());
    }
}
