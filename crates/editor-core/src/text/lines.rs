//! Line starts and display widths, kept up to date across edits.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::change::ChangeSet;

/// Display columns a tab advances to the next multiple of.
pub const TAB_WIDTH: usize = 4;

/// Display width of one grapheme starting at display column `column`.
pub fn grapheme_width(grapheme: &str, column: usize) -> usize {
    grapheme_width_with_tab_width(grapheme, column, TAB_WIDTH)
}
pub fn grapheme_width_with_tab_width(grapheme: &str, column: usize, width: usize) -> usize {
    if grapheme == "\t" {
        width - column % width
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
pub(super) struct Position {
    pub byte: usize,
    pub chars: usize,
    pub columns: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LineIndex {
    starts: Vec<usize>,
    endings: Vec<bool>,
    crlf_count: usize,
    widths: Vec<u32>,
    positions: Vec<Vec<Position>>,
    max_width: usize,
    tab_width: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        Self::with_tab_width(text, TAB_WIDTH)
    }
    pub fn with_tab_width(text: &str, tab_width: usize) -> Self {
        let mut index = Self {
            tab_width,
            starts: Vec::new(),
            endings: Vec::new(),
            crlf_count: 0,
            widths: Vec::new(),
            positions: Vec::new(),
            max_width: 0,
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
            let crlf = next.is_some_and(|n| n >= 2 && text.as_bytes()[n - 2] == b'\r');
            self.endings.push(crlf);
            self.crlf_count += usize::from(crlf);
            self.starts.push(start);
            let line = &text[start..end];
            let mut positions: Vec<Position> = Vec::new();
            let mut columns = 0;
            let mut chars = 0;
            // Long lines have a sparse index for viewport and cursor lookups.
            // Never split a grapheme, including combining marks and emoji.
            for (byte, grapheme) in line.grapheme_indices(true) {
                if line.len() > 256
                    && (positions.is_empty() || byte - positions.last().unwrap().byte >= 256)
                {
                    positions.push(Position {
                        byte,
                        chars,
                        columns,
                    });
                }
                columns += grapheme_width_with_tab_width(grapheme, columns, self.tab_width);
                chars += grapheme.chars().count();
            }
            if !positions.is_empty() {
                positions.push(Position {
                    byte: line.len(),
                    chars,
                    columns,
                });
            }
            self.max_width = self.max_width.max(columns);
            self.widths.push(columns as u32);
            self.positions.push(positions);
        }
    }

    pub fn mixed_endings(&self) -> bool {
        self.crlf_count > 0 && self.crlf_count < self.starts.len().saturating_sub(1)
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
        self.max_width
    }

    pub fn position_by(
        &self,
        line: usize,
        key: usize,
        value: impl Fn(&Position) -> usize,
    ) -> Position {
        let positions = &self.positions[line];
        let index = positions.partition_point(|p| value(p) <= key);
        index
            .checked_sub(1)
            .map(|i| positions[i].clone())
            .unwrap_or(Position {
                byte: 0,
                chars: 0,
                columns: 0,
            })
    }

    /// The line containing `offset`; a line break belongs to the line it ends.
    pub fn line_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|&start| start <= offset) - 1
    }

    pub fn content_end(&self, text: &str, line: usize) -> usize {
        content_end(text, self.starts[line], self.next_start(line))
    }

    /// Rebuild only overlapping groups of touched lines in the edited text.
    pub fn update(&mut self, text: &str, changes: &ChangeSet) {
        if self.update_long_line(text, changes) {
            return;
        }
        let edits = changes.changes();
        if edits.is_empty() {
            return;
        }
        // Merge only overlapping old line ranges. Offsets of later groups are
        // mapped by the accumulated byte delta, so insertions and deletions are
        // handled just like equal-length replacements.
        let mut groups: Vec<(usize, usize, usize, usize)> = Vec::new();
        for (i, edit) in edits.iter().enumerate() {
            let first = self.line_of(edit.old.start);
            let last = self.line_of(edit.old.end);
            if let Some(group) = groups.last_mut()
                && first <= group.1
            {
                group.1 = group.1.max(last);
                group.3 = i + 1;
            } else {
                groups.push((first, last, i, i + 1));
            }
        }
        let mut old = std::mem::replace(
            self,
            Self {
                tab_width: self.tab_width,
                starts: Vec::new(),
                endings: Vec::new(),
                crlf_count: 0,
                widths: Vec::new(),
                positions: Vec::new(),
                max_width: 0,
            },
        );
        self.starts.reserve(old.len());
        self.endings.reserve(old.len());
        self.widths.reserve(old.len());
        self.positions.reserve(old.len());
        let mut line = 0;
        let mut delta = 0isize;
        for (first, last, from, to) in groups {
            self.keep_lines(&mut old, line..first, delta);
            let start = (old.starts[first] as isize + delta) as usize;
            delta += edits[from..to]
                .iter()
                .map(|c| c.inserted.len() as isize - c.removed.len() as isize)
                .sum::<isize>();
            let following = old.next_start(last);
            let end = following.map_or(text.len(), |n| (n as isize + delta) as usize);
            self.scan(text, start, end, following.is_none());
            line = last + 1;
        }
        let end = old.len();
        self.keep_lines(&mut old, line..end, delta);
    }

    fn keep_lines(&mut self, old: &mut Self, lines: std::ops::Range<usize>, delta: isize) {
        for line in lines {
            self.starts
                .push((old.starts[line] as isize + delta) as usize);
            self.endings.push(old.endings[line]);
            self.crlf_count += usize::from(old.endings[line]);
            self.widths.push(old.widths[line]);
            self.max_width = self.max_width.max(old.widths[line] as usize);
            self.positions
                .push(std::mem::take(&mut old.positions[line]));
        }
    }

    /// Re-index the edit's surrounding graphemes, shifting sparse tail entries.
    /// A tab-phase or grapheme-context change falls back to the full scanner.
    fn update_long_line(&mut self, text: &str, changes: &ChangeSet) -> bool {
        let [change] = changes.changes() else {
            return false;
        };
        if change.inserted.contains(['\r', '\n']) || change.removed.contains(['\r', '\n']) {
            return false;
        }
        let line = self.line_of(change.old.start);
        if self.line_of(change.old.end) != line {
            return false;
        }
        let start = self.starts[line];
        let delta = change.inserted.len() as isize - change.removed.len() as isize;
        let next = self
            .next_start(line)
            .map(|offset| (offset as isize + delta) as usize);
        let end = content_end(text, start, next);
        if end - start <= 256 || self.positions[line].len() < 3 {
            return false;
        }
        let positions = &mut self.positions[line];
        let from = positions
            .partition_point(|p| p.byte < change.old.start - start)
            .saturating_sub(1);
        let to = (positions.partition_point(|p| p.byte <= change.old.end - start) + 1)
            .min(positions.len() - 1);
        let before = positions[from].clone();
        let after = positions[to].clone();
        let tail = (after.byte as isize + delta) as usize;
        let tail_offset = start + tail;
        if tail_offset > end
            || !text.is_char_boundary(tail_offset)
            || !unicode_segmentation::GraphemeCursor::new(tail_offset, text.len(), true)
                .is_boundary(text, 0)
                .unwrap_or(false)
        {
            return false;
        }
        let mut columns = before.columns;
        let mut chars = before.chars;
        let mut last = before.byte;
        let mut rebuilt = Vec::new();
        for (byte, grapheme) in text[start + before.byte..tail_offset].grapheme_indices(true) {
            let byte = before.byte + byte;
            if byte - last >= 256 {
                rebuilt.push(Position {
                    byte,
                    chars,
                    columns,
                });
                last = byte;
            }
            columns += grapheme_width_with_tab_width(grapheme, columns, self.tab_width);
            chars += grapheme.chars().count();
        }
        let column_delta = columns as isize - after.columns as isize;
        if column_delta % self.tab_width as isize != 0 && text[tail_offset..end].contains('\t') {
            return false;
        }
        let char_delta = chars as isize - after.chars as isize;
        for position in &mut positions[to..] {
            position.byte = (position.byte as isize + delta) as usize;
            position.chars = (position.chars as isize + char_delta) as usize;
            position.columns = (position.columns as isize + column_delta) as usize;
        }
        positions.splice(from + 1..to, rebuilt);
        self.widths[line] = (self.widths[line] as isize + column_delta) as u32;
        for offset in &mut self.starts[line + 1..] {
            *offset = (*offset as isize + delta) as usize;
        }
        let crlf = self
            .next_start(line)
            .is_some_and(|n| n >= 2 && text.as_bytes()[n - 2] == b'\r');
        self.crlf_count = self.crlf_count + usize::from(crlf) - usize::from(self.endings[line]);
        self.endings[line] = crlf;
        self.max_width = self.widths.iter().copied().max().unwrap_or(0) as usize;
        true
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

    #[test]
    fn disjoint_edits_with_different_lengths_preserve_untouched_lines() {
        let original = format!("head\r\n{}tail\n", "中\tstay\r\n".repeat(4096));
        for inserted in ["longer", "", "new\nlines\r\n", "e\u{301}"] {
            let mut text = original.clone();
            let mut index = LineIndex::new(&text);
            let at = text.len() - 5;
            let changes = apply(
                &mut text,
                validate(
                    &original,
                    vec![
                        Edit::replace(0..4, inserted),
                        Edit::replace(at..at + 4, "尾"),
                    ],
                )
                .unwrap(),
            );
            index.update(&text, &changes);
            assert_eq!(index, LineIndex::new(&text));
        }
    }

    fn floor(text: &str, mut offset: usize) -> usize {
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }
}
