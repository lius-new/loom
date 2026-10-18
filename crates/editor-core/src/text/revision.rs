//! Bounded, text-free edit provenance for incremental background readers.
use std::{ops::Range, sync::Arc};

/// A position as a zero-based row and a byte column within that row. Rows are
/// counted in LF bytes, so a CRLF pair is one break and a lone CR is not one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BytePoint {
    pub row: usize,
    pub column: usize,
}

/// The size of inserted text, without keeping the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextExtent {
    pub bytes: usize,
    /// LF bytes in the text.
    pub line_breaks: usize,
    /// Bytes after the last LF, or all bytes when there is none.
    pub last_line_bytes: usize,
}

impl TextExtent {
    pub fn of(text: &str) -> Self {
        let line_breaks = text.bytes().filter(|&byte| byte == b'\n').count();
        let last_line_bytes = text
            .rfind('\n')
            .map_or(text.len(), |at| text.len() - at - 1);
        Self {
            bytes: text.len(),
            line_breaks,
            last_line_bytes,
        }
    }
}

/// One replacement. `old` and `old_*_point` describe the text before the whole
/// transaction; `new` describes the text after it. Several edits of one
/// revision are therefore NOT a sequence: a reader applying them one by one
/// must go from the last edit to the first, and derive the new end from
/// `old.start + inserted_extent.bytes` rather than from `new.end`.
#[derive(Clone, Debug)]
pub struct RevisionEdit {
    pub old: Range<usize>,
    pub new: Range<usize>,
    pub old_start_point: BytePoint,
    pub old_end_point: BytePoint,
    pub inserted_extent: TextExtent,
    pub line_breaks: bool,
}

impl RevisionEdit {
    /// Where the inserted text ends, relative to the start of this edit in the
    /// text as it is when this single edit is applied.
    pub fn new_end_point(&self) -> BytePoint {
        let start = self.old_start_point;
        if self.inserted_extent.line_breaks == 0 {
            BytePoint {
                row: start.row,
                column: start.column + self.inserted_extent.bytes,
            }
        } else {
            BytePoint {
                row: start.row + self.inserted_extent.line_breaks,
                column: self.inserted_extent.last_line_bytes,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{Assoc, Edit, TextBuffer};

    #[test]
    fn metadata_maps_like_changes_without_retaining_text_or_history() {
        let mut buffer = TextBuffer::new("abcdef\nghij");
        let old = buffer.clone();
        let changes = buffer
            .apply_edits(vec![Edit::replace(1..3, "X"), Edit::insert(8, "\nYZ")])
            .unwrap();
        let revisions = buffer
            .revisions_since(old.version(), old.snapshot_identity())
            .unwrap();
        assert_eq!(revisions.len(), 1);
        assert!(!revisions[0].edits[0].line_breaks);
        assert!(revisions[0].edits[1].line_breaks);
        for offset in 0..=old.len() {
            for (after, assoc) in [(false, Assoc::Before), (true, Assoc::After)] {
                assert_eq!(revisions[0].map(offset, after), changes.map(offset, assoc));
            }
        }
        assert_eq!(Arc::strong_count(&old.text_snapshot()), 2);
    }

    #[test]
    fn journal_is_bounded_and_reload_and_divergent_snapshots_break_provenance() {
        let mut buffer = TextBuffer::new("a");
        let old = buffer.clone();
        for _ in 0..70 {
            buffer.apply_edits(vec![Edit::insert(0, "x")]).unwrap();
        }
        assert_eq!(buffer.revision_log().len(), 64);
        assert!(
            buffer
                .revisions_since(old.version(), old.snapshot_identity())
                .is_none()
        );
        let recent = buffer.clone();
        let mut branch = recent.clone();
        branch.apply_edits(vec![Edit::insert(0, "y")]).unwrap();
        buffer.apply_edits(vec![Edit::insert(0, "z")]).unwrap();
        assert!(
            buffer
                .revisions_since(branch.version(), branch.snapshot_identity())
                .is_none()
        );
        assert_eq!(
            buffer
                .revisions_since(recent.version(), recent.snapshot_identity())
                .unwrap()
                .len(),
            1
        );
        buffer.reload("reloaded".into());
        assert!(
            buffer
                .revisions_since(recent.version(), recent.snapshot_identity())
                .is_none()
        );
        let edits = (0..buffer.len()).map(|i| Edit::insert(i, "x")).collect();
        buffer.apply_edits(edits).unwrap();
        let old = buffer.clone();
        let edits = (0..2050).map(|i| Edit::insert(i, "x")).collect();
        buffer.reload("a".repeat(2050));
        buffer.apply_edits(edits).unwrap();
        assert!(buffer.revision_log().is_empty());
        assert!(
            buffer
                .revisions_since(old.version(), old.snapshot_identity())
                .is_none()
        );
    }
}

#[derive(Clone, Debug)]
pub struct TextRevision {
    pub before_version: u64,
    pub after_version: u64,
    pub before_snapshot: usize,
    pub after_snapshot: usize,
    pub edits: Arc<[RevisionEdit]>,
}

impl TextRevision {
    pub fn map(&self, offset: usize, after: bool) -> usize {
        let mut delta = 0isize;
        for edit in self.edits.iter() {
            if offset < edit.old.start {
                break;
            }
            if offset == edit.old.start || offset < edit.old.end {
                return if after { edit.new.end } else { edit.new.start };
            }
            delta = edit.new.end as isize - edit.old.end as isize;
        }
        (offset as isize + delta) as usize
    }
}

#[cfg(test)]
mod geometry_tests {
    use crate::text::{BytePoint, Edit, TextBuffer};

    fn point_of(text: &str, offset: usize) -> BytePoint {
        let before = &text.as_bytes()[..offset];
        let row = before.iter().filter(|&&byte| byte == b'\n').count();
        let start = before
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(0, |at| at + 1);
        BytePoint {
            row,
            column: offset - start,
        }
    }

    /// Replays one revision the way an incremental parser must: last edit first,
    /// with the new end derived from the inserted extent. Every recorded point
    /// is checked against a direct computation on the text at that moment.
    fn replay(old: &str, new: &str, buffer: &TextBuffer, since: &TextBuffer) {
        let revisions = buffer
            .revisions_since(since.version(), since.snapshot_identity())
            .unwrap();
        let mut working = old.to_owned();
        for revision in &revisions {
            for edit in revision.edits.iter().rev() {
                assert_eq!(edit.old_start_point, point_of(&working, edit.old.start));
                assert_eq!(edit.old_end_point, point_of(&working, edit.old.end));
                let inserted = &new[edit.new.clone()];
                working.replace_range(edit.old.clone(), inserted);
                let new_end = edit.old.start + edit.inserted_extent.bytes;
                assert_eq!(new_end - edit.old.start, inserted.len());
                assert_eq!(edit.new_end_point(), point_of(&working, new_end));
            }
        }
        assert_eq!(working, new);
    }

    #[test]
    fn spec_example_applies_back_to_front_with_inserted_extents() {
        let mut buffer = TextBuffer::new("abcdef");
        let since = buffer.clone();
        buffer
            .apply_edits(vec![Edit::replace(1..2, "XX"), Edit::replace(4..5, "YYY")])
            .unwrap();
        assert_eq!(buffer.text(), "aXXcdYYYf");
        let revision = &buffer
            .revisions_since(since.version(), since.snapshot_identity())
            .unwrap()[0];
        let steps: Vec<_> = revision
            .edits
            .iter()
            .rev()
            .map(|edit| {
                (
                    edit.old.start,
                    edit.old.end,
                    edit.old.start + edit.inserted_extent.bytes,
                )
            })
            .collect();
        // The first step's new end is 7, not the 8 that `new.end` would give.
        assert_eq!(steps, vec![(4, 5, 7), (1, 2, 3)]);
        assert_eq!(revision.edits[1].new, 5..8);
        replay("abcdef", buffer.text(), &buffer, &since);
    }

    #[test]
    fn points_follow_lf_rows_for_crlf_lone_cr_and_unicode() {
        let old = "a\r\nb中\rc\n😀d\r\n";
        let mut buffer = TextBuffer::new(old);
        let since = buffer.clone();
        buffer
            .apply_edits(vec![
                Edit::replace(1..3, "\n\n"),
                Edit::insert(old.find("\rc").unwrap(), "x\r\ny"),
                Edit::replace(old.find('😀').unwrap()..old.len(), "é\n"),
            ])
            .unwrap();
        let first = &buffer
            .revisions_since(since.version(), since.snapshot_identity())
            .unwrap()[0]
            .edits[0];
        // CRLF removed, two LF inserted: rows advance by LF only.
        assert_eq!(first.old_start_point, BytePoint { row: 0, column: 1 });
        assert_eq!(first.old_end_point, BytePoint { row: 1, column: 0 });
        assert_eq!(first.inserted_extent.line_breaks, 2);
        replay(old, buffer.text(), &buffer, &since);
        // A lone CR is not a row break.
        assert_eq!(point_of("b\rc", 3), BytePoint { row: 0, column: 3 });
    }

    #[test]
    fn random_multi_edit_revisions_replay_to_the_final_text() {
        let alphabet = ["a", "中", "😀", "\n", "\r\n", "\r", "xy", ""];
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % bound as u64) as usize
        };
        for _ in 0..300 {
            let mut old = String::new();
            for _ in 0..next(30) {
                old.push_str(alphabet[next(alphabet.len())]);
            }
            let mut buffer = TextBuffer::new(old.clone());
            let since = buffer.clone();
            let mut edits: Vec<Edit> = Vec::new();
            let mut cursor = 0;
            while cursor <= old.len() && edits.len() < 6 {
                let mut start = cursor + next(old.len() - cursor + 1);
                while !old.is_char_boundary(start) {
                    start += 1;
                }
                let mut end = (start + next(4)).min(old.len());
                while !old.is_char_boundary(end) {
                    end += 1;
                }
                let text = (0..next(3))
                    .map(|_| alphabet[next(alphabet.len())])
                    .collect::<String>();
                if !(start == end && text.is_empty()) {
                    edits.push(Edit::replace(start..end, text));
                }
                // Adjacent edits are allowed; two insertions at one offset are not.
                cursor = end.max(start + 1);
            }
            let Ok(_) = buffer.apply_edits(edits) else {
                continue;
            };
            let new = buffer.text().to_owned();
            if buffer
                .revisions_since(since.version(), since.snapshot_identity())
                .is_some()
            {
                replay(&old, &new, &buffer, &since);
            }
        }
    }
    #[test]
    fn undo_and_redo_record_replayable_geometry() {
        use crate::text::{EditKind, EditorMut, Selection};
        let mut buffer = TextBuffer::new("one\r\ntwo 中\nthree");
        let mut steps = 0;
        let mut check = |buffer: &mut TextBuffer, op: &dyn Fn(&mut EditorMut<'_>)| {
            let before_text = buffer.text().to_owned();
            let since = buffer.clone();
            op(&mut EditorMut::new(
                buffer,
                &mut Selection::default(),
                Vec::new(),
            ));
            let revisions = buffer
                .revisions_since(since.version(), since.snapshot_identity())
                .expect("the journal covers this step");
            assert_eq!(
                revisions.len(),
                1,
                "one batch per step keeps the replay simple"
            );
            replay(&before_text, buffer.text(), buffer, &since);
            steps += 1;
        };
        check(&mut buffer, &|e| {
            e.replace(3..5, "\n\n\n", EditKind::Other).unwrap();
        });
        check(&mut buffer, &|e| {
            e.replace(0..1, "😀x", EditKind::Other).unwrap();
        });
        check(&mut buffer, &|e| {
            e.undo();
        });
        check(&mut buffer, &|e| {
            e.undo();
        });
        check(&mut buffer, &|e| {
            e.redo();
        });
        check(&mut buffer, &|e| {
            e.redo();
        });
        assert_eq!(steps, 6);
        assert_eq!(buffer.text(), "😀xne\n\n\ntwo 中\nthree");
    }

    #[test]
    fn reload_and_divergent_snapshots_break_the_chain() {
        let mut buffer = TextBuffer::new("a\nb");
        let since = buffer.clone();
        buffer.apply_edits(vec![Edit::insert(1, "\n")]).unwrap();
        assert!(
            buffer
                .revisions_since(since.version(), since.snapshot_identity())
                .is_some()
        );
        buffer.reload("a\nb\nc".into());
        assert!(
            buffer
                .revisions_since(since.version(), since.snapshot_identity())
                .is_none()
        );
        // Recovery is a fresh baseline: from a snapshot taken after the reload
        // the chain is intact again.
        let after = buffer.clone();
        buffer.apply_edits(vec![Edit::insert(0, "x")]).unwrap();
        assert_eq!(
            buffer
                .revisions_since(after.version(), after.snapshot_identity())
                .unwrap()
                .len(),
            1
        );
    }
}
