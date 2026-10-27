//! Text selection inside a diff view: which code characters are chosen and
//! what they copy as. No geometry; the UI turns pointer positions into points.
use super::diff_document::{DiffDocument, DiffRowKind};
use crate::domain::identity::PaneId;
use std::ops::Range;

/// Which column of a side-by-side diff the selection lives in. Inline diffs
/// have a single column and always use `New`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffSide {
    Old,
    New,
}

/// A character position: the file section (0 for a single-file diff), the row
/// inside it and the character index in that row's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DiffPoint {
    pub section: usize,
    pub row: usize,
    pub column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionUnit {
    Character,
    Word,
    Line,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffSelection {
    pub pane: PaneId,
    pub side: DiffSide,
    pub anchor: DiffPoint,
    pub head: DiffPoint,
    pub unit: SelectionUnit,
    /// The range the press selected; word and line drags grow from it.
    pub origin: (DiffPoint, DiffPoint),
    pub dragging: bool,
}

fn section_document(document: &DiffDocument, section: usize) -> Option<&DiffDocument> {
    match &document.sections {
        Some(sections) => sections.get(section).map(|item| &*item.document),
        None => (section == 0).then_some(document),
    }
}

/// The selectable code of one row; hunk headers and blank sides have none.
pub fn row_text(
    document: &DiffDocument,
    split: bool,
    section: usize,
    row: usize,
    side: DiffSide,
) -> Option<&str> {
    let document = section_document(document, section)?;
    if split {
        let row = document.split_rows.get(row)?;
        if row.hunk.is_some() {
            return None;
        }
        match side {
            DiffSide::Old => row.old_text.as_deref(),
            DiffSide::New => row.new_text.as_deref(),
        }
    } else {
        let row = document.rows.get(row)?;
        (row.kind != DiffRowKind::Hunk).then_some(row.text.as_str())
    }
}

pub fn row_count(document: &DiffDocument, split: bool, section: usize) -> usize {
    section_document(document, section).map_or(0, |item| {
        if split {
            item.split_rows.len()
        } else {
            item.rows.len()
        }
    })
}

fn section_count(document: &DiffDocument) -> usize {
    document.sections.as_ref().map_or(1, Vec::len)
}

fn collapsed(
    document: &DiffDocument,
    section: usize,
    collapsed: &std::collections::BTreeSet<super::DiffRequest>,
) -> bool {
    document
        .sections
        .as_ref()
        .and_then(|sections| sections.get(section))
        .is_some_and(|item| collapsed.contains(&item.request()))
}

fn slice_chars(text: &str, range: Range<usize>) -> &str {
    let mut indices = text.char_indices().map(|(index, _)| index);
    let start = indices.clone().nth(range.start).unwrap_or(text.len());
    let end = indices.nth(range.end).unwrap_or(text.len());
    &text[start..end.max(start)]
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Word,
    Space,
    Other,
}

fn class(ch: char) -> CharClass {
    if ch.is_alphanumeric() || ch == '_' {
        CharClass::Word
    } else if ch.is_whitespace() {
        CharClass::Space
    } else {
        CharClass::Other
    }
}

/// The run of same-kind characters around `column`.
pub fn word_range(text: &str, column: usize) -> Range<usize> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return 0..0;
    }
    let at = column.min(chars.len() - 1);
    let kind = class(chars[at]);
    let mut start = at;
    while start > 0 && class(chars[start - 1]) == kind {
        start -= 1;
    }
    let mut end = at + 1;
    while end < chars.len() && class(chars[end]) == kind {
        end += 1;
    }
    start..end
}

/// The range a press or drag position selects for `unit`.
fn unit_range(
    document: &DiffDocument,
    split: bool,
    side: DiffSide,
    point: DiffPoint,
    unit: SelectionUnit,
) -> (DiffPoint, DiffPoint) {
    let text = row_text(document, split, point.section, point.row, side).unwrap_or("");
    let range = match unit {
        SelectionUnit::Character => return (point, point),
        SelectionUnit::Word => word_range(text, point.column),
        SelectionUnit::Line => 0..text.chars().count(),
    };
    (
        DiffPoint {
            column: range.start,
            ..point
        },
        DiffPoint {
            column: range.end,
            ..point
        },
    )
}

impl DiffSelection {
    /// Start a selection at `point`. A shift press keeps the old anchor.
    pub fn start(
        document: &DiffDocument,
        split: bool,
        pane: PaneId,
        side: DiffSide,
        point: DiffPoint,
        unit: SelectionUnit,
        extend: Option<&DiffSelection>,
    ) -> Self {
        if unit == SelectionUnit::Character
            && let Some(previous) = extend.filter(|old| old.pane == pane && old.side == side)
        {
            return Self {
                head: point,
                dragging: true,
                unit,
                origin: (previous.anchor, previous.anchor),
                ..*previous
            };
        }
        let (start, end) = unit_range(document, split, side, point, unit);
        Self {
            pane,
            side,
            anchor: start,
            head: end,
            unit,
            origin: (start, end),
            dragging: true,
        }
    }

    /// Move the free end to `point`, growing by whole words or lines when the
    /// press selected one.
    pub fn drag_to(&mut self, document: &DiffDocument, split: bool, point: DiffPoint) {
        let (start, end) = unit_range(document, split, self.side, point, self.unit);
        if self.unit == SelectionUnit::Character {
            self.head = point;
        } else if point < self.origin.0 {
            self.anchor = self.origin.1;
            self.head = start;
        } else {
            self.anchor = self.origin.0;
            self.head = end;
        }
    }

    /// Everything on the active side of every expanded file.
    pub fn all(
        document: &DiffDocument,
        split: bool,
        pane: PaneId,
        hidden: &std::collections::BTreeSet<super::DiffRequest>,
    ) -> Option<Self> {
        let side = DiffSide::New;
        let mut first = None;
        let mut last = None;
        for section in 0..section_count(document) {
            if collapsed(document, section, hidden) {
                continue;
            }
            for row in 0..row_count(document, split, section) {
                if let Some(text) = row_text(document, split, section, row, side) {
                    first.get_or_insert(DiffPoint {
                        section,
                        row,
                        column: 0,
                    });
                    last = Some(DiffPoint {
                        section,
                        row,
                        column: text.chars().count(),
                    });
                }
            }
        }
        let (anchor, head) = (first?, last?);
        Some(Self {
            pane,
            side,
            anchor,
            head,
            unit: SelectionUnit::Character,
            origin: (anchor, anchor),
            dragging: false,
        })
    }

    pub fn ordered(&self) -> (DiffPoint, DiffPoint) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// The selected characters of one row, if the selection reaches it.
    pub fn span(
        &self,
        document: &DiffDocument,
        split: bool,
        section: usize,
        row: usize,
    ) -> Option<Range<usize>> {
        let (start, end) = self.ordered();
        let key = (section, row);
        if key < (start.section, start.row) || key > (end.section, end.row) {
            return None;
        }
        let length = row_text(document, split, section, row, self.side)?
            .chars()
            .count();
        let from = if key == (start.section, start.row) {
            start.column.min(length)
        } else {
            0
        };
        let to = if key == (end.section, end.row) {
            end.column.min(length)
        } else {
            length
        };
        Some(from..to.max(from))
    }

    /// The text a copy puts on the clipboard: code only, one row per line.
    pub fn text(
        &self,
        document: &DiffDocument,
        split: bool,
        hidden: &std::collections::BTreeSet<super::DiffRequest>,
    ) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let (start, end) = self.ordered();
        let mut lines = Vec::new();
        for section in start.section..=end.section.min(section_count(document) - 1) {
            if collapsed(document, section, hidden) {
                continue;
            }
            let first = if section == start.section {
                start.row
            } else {
                0
            };
            let last = if section == end.section {
                end.row + 1
            } else {
                row_count(document, split, section)
            };
            for row in first..last.min(row_count(document, split, section)) {
                if let (Some(range), Some(text)) = (
                    self.span(document, split, section, row),
                    row_text(document, split, section, row, self.side),
                ) {
                    lines.push(slice_chars(text, range));
                }
            }
        }
        (!lines.is_empty()).then(|| lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_group_by_character_kind() {
        assert_eq!(word_range("let value_1 = 2;", 5), 4..11);
        assert_eq!(word_range("let value_1 = 2;", 3), 3..4);
        assert_eq!(word_range("a == b", 3), 2..4);
        assert_eq!(word_range("", 0), 0..0);
        assert_eq!(word_range("end", 9), 0..3);
    }

    #[test]
    fn slices_follow_characters_not_bytes() {
        assert_eq!(slice_chars("héllo wörld", 1..8), "éllo wö");
        assert_eq!(slice_chars("abc", 2..9), "c");
        assert_eq!(slice_chars("abc", 5..9), "");
    }
}
