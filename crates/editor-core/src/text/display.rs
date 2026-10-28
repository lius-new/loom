//! Source offsets and displayed rows share one mapping for wrap and folds.
use super::{Assoc, TextBuffer};
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayRow {
    pub line: usize,
    pub range: Range<usize>,
    /// Source display column at the start, for tabs on wrapped rows.
    pub column: usize,
    pub continuation: bool,
}

#[derive(Clone, Debug)]
pub struct DisplayMap {
    /// None is a zero-allocation linear fallback, including large documents.
    rows: Option<Vec<DisplayRow>>,
    count: usize,
    wrapped: bool,
    folded: bool,
}
const MAX_BYTES: usize = 1024 * 1024;
const MAX_ROWS: usize = 100_000;
impl DisplayMap {
    /// The single display-row movement rule used by ordinary and Vim input.
    pub fn navigate(
        &self,
        buffer: &TextBuffer,
        state: super::Selection,
        movement: crate::normal::Movement,
        extend: bool,
    ) -> super::Selection {
        use crate::normal::Movement;

        let row = self.caret_row(buffer, state.cursor, state.wrap_affinity);
        let source = self.row(buffer, row);
        if state.range().is_none() {
            let flip = match movement {
                Movement::Left
                    if state.wrap_affinity == Assoc::After
                        && source.continuation
                        && state.cursor == source.range.start =>
                {
                    Some(Assoc::Before)
                }
                Movement::Right if state.wrap_affinity == Assoc::Before => Some(Assoc::After),
                _ => None,
            };
            if let Some(affinity) = flip {
                return crate::text::Selection {
                    wrap_affinity: affinity,
                    ..state
                };
            }
        }
        let vertical = match movement {
            Movement::Up => Some(row.saturating_sub(1)),
            Movement::Down => Some((row + 1).min(self.len() - 1)),
            Movement::PageUp(n) => Some(row.saturating_sub(n)),
            Movement::PageDown(n) => Some((row + n).min(self.len() - 1)),
            _ => None,
        };
        let mut after = if let Some(target) = vertical {
            let column = state.preferred_column.unwrap_or_else(|| {
                buffer
                    .display_column(state.cursor)
                    .saturating_sub(source.column)
            });
            let at = self.offset_at(buffer, target, column);
            crate::text::Selection {
                cursor: at,
                anchor: extend.then(|| state.anchor.unwrap_or(state.cursor)),
                wrap_affinity: self.affinity_at(buffer, target, at),
                preferred_column: Some(column),
            }
        } else if matches!(movement, Movement::Home | Movement::End) {
            let at = if movement == Movement::Home {
                source.range.start
            } else {
                source.range.end
            };
            crate::text::Selection {
                cursor: at,
                anchor: extend.then(|| state.anchor.unwrap_or(state.cursor)),
                wrap_affinity: self.affinity_at(buffer, row, at),
                preferred_column: None,
            }
        } else {
            crate::normal::navigated(buffer, state, movement, extend)
        };
        if matches!(movement, Movement::Left | Movement::Right) {
            after.wrap_affinity = if movement == Movement::Right {
                Assoc::Before
            } else {
                Assoc::After
            };
        }
        after
    }
    /// Logical Vim j/k steps skip folds while retaining source-line columns.
    pub fn adjacent_line(
        &self,
        buffer: &TextBuffer,
        cursor: usize,
        down: bool,
        count: usize,
    ) -> usize {
        let line = buffer.line_of(cursor);
        let last = buffer.line_count() - 1;
        if !self.folded {
            return if down {
                line.saturating_add(count).min(last)
            } else {
                line.saturating_sub(count)
            };
        }
        let mut row = self.row_of(buffer, buffer.line_start(line));
        for _ in 0..count.min(buffer.line_count()) {
            let current = self.source_line(row);
            if down {
                while row + 1 < self.len() && self.source_line(row + 1) == current {
                    row += 1;
                }
                if row + 1 == self.len() {
                    break;
                }
                row += 1;
            } else {
                while row > 0 && self.source_line(row - 1) == current {
                    row -= 1;
                }
                if row == 0 {
                    break;
                }
                row -= 1;
            }
        }
        self.source_line(row)
    }
    pub fn new(buffer: &TextBuffer, columns: Option<usize>, folds: &[Range<usize>]) -> Self {
        let linear = || Self {
            rows: None,
            count: buffer.line_count(),
            wrapped: false,
            folded: false,
        };
        if columns.is_none() && folds.is_empty()
            || buffer.len() > MAX_BYTES
            || buffer.line_count() > MAX_ROWS
        {
            return linear();
        }
        let columns = columns.map(|c| c.max(4));
        let mut folds = folds
            .iter()
            .filter(|r| r.start < r.end && r.end <= buffer.len())
            .cloned()
            .collect::<Vec<_>>();
        folds.sort_by_key(|r| r.start);
        let mut hidden = std::collections::BTreeMap::<usize, usize>::new();
        for fold in &folds {
            let first = buffer.line_of(fold.start);
            let last = buffer.line_of(fold.end.saturating_sub(1));
            hidden
                .entry(first)
                .and_modify(|end| *end = (*end).max(last))
                .or_insert(last);
        }
        let mut rows = Vec::new();
        let mut line = 0;
        while line < buffer.line_count() {
            let range = buffer.line_range(line);
            let mut at = range.start;
            let mut column = 0;
            loop {
                let end = if let Some(columns) = columns {
                    let candidate = buffer
                        .offset_at_display_column(line, column + columns)
                        .min(range.end);
                    if candidate <= at && at < range.end {
                        buffer.next_grapheme(at).min(range.end)
                    } else {
                        candidate
                    }
                } else {
                    range.end
                };
                rows.push(DisplayRow {
                    line,
                    range: at..end,
                    column,
                    continuation: at != range.start,
                });
                if rows.len() > MAX_ROWS {
                    if columns.is_some() && !folds.is_empty() {
                        return Self::new(buffer, None, &folds);
                    }
                    return linear();
                }
                if end >= range.end {
                    break;
                }
                at = end;
                column = buffer.display_column(at);
            }
            let folded = hidden.get(&line).copied();
            line = folded.map_or(line + 1, |end| end.max(line) + 1);
        }
        Self {
            count: rows.len(),
            rows: Some(rows),
            wrapped: columns.is_some(),
            folded: !folds.is_empty(),
        }
    }
    pub fn caret_row(&self, buffer: &TextBuffer, offset: usize, affinity: Assoc) -> usize {
        let row = self.row_of(buffer, offset);
        if row > 0 && affinity == Assoc::Before {
            let current = self.row(buffer, row);
            if current.continuation && current.range.start == offset {
                return row - 1;
            }
        }
        row
    }
    pub fn affinity_at(&self, buffer: &TextBuffer, row: usize, offset: usize) -> Assoc {
        if row + 1 < self.len() {
            let next = self.row(buffer, row + 1);
            if next.continuation && next.range.start == offset {
                return Assoc::Before;
            }
        }
        Assoc::After
    }
    pub fn source_line(&self, row: usize) -> usize {
        let row = row.min(self.count.saturating_sub(1));
        self.rows.as_ref().map_or(row, |rows| rows[row].line)
    }
    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub fn folded(&self) -> bool {
        self.folded
    }
    pub fn wrapped(&self) -> bool {
        self.wrapped
    }
    pub fn row(&self, buffer: &TextBuffer, row: usize) -> DisplayRow {
        let row = row.min(self.count.saturating_sub(1));
        self.rows
            .as_ref()
            .map(|rows| rows[row].clone())
            .unwrap_or_else(|| DisplayRow {
                line: row,
                range: buffer.line_range(row),
                column: 0,
                continuation: false,
            })
    }
    pub fn row_of(&self, buffer: &TextBuffer, offset: usize) -> usize {
        self.rows
            .as_ref()
            .map(|rows| {
                rows.partition_point(|r| r.range.start <= offset)
                    .saturating_sub(1)
            })
            .unwrap_or_else(|| buffer.line_of(offset))
    }
    pub fn hidden(&self, buffer: &TextBuffer, offset: usize) -> bool {
        self.row(buffer, self.row_of(buffer, offset)).line != buffer.line_of(offset)
    }
    pub fn offset_at(&self, buffer: &TextBuffer, row: usize, column: usize) -> usize {
        let row = self.row(buffer, row);
        buffer
            .offset_at_display_column(row.line, row.column + column)
            .clamp(row.range.start, row.range.end)
    }
    pub fn column_of(&self, buffer: &TextBuffer, offset: usize) -> usize {
        let row = self.row(buffer, self.row_of(buffer, offset));
        buffer
            .display_column(offset.min(row.range.end))
            .saturating_sub(row.column)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wraps_use_graphemes_and_original_tab_stops_and_fold_mapping_is_reversible() {
        let text = TextBuffer::new("a😀😀\t中文e\u{301}\r\n{\r\n hidden\r\n}\r\nlast");
        let start = text.line_start(1);
        let end = text.line_start(4);
        let folded_range = start..end;
        let display = DisplayMap::new(&text, Some(4), std::slice::from_ref(&folded_range));
        assert!(display.wrapped());
        for row in 0..display.len() {
            let r = display.row(&text, row);
            assert!(text.is_grapheme_boundary(r.range.start));
            assert!(text.is_grapheme_boundary(r.range.end));
            assert_eq!(display.row_of(&text, r.range.start), row);
        }
        assert!(display.hidden(&text, text.line_start(2)));
        assert!(!display.hidden(&text, text.line_start(4)));
        let expanded = DisplayMap::new(&text, Some(4), &[]);
        assert!(!expanded.hidden(&text, text.line_start(2)));
        assert!(expanded.len() > display.len());
    }
    #[test]
    fn empty_eof_long_lines_and_large_file_fallbacks_are_bounded() {
        for source in ["", "abc\n", "a😀b😀c😀", "\t\t\t\n"] {
            let text = TextBuffer::new(source);
            let map = DisplayMap::new(&text, Some(4), &[]);
            assert!(!map.is_empty());
            assert!(map.row_of(&text, text.len()) < map.len());
            for at in 0..=text.len() {
                if text.is_grapheme_boundary(at) {
                    let row = map.row_of(&text, at);
                    if !map.hidden(&text, at) {
                        let col = map.column_of(&text, at);
                        assert_eq!(map.offset_at(&text, row, col), at);
                    }
                }
            }
        }
        let text = TextBuffer::new(" ".repeat(MAX_BYTES + 1));
        let map = DisplayMap::new(&text, Some(4), &[]);
        assert_eq!(map.len(), 1);
        assert!(!map.wrapped());
    }
}
