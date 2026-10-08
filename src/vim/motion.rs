//! Cursor motions on the text, following Vim's rules for words, lines,
//! paragraphs, character finds and bracket matching.
//!
//! Positions are grapheme boundaries. As in Vim, the position at the end of a
//! line (on its line break, or at the end of the text) is the line's "NUL"
//! position: reachable by some motions, but not a resting place in Normal mode.

use crate::model::text::TextBuffer;

use super::command::FindKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionKind {
    Exclusive,
    Inclusive,
    Linewise,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Moved {
    pub offset: usize,
    pub kind: MotionKind,
    /// Display column to keep for following vertical moves (`usize::MAX`
    /// sticks to line ends); `None` keeps whatever the motion does not set.
    pub column: Option<usize>,
}

impl Moved {
    pub fn exclusive(offset: usize) -> Self {
        Self {
            offset,
            kind: MotionKind::Exclusive,
            column: None,
        }
    }

    pub fn inclusive(offset: usize) -> Self {
        Self {
            offset,
            kind: MotionKind::Inclusive,
            column: None,
        }
    }

    pub fn linewise(offset: usize) -> Self {
        Self {
            offset,
            kind: MotionKind::Linewise,
            column: None,
        }
    }
}

/// Vim's character classes: blanks (including line ends), punctuation and
/// word characters. Big words only tell blanks from everything else.
pub fn class(buffer: &TextBuffer, offset: usize, big: bool) -> u8 {
    let Some(grapheme) = buffer.grapheme_at(offset) else {
        return 0;
    };
    let first = grapheme.chars().next().unwrap_or(' ');
    if first.is_whitespace() {
        0
    } else if big || first.is_alphanumeric() || first == '_' {
        2
    } else {
        1
    }
}

pub fn is_line_end(buffer: &TextBuffer, offset: usize) -> bool {
    offset == buffer.line_end(buffer.line_of(offset))
}

fn is_empty_line_start(buffer: &TextBuffer, offset: usize) -> bool {
    let line = buffer.line_of(offset);
    offset == buffer.line_start(line) && buffer.line_start(line) == buffer.line_end(line)
}

/// Step forward one position, crossing line ends. Returns `None` at the end
/// of the text, `Some(1)` after moving to the next line, `Some(2)` after
/// moving onto a line end and `Some(0)` otherwise.
fn inc(buffer: &TextBuffer, offset: &mut usize) -> Option<u8> {
    let line = buffer.line_of(*offset);
    if *offset >= buffer.line_end(line) {
        if line + 1 >= buffer.line_count() {
            return None;
        }
        *offset = buffer.line_start(line + 1);
        return Some(1);
    }
    *offset = buffer.next_grapheme(*offset).min(buffer.line_end(line));
    Some(if *offset == buffer.line_end(line) {
        2
    } else {
        0
    })
}

/// Step back one position. Returns `None` at the start of the text and
/// `Some(1)` after moving onto the previous line's end.
fn dec(buffer: &TextBuffer, offset: &mut usize) -> Option<u8> {
    let line = buffer.line_of(*offset);
    if *offset <= buffer.line_start(line) {
        if line == 0 {
            return None;
        }
        *offset = buffer.line_end(line - 1);
        return Some(1);
    }
    *offset = buffer.prev_grapheme(*offset);
    Some(0)
}

/// `w`/`W`. With `operator`, the last word stops at its line's end instead of
/// continuing on the next line (Vim's special case for `dw`).
pub fn word_forward(
    buffer: &TextBuffer,
    mut offset: usize,
    count: usize,
    big: bool,
    operator: bool,
) -> usize {
    for remaining in (0..count).rev() {
        let start_class = class(buffer, offset, big);
        let last_line = buffer.line_of(offset) + 1 == buffer.line_count();
        match inc(buffer, &mut offset) {
            None => return offset,
            Some(step) if step >= 1 && last_line => return offset,
            Some(step) if step >= 1 && operator && remaining == 0 => return offset,
            _ => {}
        }
        if start_class != 0 {
            while class(buffer, offset, big) == start_class && !is_line_end(buffer, offset) {
                match inc(buffer, &mut offset) {
                    None => return offset,
                    Some(step) if step >= 1 && operator && remaining == 0 => return offset,
                    _ => {}
                }
            }
        }
        while class(buffer, offset, big) == 0 {
            if is_empty_line_start(buffer, offset) {
                break;
            }
            match inc(buffer, &mut offset) {
                None => return offset,
                Some(step) if step >= 1 && operator && remaining == 0 => return offset,
                _ => {}
            }
        }
    }
    offset
}

/// `e`/`E`. With `stop`, a cursor already on a word's end stays there for
/// the first count (how `cw` changes to the end of the current word).
pub fn word_end(
    buffer: &TextBuffer,
    mut offset: usize,
    count: usize,
    big: bool,
    mut stop: bool,
) -> usize {
    for _ in 0..count {
        let start_class = class(buffer, offset, big);
        if inc(buffer, &mut offset).is_none() {
            return offset;
        }
        if class(buffer, offset, big) == start_class
            && start_class != 0
            && !is_line_end(buffer, offset)
        {
            skip_class_forward(buffer, &mut offset, start_class, big);
        } else if !stop || start_class == 0 {
            while class(buffer, offset, big) == 0 {
                if inc(buffer, &mut offset).is_none() {
                    return offset;
                }
            }
            let word_class = class(buffer, offset, big);
            skip_class_forward(buffer, &mut offset, word_class, big);
        }
        dec(buffer, &mut offset);
        stop = false;
    }
    offset
}

/// Move past every position of class `class`, landing on the first one that
/// is not (possibly a line end).
fn skip_class_forward(buffer: &TextBuffer, offset: &mut usize, class_id: u8, big: bool) {
    while class(buffer, *offset, big) == class_id && !is_line_end(buffer, *offset) {
        if inc(buffer, offset).is_none() {
            return;
        }
    }
}

/// `b`/`B`.
pub fn word_back(buffer: &TextBuffer, mut offset: usize, count: usize, big: bool) -> usize {
    'count: for _ in 0..count {
        if dec(buffer, &mut offset).is_none() {
            return offset;
        }
        while class(buffer, offset, big) == 0 {
            if is_empty_line_start(buffer, offset) {
                continue 'count;
            }
            if dec(buffer, &mut offset).is_none() {
                return offset;
            }
        }
        let word_class = class(buffer, offset, big);
        loop {
            let mut previous = offset;
            match dec(buffer, &mut previous) {
                Some(0) if class(buffer, previous, big) == word_class => offset = previous,
                _ => break,
            }
        }
    }
    offset
}

/// `ge`/`gE`.
pub fn word_end_back(buffer: &TextBuffer, mut offset: usize, count: usize, big: bool) -> usize {
    for _ in 0..count {
        let start_class = class(buffer, offset, big);
        if dec(buffer, &mut offset).is_none() {
            return offset;
        }
        if start_class != 0 {
            while class(buffer, offset, big) == start_class && !is_line_end(buffer, offset) {
                if dec(buffer, &mut offset).is_none() {
                    return offset;
                }
            }
        }
        while class(buffer, offset, big) == 0 {
            if is_empty_line_start(buffer, offset) {
                break;
            }
            if dec(buffer, &mut offset).is_none() {
                return offset;
            }
        }
    }
    offset
}

/// The first non-blank of `line`, or its end when it is blank.
pub fn first_non_blank(buffer: &TextBuffer, line: usize) -> usize {
    let range = buffer.line_range(line);
    buffer.text()[range.clone()]
        .char_indices()
        .find(|(_, c)| *c != ' ' && *c != '\t')
        .map_or(range.end, |(index, _)| range.start + index)
}

/// The start of the last grapheme on `line`, or its start when it is empty.
pub fn last_char(buffer: &TextBuffer, line: usize) -> usize {
    let range = buffer.line_range(line);
    if range.is_empty() {
        range.start
    } else {
        buffer.prev_grapheme(range.end).max(range.start)
    }
}

/// Where a Normal-mode cursor may rest: on a character, never past a line's
/// last character.
pub fn normal_position(buffer: &TextBuffer, offset: usize) -> usize {
    let offset = buffer.grapheme_floor(offset);
    let line = buffer.line_of(offset);
    offset.min(last_char(buffer, line))
}

/// The offset on `line` at display column `column` (sticking to the last
/// character for `usize::MAX`), as a Normal-mode position.
pub fn at_column(buffer: &TextBuffer, line: usize, column: usize) -> usize {
    normal_position(buffer, buffer.offset_at_display_column(line, column))
}

/// `f`, `F`, `t` and `T` within the current line.
pub fn find_in_line(
    buffer: &TextBuffer,
    offset: usize,
    kind: FindKind,
    target: char,
    count: usize,
    repeating: bool,
) -> Option<usize> {
    let line = buffer.line_range(buffer.line_of(offset));
    let text = buffer.text();
    let matches = |at: usize| text[at..].starts_with(target);
    let mut position = offset;
    let mut found = 0;
    match kind {
        FindKind::To | FindKind::Till => {
            // `;` after `t` must not stick in front of the same character.
            if kind == FindKind::Till && repeating {
                let next = buffer.next_grapheme(position);
                if next < line.end && matches(next) {
                    position = next;
                }
            }
            while found < count {
                position = buffer.next_grapheme(position);
                if position >= line.end {
                    return None;
                }
                if matches(position) {
                    found += 1;
                }
            }
            Some(if kind == FindKind::Till {
                buffer.prev_grapheme(position)
            } else {
                position
            })
        }
        FindKind::ToBack | FindKind::TillBack => {
            if kind == FindKind::TillBack && repeating && position > line.start {
                let previous = buffer.prev_grapheme(position);
                if previous > line.start && matches(buffer.prev_grapheme(previous)) {
                    position = previous;
                }
            }
            while found < count {
                if position <= line.start {
                    return None;
                }
                position = buffer.prev_grapheme(position);
                if matches(position) {
                    found += 1;
                }
            }
            Some(if kind == FindKind::TillBack {
                buffer.next_grapheme(position)
            } else {
                position
            })
        }
    }
}

const BRACKETS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

/// `%`: the bracket matching the first bracket at or after the cursor on its
/// line.
pub fn match_pair(buffer: &TextBuffer, offset: usize) -> Option<usize> {
    let line_end = buffer.line_end(buffer.line_of(offset));
    let text = buffer.text();
    let (at, bracket) = text[offset..line_end]
        .char_indices()
        .find(|(_, c)| BRACKETS.iter().any(|(open, close)| c == open || c == close))
        .map(|(index, c)| (offset + index, c))?;
    let (open, close, forward) = BRACKETS.iter().find_map(|&(open, close)| {
        if bracket == open {
            Some((open, close, true))
        } else if bracket == close {
            Some((open, close, false))
        } else {
            None
        }
    })?;
    find_unmatched(text, at, open, close, forward)
}

/// From the bracket at `at`, the offset of its partner in direction
/// `forward`, counting nesting.
pub fn find_unmatched(
    text: &str,
    at: usize,
    open: char,
    close: char,
    forward: bool,
) -> Option<usize> {
    let mut depth = 0usize;
    if forward {
        for (index, c) in text[at..].char_indices() {
            if c == open {
                depth += 1;
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    return Some(at + index);
                }
            }
        }
    } else {
        for (index, c) in text[..at + close.len_utf8()].char_indices().rev() {
            if c == close {
                depth += 1;
            } else if c == open {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
        }
    }
    None
}

fn is_blank_line(buffer: &TextBuffer, line: usize) -> bool {
    buffer.line_range(line).is_empty()
}

/// `}` and `{`: to the next or previous empty line after a non-empty one,
/// or to the end or start of the text.
pub fn paragraph(buffer: &TextBuffer, offset: usize, count: usize, forward: bool) -> Moved {
    let last = buffer.line_count() - 1;
    let mut line = buffer.line_of(offset);
    for _ in 0..count {
        let mut skipped = false;
        loop {
            if !is_blank_line(buffer, line) {
                skipped = true;
            }
            let next = if forward {
                (line < last).then(|| line + 1)
            } else {
                line.checked_sub(1)
            };
            let Some(next) = next else {
                break;
            };
            line = next;
            if skipped && is_blank_line(buffer, line) {
                break;
            }
        }
    }
    if forward && line == last && !is_blank_line(buffer, line) {
        // Past the last paragraph: the motion includes the final character.
        Moved::inclusive(last_char(buffer, line))
    } else {
        Moved::exclusive(buffer.line_start(line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str, marker: &str) -> usize {
        text.find(marker).unwrap()
    }

    #[test]
    fn words_follow_vim_classes_and_stop_on_empty_lines() {
        let buffer = TextBuffer::new("foo.bar  baz\n\n  qux");
        let text = buffer.text();
        assert_eq!(word_forward(&buffer, 0, 1, false, false), at(text, ".bar"));
        assert_eq!(word_forward(&buffer, 0, 1, true, false), at(text, "baz"));
        assert_eq!(
            word_forward(&buffer, at(text, "baz"), 1, false, false),
            13,
            "empty line"
        );
        assert_eq!(word_forward(&buffer, 13, 1, false, false), at(text, "qux"));
        assert_eq!(
            word_forward(&buffer, at(text, "baz"), 1, false, true),
            12,
            "dw stops at EOL"
        );
        assert_eq!(
            word_forward(&buffer, at(text, "qux"), 1, false, false),
            text.len()
        );

        assert_eq!(word_end(&buffer, 0, 1, false, false), 2);
        assert_eq!(word_end(&buffer, 2, 1, false, false), 3);
        assert_eq!(
            word_end(&buffer, 2, 1, false, true),
            2,
            "cw on a word's end"
        );
        assert_eq!(word_end(&buffer, at(text, "baz"), 1, false, false), 11);
        assert_eq!(
            word_end(&buffer, 11, 1, false, false),
            text.len() - 1,
            "across empty lines"
        );

        assert_eq!(word_back(&buffer, at(text, "qux"), 1, false), 13);
        assert_eq!(word_back(&buffer, 13, 1, false), at(text, "baz"));
        assert_eq!(
            word_back(&buffer, at(text, "baz"), 2, false),
            3,
            "punctuation is a word"
        );
        assert_eq!(word_end_back(&buffer, at(text, "baz"), 1, false), 6);
    }

    #[test]
    fn finds_stay_on_the_line_and_repeat_past_adjacent_targets() {
        let buffer = TextBuffer::new("a,b,c\nd,e");
        assert_eq!(
            find_in_line(&buffer, 0, FindKind::To, ',', 2, false),
            Some(3)
        );
        assert_eq!(
            find_in_line(&buffer, 0, FindKind::Till, ',', 1, false),
            Some(0)
        );
        assert_eq!(
            find_in_line(&buffer, 0, FindKind::Till, ',', 1, true),
            Some(2)
        );
        assert_eq!(find_in_line(&buffer, 0, FindKind::To, ',', 3, false), None);
        assert_eq!(
            find_in_line(&buffer, 4, FindKind::ToBack, ',', 1, false),
            Some(3)
        );
        assert_eq!(
            find_in_line(&buffer, 4, FindKind::TillBack, 'b', 1, false),
            Some(3)
        );
    }

    #[test]
    fn brackets_and_paragraphs() {
        let buffer = TextBuffer::new("f(a[1], (b)) x\n\n\npara\nend");
        assert_eq!(match_pair(&buffer, 0), Some(11));
        assert_eq!(match_pair(&buffer, 11), Some(1));
        assert_eq!(match_pair(&buffer, 3), Some(5));
        assert_eq!(match_pair(&buffer, 12), None);
        assert_eq!(paragraph(&buffer, 0, 1, true), Moved::exclusive(15));
        assert_eq!(paragraph(&buffer, 0, 2, true), Moved::inclusive(24));
        assert_eq!(paragraph(&buffer, 26, 1, false), Moved::exclusive(16));
    }

    #[test]
    fn normal_positions_never_rest_past_the_last_character() {
        let buffer = TextBuffer::new("ab\r\n\n中");
        assert_eq!(normal_position(&buffer, 2), 1);
        assert_eq!(normal_position(&buffer, 4), 4);
        assert_eq!(normal_position(&buffer, 8), 5);
        assert_eq!(at_column(&buffer, 0, usize::MAX), 1);
        assert_eq!(first_non_blank(&TextBuffer::new("  x"), 0), 2);
    }
}
