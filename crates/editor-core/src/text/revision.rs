//! Bounded, text-free edit provenance for incremental background readers.
use std::{ops::Range, sync::Arc};

#[derive(Clone, Debug)]
pub struct RevisionEdit {
    pub old: Range<usize>,
    pub new: Range<usize>,
    pub line_breaks: bool,
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
