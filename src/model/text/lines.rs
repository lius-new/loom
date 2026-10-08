//! Line starts and display widths, kept up to date across edits.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::change::ChangeSet;

/// Display columns a tab advances to the next multiple of.
pub const TAB_WIDTH: usize = 4;

/// Display width of one grapheme starting at display column `column`.
pub fn grapheme_width(grapheme: &str, column: usize) -> usize {
    if grapheme == "\t" {
        TAB_WIDTH - column % TAB_WIDTH
    } else {
        UnicodeWidthStr::width(grapheme)
    }
}

/// Display columns `s` occupies when it starts at column 0.
pub fn display_width(s: &str) -> usize {
    s.graphemes(true).fold(0, |column, grapheme| {
        column + grapheme_width(grapheme, column)
    })
}

/// The offset after a line's content, before its CR LF or LF.
fn content_end(text: &str, start: usize, next_start: Option<usize>) -> usize {
    match next_start {
        None => text.len(),
        Some(next) => {
            let end = next - 1;
            if end > start && text.as_bytes()[end - 1] == b'\r' {
                end - 1
            } else {
                end
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LineIndex {
    starts: Vec<usize>,
    widths: Vec<u32>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut index = Self {
            starts: Vec::new(),
            widths: Vec::new(),
        };
        index.scan(text, 0, text.len(), true);
        index
    }

    /// Append the lines starting in `text[from..to]`. `to_end` says whether
    /// `to` is the end of the text, so a final line break opens a last line.
    fn scan(&mut self, text: &str, from: usize, to: usize, to_end: bool) {
        let mut starts = vec![from];
        for (i, byte) in text.as_bytes()[from..to].iter().enumerate() {
            if *byte == b'\n' && (to_end || from + i + 1 < to) {
                starts.push(from + i + 1);
            }
        }
        let next_after_region = (!to_end).then_some(to);
        for (i, &start) in starts.iter().enumerate() {
            let next = starts.get(i + 1).copied().or(next_after_region);
            let end = content_end(text, start, next);
            self.starts.push(start);
            self.widths.push(display_width(&text[start..end]) as u32);
        }
    }

    pub fn len(&self) -> usize {
        self.starts.len()
    }

    pub fn start(&self, line: usize) -> usize {
        self.starts[line]
    }

    /// The start of the following line, if there is one.
    pub fn next_start(&self, line: usize) -> Option<usize> {
        self.starts.get(line + 1).copied()
    }

    pub fn width(&self, line: usize) -> usize {
        self.widths[line] as usize
    }

    pub fn max_width(&self) -> usize {
        self.widths.iter().copied().max().unwrap_or(0) as usize
    }

    /// The line containing `offset`; a line break belongs to the line it ends.
    pub fn line_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|&start| start <= offset) - 1
    }

    pub fn content_end(&self, text: &str, line: usize) -> usize {
        content_end(text, self.starts[line], self.next_start(line))
    }

    /// Rebuild the lines `changes` touched in `text`, the text after them.
    pub fn update(&mut self, text: &str, changes: &ChangeSet) {
        let (Some(first), Some(last)) = (changes.changes().first(), changes.changes().last())
        else {
            return;
        };
        let delta: isize = changes
            .changes()
            .iter()
            .map(|change| change.inserted.len() as isize - change.removed.len() as isize)
            .sum();
        let first_line = self.line_of(first.old.start);
        let last_line = self.line_of(last.old.end);
        let region_start = self.starts[first_line];
        let following = self.next_start(last_line);
        let to_end = following.is_none();
        let region_end = following.map_or(text.len(), |next| (next as isize + delta) as usize);

        let mut rebuilt = Self {
            starts: Vec::new(),
            widths: Vec::new(),
        };
        rebuilt.scan(text, region_start, region_end, to_end);
        let tail = last_line + 1;
        for start in &mut self.starts[tail..] {
            *start = (*start as isize + delta) as usize;
        }
        self.starts.splice(first_line..tail, rebuilt.starts);
        self.widths.splice(first_line..tail, rebuilt.widths);
    }
}

#[cfg(test)]
mod tests {
    use super::super::buffer::apply;
    use super::super::change::{Edit, validate};
    use super::*;

    #[test]
    fn lines_split_on_lf_and_crlf_and_measure_tabs_and_wide_characters() {
        let text = "a\tb\r\n中文\n\nend\n";
        let index = LineIndex::new(text);
        assert_eq!(index.starts, [0, 5, 12, 13, 17]);
        assert_eq!(index.widths, [5, 4, 0, 3, 0]);
        assert_eq!(index.content_end(text, 0), 3);
        assert_eq!(index.line_of(4), 0, "the LF belongs to the line it ends");
        assert_eq!(index.line_of(5), 1);
        assert_eq!(index.line_of(17), 4);
    }

    /// Every incremental update must equal a fresh scan of the result.
    #[test]
    fn incremental_updates_match_a_full_rebuild() {
        let mut text = String::from("one\ntwo\r\nthree\n\tfour\nfive");
        let mut index = LineIndex::new(&text);
        let mut seed = 7u64;
        let mut next = |bound: usize| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) as usize) % bound.max(1)
        };
        let pieces = ["", "x", "\n", "\r\n", "a\nb", "\t", "中\n\n", "end"];
        for _ in 0..500 {
            let mut edits = Vec::new();
            let mut cursor = 0;
            while cursor < text.len() && edits.len() < 3 {
                let start = cursor + next(text.len() - cursor + 1);
                let end = (start + next(6)).min(text.len());
                if start > text.len() {
                    break;
                }
                let (start, end) = (floor(&text, start), floor(&text, end));
                edits.push(Edit::replace(start..end, pieces[next(pieces.len())]));
                cursor = end + 1;
            }
            let Ok(edits) = validate(&text, edits) else {
                continue;
            };
            let changes = apply(&mut text, edits);
            index.update(&text, &changes);
            assert_eq!(index, LineIndex::new(&text), "after edits to {text:?}");
        }
    }

    fn floor(text: &str, mut offset: usize) -> usize {
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }
}
