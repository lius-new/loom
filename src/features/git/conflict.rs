//! Merge-conflict regions of an unmerged file, read from the combined diff Git
//! prints for it (`git diff -- <path>`), never from the file's own text.
//!
//! A combined diff prefixes each line with one column per parent. A line absent
//! from both parents is what Git wrote into the worktree itself: the markers
//! and any diff3 base lines. A line present in only one parent belongs to that
//! side. The regions follow from that structure, so a `<<<<<<<` in an ordinary
//! file is never mistaken for a conflict.

use std::ops::Range;

/// One conflict in the worktree file. Lines are one-based; ranges are half-open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictRegion {
    /// The line that opens the conflict.
    pub start: usize,
    /// The lines kept from the first parent (HEAD, or the upstream in a rebase).
    pub current: Range<usize>,
    /// The first line after the current side: the base marker or the separator.
    pub separator: usize,
    /// The lines kept from the second parent.
    pub incoming: Range<usize>,
    /// The line that closes the conflict.
    pub end: usize,
}

/// Which part of a conflict a line belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictPart {
    /// An opening, base, separator or closing line.
    Marker,
    Current,
    Incoming,
}

impl ConflictRegion {
    /// The part of this conflict that one-based `line` is, if it is inside.
    pub fn part_of(&self, line: usize) -> Option<ConflictPart> {
        if !self.lines().contains(&line) {
            None
        } else if self.current.contains(&line) {
            Some(ConflictPart::Current)
        } else if self.incoming.contains(&line) {
            Some(ConflictPart::Incoming)
        } else {
            Some(ConflictPart::Marker)
        }
    }

    /// Every line from the opening to the closing line, inclusive.
    pub fn lines(&self) -> Range<usize> {
        self.start..self.end + 1
    }
}

/// Moves the regions Git found in the saved text `saved` to where those
/// lines are in the unsaved `current` text. A region the edits touch is
/// dropped: Git's answer no longer describes it until the file is saved.
pub fn shifted(regions: &[ConflictRegion], saved: &str, current: &str) -> Vec<ConflictRegion> {
    if saved == current {
        return regions.to_vec();
    }
    let old: Vec<&str> = saved.split('\n').collect();
    let new: Vec<&str> = current.split('\n').collect();
    let shortest = old.len().min(new.len());
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(shortest - prefix)
        .take_while(|(a, b)| a == b)
        .count();
    let delta = new.len() as isize - old.len() as isize;
    let moved = |line: usize| (line as isize + delta) as usize;
    regions
        .iter()
        .filter_map(|region| {
            if region.end <= prefix {
                Some(region.clone())
            } else if region.start > old.len() - suffix {
                Some(ConflictRegion {
                    start: moved(region.start),
                    current: moved(region.current.start)..moved(region.current.end),
                    separator: moved(region.separator),
                    incoming: moved(region.incoming.start)..moved(region.incoming.end),
                    end: moved(region.end),
                })
            } else {
                None
            }
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    /// In both parents: ordinary context.
    Both,
    /// In the first parent only.
    Current,
    /// In the second parent only.
    Incoming,
    /// In neither parent: written by Git.
    Git,
}

/// The conflict regions of `diff`, the raw output of `git diff` for one
/// unmerged path. Anything that is not a two-parent combined diff yields none.
pub fn parse_regions(diff: &str) -> Vec<ConflictRegion> {
    let mut regions = Vec::new();
    let mut lines = diff.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(first) = hunk_start(line) else {
            continue;
        };
        let mut rows = Vec::new();
        let mut number = first;
        while let Some(body) = lines.peek() {
            if body.starts_with("@@") || body.starts_with("diff ") {
                break;
            }
            let body = lines.next().unwrap_or_default();
            let mut columns = body.chars();
            let (Some(a), Some(b)) = (columns.next(), columns.next()) else {
                continue;
            };
            let side = match (a, b) {
                // A removed line is not in the worktree and takes no number.
                ('-', _) | (_, '-') | ('\\', _) => continue,
                (' ', ' ') => Side::Both,
                (' ', '+') => Side::Current,
                ('+', ' ') => Side::Incoming,
                ('+', '+') => Side::Git,
                _ => continue,
            };
            rows.push((number, side));
            number += 1;
        }
        regions.extend(regions_in(&rows));
    }
    regions
}

/// The worktree line a two-parent hunk header (`@@@ -a -b +c,d @@@`) starts at.
fn hunk_start(header: &str) -> Option<usize> {
    let rest = header.strip_prefix("@@@ ")?;
    let mut ranges = rest.split(' ');
    let (first, second, worktree) = (ranges.next()?, ranges.next()?, ranges.next()?);
    if !first.starts_with('-') || !second.starts_with('-') || ranges.next()? != "@@@" {
        return None;
    }
    let start = worktree.strip_prefix('+')?.split(',').next()?;
    start.parse().ok()
}

/// Split one hunk's numbered lines into regions: a Git line opens one, then
/// the current side, Git lines for the separator, the incoming side and a
/// closing Git line. Adjacent regions share no line, so a run of Git lines
/// after the incoming side gives its first line to this region and the rest to
/// the next.
fn regions_in(rows: &[(usize, Side)]) -> Vec<ConflictRegion> {
    let mut regions = Vec::new();
    let run = |from: usize, side: Side| {
        rows[from..]
            .iter()
            .take_while(|(_, kind)| *kind == side)
            .count()
    };
    let mut at = 0;
    while at < rows.len() {
        if rows[at].1 != Side::Git {
            at += 1;
            continue;
        }
        let start = rows[at].0;
        let current_from = at + 1;
        let current_len = run(current_from, Side::Current);
        let separator_from = current_from + current_len;
        let separator_len = run(separator_from, Side::Git);
        let incoming_from = separator_from + separator_len;
        let incoming_len = run(incoming_from, Side::Incoming);
        let closing_from = incoming_from + incoming_len;
        let closing_len = run(closing_from, Side::Git);

        // With no incoming lines the separator and the closing line are one
        // run of Git lines, whose last line closes the conflict.
        let closing = if incoming_len == 0 {
            if separator_len < 2 {
                at += 1;
                continue;
            }
            separator_from + separator_len - 1
        } else if closing_len == 0 || separator_len == 0 {
            at += 1;
            continue;
        } else {
            closing_from
        };
        let number = |index: usize| rows[index].0;
        let incoming_start = if incoming_len == 0 {
            number(closing)
        } else {
            number(incoming_from)
        };
        regions.push(ConflictRegion {
            start,
            current: start + 1..start + 1 + current_len,
            separator: number(separator_from),
            incoming: incoming_start..incoming_start + incoming_len,
            end: number(closing),
        });
        at = closing + 1;
    }
    regions
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE: &str = "diff --cc f\nindex c643bf4,58c9bd1..0000000\n--- a/f\n+++ b/f\n@@@ -2,3 -2,3 +2,9 @@@\n++<<<<<<< HEAD\n +X\n +2\n +X2\n++=======\n+ Y\n+ 2\n+ Y2\n++>>>>>>> 2cf3dea (y)\n";

    #[test]
    fn a_combined_diff_yields_the_sides_of_each_conflict() {
        assert_eq!(
            parse_regions(SIMPLE),
            vec![ConflictRegion {
                start: 2,
                current: 3..6,
                separator: 6,
                incoming: 7..10,
                end: 10,
            }]
        );
    }

    #[test]
    fn lines_know_their_part_of_the_conflict() {
        let region = &parse_regions(SIMPLE)[0];
        let parts: Vec<_> = (1..=11).map(|line| region.part_of(line)).collect();
        use super::ConflictPart::*;
        assert_eq!(
            parts,
            [
                None,
                Some(Marker),
                Some(Current),
                Some(Current),
                Some(Current),
                Some(Marker),
                Some(Incoming),
                Some(Incoming),
                Some(Incoming),
                Some(Marker),
                None
            ]
        );
    }

    fn region_at(start: usize) -> ConflictRegion {
        ConflictRegion {
            start,
            current: start + 1..start + 2,
            separator: start + 2,
            incoming: start + 3..start + 4,
            end: start + 4,
        }
    }

    #[test]
    fn edits_above_a_conflict_move_it_and_edits_inside_drop_it() {
        let saved = "a\nb\nc\nd\ne\nf\ng\nh";
        let regions = [region_at(3)];
        // A line added at the top moves the conflict down.
        let above = "new\na\nb\nc\nd\ne\nf\ng\nh";
        assert_eq!(shifted(&regions, saved, above)[0].start, 4);
        // An edit below leaves it where it was.
        let below = "a\nb\nc\nd\ne\nf\ng\nH";
        assert_eq!(shifted(&regions, saved, below), regions);
        // An edit inside it makes its position untrustworthy.
        let inside = "a\nb\nc\nX\ne\nf\ng\nh";
        assert!(shifted(&regions, saved, inside).is_empty());
        // Saved text equal to the buffer changes nothing.
        assert_eq!(shifted(&regions, saved, saved), regions);
    }

    #[test]
    fn marker_text_in_an_ordinary_diff_is_not_a_conflict() {
        let diff =
            "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1,3 @@\n+<<<<<<< x\n+a\n+>>>>>>> y\n";
        assert!(parse_regions(diff).is_empty());
    }

    #[test]
    fn a_resolved_but_unstaged_file_has_no_regions() {
        let diff = "diff --cc f\n--- a/f\n+++ b/f\n@@@ -1,3 -1,3 +1,3 @@@\n  a\n +X\n+ Y\n  c\n";
        assert!(parse_regions(diff).is_empty());
    }

    #[test]
    fn empty_sides_and_diff3_base_lines_keep_their_boundaries() {
        // Current side empty; incoming one line; diff3 base marker and line.
        let diff = "@@@ -1,1 -1,1 +1,6 @@@\n++<<<<<<< HEAD\n++||||||| base\n++b\n++=======\n+ Y\n++>>>>>>> x\n";
        assert_eq!(
            parse_regions(diff),
            vec![ConflictRegion {
                start: 1,
                current: 2..2,
                separator: 2,
                incoming: 5..6,
                end: 6,
            }]
        );
    }

    #[test]
    fn an_empty_incoming_side_closes_on_the_last_git_line() {
        let diff = "@@@ -1,1 -1,1 +1,4 @@@\n++<<<<<<< HEAD\n +X\n++=======\n++>>>>>>> x\n";
        assert_eq!(
            parse_regions(diff),
            vec![ConflictRegion {
                start: 1,
                current: 2..3,
                separator: 3,
                incoming: 4..4,
                end: 4,
            }]
        );
    }

    #[test]
    fn removed_lines_take_no_worktree_line_number() {
        let diff =
            "@@@ -1,2 -1,2 +5,5 @@@\n- gone\n++<<<<<<< HEAD\n +X\n++=======\n+ Y\n++>>>>>>> x\n";
        assert_eq!(parse_regions(diff)[0].start, 5);
        assert_eq!(parse_regions(diff)[0].end, 9);
    }

    #[test]
    fn two_regions_in_one_hunk_do_not_share_lines() {
        let diff = "@@@ -1,1 -1,1 +1,10 @@@\n++<<<<<<< a\n +X\n++=======\n+ Y\n++>>>>>>> b\n++<<<<<<< a\n +P\n++=======\n+ Q\n++>>>>>>> b\n";
        let regions = parse_regions(diff);
        assert_eq!(regions.len(), 2);
        assert_eq!((regions[0].start, regions[0].end), (1, 5));
        assert_eq!((regions[1].start, regions[1].end), (6, 10));
    }
}
