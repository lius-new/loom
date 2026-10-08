//! Text objects: words, quoted strings, bracket blocks and paragraphs.

use std::ops::Range;

use crate::model::text::TextBuffer;

use super::motion::{class, find_unmatched};

/// The range a text object covers, and whether it is made of whole lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selected {
    pub range: Range<usize>,
    pub linewise: bool,
}

impl Selected {
    fn chars(range: Range<usize>) -> Self {
        Self {
            range,
            linewise: false,
        }
    }
}

/// `iw`, `aw`, `iW`, `aW` on the cursor's line.
pub fn word(
    buffer: &TextBuffer,
    offset: usize,
    count: usize,
    around: bool,
    big: bool,
) -> Option<Selected> {
    let line = buffer.line_range(buffer.line_of(offset));
    if line.is_empty() {
        return None;
    }
    let offset = offset.min(buffer.prev_grapheme(line.end));
    let class_at = |at: usize| {
        if at >= line.end {
            0
        } else {
            class(buffer, at, big)
        }
    };
    let run_end = |mut at: usize| {
        let run = class_at(at);
        while at < line.end && class_at(at) == run {
            at = buffer.next_grapheme(at);
        }
        at
    };
    let own = class_at(offset);
    let mut start = offset;
    while start > line.start && class_at(buffer.prev_grapheme(start)) == own {
        start = buffer.prev_grapheme(start);
    }
    let mut end = run_end(offset);
    if around {
        if own == 0 || (end < line.end && class_at(end) == 0) {
            // Blanks and the following word, or a word and its trailing blanks.
            end = run_end(end);
        } else {
            while start > line.start && class_at(buffer.prev_grapheme(start)) == 0 {
                start = buffer.prev_grapheme(start);
            }
        }
    }
    for _ in 1..count {
        if end >= line.end {
            break;
        }
        end = run_end(end);
        if around && end < line.end && class_at(end) == 0 {
            end = run_end(end);
        }
    }
    Some(Selected::chars(start..end))
}

/// `i"`, `a"` and the other quotes, within the cursor's line.
pub fn quote(buffer: &TextBuffer, offset: usize, quote: char, around: bool) -> Option<Selected> {
    let line = buffer.line_range(buffer.line_of(offset));
    let text = &buffer.text()[line.clone()];
    let mut quotes = Vec::new();
    let mut escaped = false;
    for (index, c) in text.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == quote {
            quotes.push(line.start + index);
        }
    }
    let (open, close) = quotes
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .find(|&(open, close)| offset >= open && offset <= close)
        .or_else(|| {
            quotes
                .chunks_exact(2)
                .map(|pair| (pair[0], pair[1]))
                .find(|&(open, _)| open > offset)
        })?;
    let quote_len = quote.len_utf8();
    if !around {
        return Some(Selected::chars(open + quote_len..close));
    }
    let mut start = open;
    let mut end = close + quote_len;
    let blank = |at: usize| matches!(buffer.text()[at..].chars().next(), Some(' ' | '\t'));
    if end < line.end && blank(end) {
        while end < line.end && blank(end) {
            end += 1;
        }
    } else {
        while start > line.start && blank(start - 1) {
            start -= 1;
        }
    }
    Some(Selected::chars(start..end))
}

/// `i(`, `a(` and the other bracket blocks around the cursor. An inner block
/// spanning lines covers only the lines between the brackets.
pub fn bracket(
    buffer: &TextBuffer,
    offset: usize,
    open: char,
    close: char,
    count: usize,
    around: bool,
) -> Option<Selected> {
    let text = buffer.text();
    let at = |offset: usize| text[offset..].chars().next();
    let mut start = if at(offset) == Some(open) {
        offset
    } else if at(offset) == Some(close) {
        find_unmatched(text, offset, open, close, false)?
    } else {
        enclosing_open(text, offset, open, close)?
    };
    for _ in 1..count {
        start = enclosing_open(text, start, open, close)?;
    }
    let end = find_unmatched(text, start, open, close, true)?;
    if around {
        return Some(Selected::chars(start..end + close.len_utf8()));
    }
    let mut inner_start = start + open.len_utf8();
    let mut inner_end = end;
    let after_open = buffer.line_of(inner_start);
    let close_line = buffer.line_of(end);
    let opens_line =
        inner_start == buffer.line_end(buffer.line_of(start)) && after_open < close_line;
    let closes_line = close_line > buffer.line_of(start)
        && text[buffer.line_start(close_line)..end]
            .chars()
            .all(|c| c == ' ' || c == '\t');
    if opens_line {
        inner_start = buffer.line_start(buffer.line_of(start) + 1);
    }
    if closes_line {
        inner_end = buffer.line_start(close_line);
    }
    Some(Selected {
        linewise: opens_line && closes_line && inner_start < inner_end,
        range: inner_start..inner_end.max(inner_start),
    })
}

/// The unmatched `open` before `offset`, skipping nested pairs.
fn enclosing_open(text: &str, offset: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    for (index, c) in text[..offset].char_indices().rev() {
        if c == close {
            depth += 1;
        } else if c == open {
            if depth == 0 {
                return Some(index);
            }
            depth -= 1;
        }
    }
    None
}

/// `ip` and `ap`: runs of non-blank (or blank) lines; `ap` adds the blank
/// lines after the paragraph, or before it at the end of the text.
pub fn paragraph(
    buffer: &TextBuffer,
    offset: usize,
    count: usize,
    around: bool,
) -> Option<Selected> {
    let blank = |line: usize| buffer.line_range(line).is_empty();
    let last = buffer.line_count() - 1;
    let run_end = |line: usize| {
        let mut end = line;
        while end < last && blank(end + 1) == blank(line) {
            end += 1;
        }
        end
    };
    let line = buffer.line_of(offset);
    let mut first = line;
    while first > 0 && blank(first - 1) == blank(line) {
        first -= 1;
    }
    let mut end = run_end(line);
    let mut runs = if around { 2 } else { 1 };
    runs += (count - 1) * if around { 2 } else { 1 };
    runs -= 1;
    let mut extended = false;
    while runs > 0 && end < last {
        end = run_end(end + 1);
        runs -= 1;
        extended = true;
    }
    if around && !extended && !blank(line) {
        while first > 0 && blank(first - 1) {
            first -= 1;
        }
    }
    Some(Selected {
        range: buffer.line_start(first)..buffer.line_range_with_break(end).end,
        linewise: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(buffer: &TextBuffer, selected: Option<Selected>) -> Option<String> {
        selected.map(|selected| buffer.text()[selected.range].to_owned())
    }

    #[test]
    fn words_take_trailing_or_leading_blanks() {
        let buffer = TextBuffer::new("call foo.bar  baz");
        let word =
            |offset, count, around| text_of(&buffer, word(&buffer, offset, count, around, false));
        assert_eq!(word(6, 1, false).as_deref(), Some("foo"));
        assert_eq!(
            word(6, 1, true).as_deref(),
            Some(" foo"),
            "no trailing blank"
        );
        assert_eq!(word(0, 1, true).as_deref(), Some("call "));
        assert_eq!(word(15, 1, true).as_deref(), Some("  baz"));
        assert_eq!(word(12, 1, false).as_deref(), Some("  "));
        assert_eq!(word(6, 3, false).as_deref(), Some("foo.bar"));
        assert_eq!(
            text_of(&buffer, super::word(&buffer, 6, 1, false, true)).as_deref(),
            Some("foo.bar")
        );
    }

    #[test]
    fn quotes_pair_up_from_the_line_start() {
        let buffer = TextBuffer::new(r#"x = "a \"b\"" + "c";"#);
        let quote = |offset, around| text_of(&buffer, quote(&buffer, offset, '"', around));
        assert_eq!(quote(6, false).as_deref(), Some(r#"a \"b\""#));
        assert_eq!(
            quote(0, false).as_deref(),
            Some(r#"a \"b\""#),
            "the next string"
        );
        assert_eq!(quote(17, false).as_deref(), Some("c"));
        assert_eq!(
            quote(17, true).as_deref(),
            Some(r#" "c""#),
            "leading blank at the end"
        );
        assert_eq!(quote(4, true).as_deref(), Some(r#""a \"b\"" "#));
    }

    #[test]
    fn brackets_nest_and_inner_blocks_drop_their_bracket_lines() {
        let buffer = TextBuffer::new("f(a, (b), c)");
        let block = |offset, count, around| {
            text_of(&buffer, bracket(&buffer, offset, '(', ')', count, around))
        };
        assert_eq!(block(3, 1, false).as_deref(), Some("a, (b), c"));
        assert_eq!(block(6, 1, false).as_deref(), Some("b"));
        assert_eq!(block(6, 2, true).as_deref(), Some("(a, (b), c)"));
        assert_eq!(
            block(5, 1, false).as_deref(),
            Some("b"),
            "on the open bracket"
        );
        assert_eq!(block(0, 1, false), None);

        let buffer = TextBuffer::new("fn x() {\n    body;\n    more;\n}");
        let inner = bracket(&buffer, 12, '{', '}', 1, false).unwrap();
        assert!(inner.linewise);
        assert_eq!(&buffer.text()[inner.range], "    body;\n    more;\n");
    }

    #[test]
    fn paragraphs_are_line_runs() {
        let buffer = TextBuffer::new("a\nb\n\n\nc\n\nd");
        let paragraph =
            |offset, count, around| text_of(&buffer, paragraph(&buffer, offset, count, around));
        assert_eq!(paragraph(0, 1, false).as_deref(), Some("a\nb\n"));
        assert_eq!(paragraph(0, 1, true).as_deref(), Some("a\nb\n\n\n"));
        assert_eq!(paragraph(4, 1, false).as_deref(), Some("\n\n"));
        assert_eq!(paragraph(0, 2, false).as_deref(), Some("a\nb\n\n\n"));
        assert_eq!(
            paragraph(10, 1, true).as_deref(),
            Some("\nd"),
            "blank lines before the last"
        );
    }
}
