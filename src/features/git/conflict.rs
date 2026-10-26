//! Merge-conflict regions of an unmerged file, read from the combined diff Git
//! prints for it (`git diff -- <path>`), never from the file's own text.
//!
//! A combined diff prefixes each line with one column per parent. A line absent
//! from both parents is what Git wrote into the worktree itself: the markers
//! and any diff3 base lines. A line present in only one parent belongs to that
//! side. The regions follow from that structure, so a `<<<<<<<` in an ordinary
//! file is never mistaken for a conflict.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::PathBuf;

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

/// The conflicts of each file in `diff`, the raw output of `git diff` for
/// several unmerged paths, keyed by the path Git printed. A file with none is
/// left out.
pub fn regions_by_file(diff: &str) -> BTreeMap<PathBuf, Vec<ConflictRegion>> {
    let mut files = BTreeMap::new();
    let mut sections: Vec<(&str, usize)> = Vec::new();
    let mut offset = 0;
    for line in diff.split_inclusive('\n') {
        if let Some(path) = line.strip_prefix("diff --cc ") {
            sections.push((path.trim_end_matches(['\r', '\n']), offset));
        }
        offset += line.len();
    }
    for (index, (path, start)) in sections.iter().enumerate() {
        let end = sections
            .get(index + 1)
            .map_or(diff.len(), |(_, next)| *next);
        let regions = parse_regions(&diff[*start..end]);
        if !regions.is_empty() {
            files.insert(PathBuf::from(path), regions);
        }
    }
    files
}

/// How many conflict-marker lines `git diff --check` reported in each file of
/// `output`. Git, not a text scan, decides what counts as a marker.
pub fn leftover_markers(output: &str) -> BTreeMap<PathBuf, usize> {
    let mut files = BTreeMap::new();
    for line in output.lines() {
        let Some(location) = line.strip_suffix(": leftover conflict marker") else {
            continue;
        };
        let Some((path, number)) = location.rsplit_once(':') else {
            continue;
        };
        if number.parse::<usize>().is_ok() {
            *files.entry(PathBuf::from(path)).or_insert(0) += 1;
        }
    }
    files
}

/// What to keep of a conflict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Current,
    Incoming,
    /// The current side, then the incoming side.
    Both,
}

/// Whether the opening, separator and closing lines of `region` are still
/// where Git reported them, given `line` (one-based, `None` past the end).
/// Git read the file on disk, so its answer is stale after the text changes;
/// this tells a position that can still be drawn on or edited from one that
/// cannot. It never decides whether something is a conflict.
pub fn markers_in_place<'a>(
    region: &ConflictRegion,
    line: impl Fn(usize) -> Option<&'a str>,
) -> bool {
    line(region.start).is_some_and(|text| text.starts_with("<<<<<<<"))
        && line(region.separator)
            .is_some_and(|text| text.starts_with("=======") || text.starts_with("|||||||"))
        && line(region.end).is_some_and(|text| text.starts_with(">>>>>>>"))
}

/// The edit that resolves `region` in `text`: the byte range to replace and
/// what to put there. `None` when `text` no longer has the opening, separator
/// and closing lines where Git reported them, because the file changed since
/// Git looked and the position cannot be trusted.
pub fn resolution_edit(
    text: &str,
    region: &ConflictRegion,
    resolution: Resolution,
) -> Option<(Range<usize>, String)> {
    let mut starts = vec![0];
    starts.extend(
        text.match_indices('\n')
            .map(|(at, _)| at + 1)
            .filter(|at| *at < text.len()),
    );
    // Where one-based line `n` begins; one past the last line is the end.
    let begin = |n: usize| starts.get(n.checked_sub(1)?).copied().or(Some(text.len()));
    let line = |n: usize| Some(&text[begin(n)?..begin(n + 1)?]);
    let side = |lines: &Range<usize>| Some(&text[begin(lines.start)?..begin(lines.end)?]);

    if !markers_in_place(region, line) {
        return None;
    }
    let replacement = match resolution {
        Resolution::Current => side(&region.current)?.to_owned(),
        Resolution::Incoming => side(&region.incoming)?.to_owned(),
        Resolution::Both => format!("{}{}", side(&region.current)?, side(&region.incoming)?),
    };
    Some((begin(region.start)?..begin(region.end + 1)?, replacement))
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

    const FILE: &str = "keep\n<<<<<<< HEAD\nX\nX2\n=======\nY\n>>>>>>> topic\nafter\n";

    fn file_region() -> ConflictRegion {
        ConflictRegion {
            start: 2,
            current: 3..5,
            separator: 5,
            incoming: 6..7,
            end: 7,
        }
    }

    fn resolved(resolution: Resolution) -> String {
        let (range, text) = resolution_edit(FILE, &file_region(), resolution).unwrap();
        format!("{}{text}{}", &FILE[..range.start], &FILE[range.end..])
    }

    #[test]
    fn each_resolution_keeps_its_side_and_drops_the_markers() {
        assert_eq!(resolved(Resolution::Current), "keep\nX\nX2\nafter\n");
        assert_eq!(resolved(Resolution::Incoming), "keep\nY\nafter\n");
        assert_eq!(resolved(Resolution::Both), "keep\nX\nX2\nY\nafter\n");
    }

    #[test]
    fn a_resolution_is_refused_when_the_markers_moved() {
        let moved = format!("extra\n{FILE}");
        assert!(resolution_edit(&moved, &file_region(), Resolution::Current).is_none());
        let past_the_end = ConflictRegion {
            end: 40,
            ..file_region()
        };
        assert!(resolution_edit(FILE, &past_the_end, Resolution::Current).is_none());
    }

    #[test]
    fn a_conflict_closing_the_file_without_a_newline_resolves() {
        let text = "<<<<<<< a\nX\n=======\nY\n>>>>>>> b";
        let region = ConflictRegion {
            start: 1,
            current: 2..3,
            separator: 3,
            incoming: 4..5,
            end: 5,
        };
        let (range, replacement) = resolution_edit(text, &region, Resolution::Incoming).unwrap();
        assert_eq!(range, 0..text.len());
        assert_eq!(replacement, "Y\n");
    }

    #[test]
    fn regions_are_grouped_by_the_file_git_printed() {
        let two = format!(
            "{SIMPLE}diff --cc dir/g.txt\nindex 1..2\n--- a/dir/g.txt\n+++ b/dir/g.txt\n@@@ -1,1 -1,1 +1,4 @@@\n++<<<<<<< a\n +P\n++=======\n+ Q\n++>>>>>>> b\n"
        );
        let files = regions_by_file(&two);
        assert_eq!(files.len(), 2);
        assert_eq!(files[&PathBuf::from("f")].len(), 1);
        assert_eq!(files[&PathBuf::from("dir/g.txt")][0].start, 1);
    }

    #[test]
    fn git_check_output_counts_marker_lines_per_file() {
        let output = "f.txt:2: leftover conflict marker\nf.txt:6: leftover conflict marker\ndir/g.txt:1: leftover conflict marker\nf.txt:9: trailing whitespace.\n+x  \n";
        let counts = leftover_markers(output);
        assert_eq!(counts[&PathBuf::from("f.txt")], 2);
        assert_eq!(counts[&PathBuf::from("dir/g.txt")], 1);
        assert_eq!(counts.len(), 2);
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
