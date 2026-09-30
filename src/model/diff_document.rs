//! Read-only, line-oriented documents shown by the diff editor.

use std::path::{Path, PathBuf};

use crate::git::DiffTarget;
use crate::git::diff::{DiffLine, UnifiedDiff};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffRowKind {
    Context,
    Addition,
    Deletion,
    Hunk,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffRow {
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
    pub kind: DiffRowKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitDiffRow {
    pub old_line: Option<usize>,
    pub old_text: Option<String>,
    pub new_line: Option<usize>,
    pub new_text: Option<String>,
    pub hunk: Option<String>,
    pub changed: bool,
}

#[derive(Clone, Debug)]
pub struct DiffDocument {
    pub repository_root: PathBuf,
    pub path: PathBuf,
    pub target: DiffTarget,
    pub rows: Vec<DiffRow>,
    pub split_rows: Vec<SplitDiffRow>,
    pub binary: bool,
}

impl DiffDocument {
    pub fn from_unified(
        repository_root: PathBuf,
        path: PathBuf,
        target: DiffTarget,
        diff: UnifiedDiff,
    ) -> Self {
        let mut rows = Vec::new();
        let mut binary = false;

        if let Some(file) = diff.files.into_iter().next() {
            binary = file.binary;
            for (hunk_index, hunk) in file.hunks.into_iter().enumerate() {
                if hunk_index > 0 || hunk.old_start > 1 || hunk.new_start > 1 {
                    rows.push(DiffRow {
                        old_line: None,
                        new_line: None,
                        text: format!(
                            "@@ -{},{} +{},{} @@{}",
                            hunk.old_start,
                            hunk.old_lines,
                            hunk.new_start,
                            hunk.new_lines,
                            if hunk.header.is_empty() {
                                String::new()
                            } else {
                                format!(" {}", hunk.header)
                            }
                        ),
                        kind: DiffRowKind::Hunk,
                    });
                }

                let mut old_line = hunk.old_start;
                let mut new_line = hunk.new_start;
                for line in hunk.lines {
                    match line {
                        DiffLine::Context(text) => {
                            rows.push(DiffRow {
                                old_line: line_number(old_line),
                                new_line: line_number(new_line),
                                text,
                                kind: DiffRowKind::Context,
                            });
                            old_line += 1;
                            new_line += 1;
                        }
                        DiffLine::Addition(text) => {
                            rows.push(DiffRow {
                                old_line: None,
                                new_line: line_number(new_line),
                                text,
                                kind: DiffRowKind::Addition,
                            });
                            new_line += 1;
                        }
                        DiffLine::Deletion(text) => {
                            rows.push(DiffRow {
                                old_line: line_number(old_line),
                                new_line: None,
                                text,
                                kind: DiffRowKind::Deletion,
                            });
                            old_line += 1;
                        }
                        DiffLine::NoNewline => {}
                    }
                }
            }
        }

        let split_rows = build_split_rows(&rows);
        Self {
            repository_root,
            path,
            target,
            rows,
            split_rows,
            binary,
        }
    }

    pub fn added(
        repository_root: PathBuf,
        path: PathBuf,
        target: DiffTarget,
        contents: &str,
    ) -> Self {
        let rows: Vec<DiffRow> = contents
            .lines()
            .enumerate()
            .map(|(index, text)| DiffRow {
                old_line: None,
                new_line: Some(index + 1),
                text: text.to_owned(),
                kind: DiffRowKind::Addition,
            })
            .collect();
        let split_rows = build_split_rows(&rows);
        Self {
            repository_root,
            path,
            target,
            rows,
            split_rows,
            binary: false,
        }
    }

    pub fn absolute_path(&self) -> PathBuf {
        self.repository_root.join(&self.path)
    }

    pub fn title(&self) -> String {
        let name = self
            .path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned());
        format!("{name} ({})", self.source_label())
    }

    pub fn source_label(&self) -> &'static str {
        match self.target {
            DiffTarget::HeadToIndex => "Index",
            DiffTarget::IndexToWorktree | DiffTarget::HeadToWorktree => "Working Tree",
        }
    }

    pub fn matches(&self, repository_root: &Path, path: &Path, target: DiffTarget) -> bool {
        self.repository_root == repository_root && self.path == path && self.target == target
    }

    pub fn additions(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.kind == DiffRowKind::Addition)
            .count()
    }

    pub fn deletions(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.kind == DiffRowKind::Deletion)
            .count()
    }

    pub fn change_starts(&self, split: bool) -> Vec<usize> {
        if split {
            self.split_rows
                .iter()
                .enumerate()
                .filter_map(|(index, row)| {
                    (row.changed
                        && (index == 0 || !self.split_rows[index.saturating_sub(1)].changed))
                        .then_some(index)
                })
                .collect()
        } else {
            self.rows
                .iter()
                .enumerate()
                .filter_map(|(index, row)| {
                    let changed = matches!(row.kind, DiffRowKind::Addition | DiffRowKind::Deletion);
                    let previous_changed = index > 0
                        && matches!(
                            self.rows[index - 1].kind,
                            DiffRowKind::Addition | DiffRowKind::Deletion
                        );
                    (changed && !previous_changed).then_some(index)
                })
                .collect()
        }
    }
}

fn build_split_rows(rows: &[DiffRow]) -> Vec<SplitDiffRow> {
    let mut result = Vec::new();
    let mut index = 0;
    while index < rows.len() {
        match rows[index].kind {
            DiffRowKind::Context => {
                let row = &rows[index];
                result.push(SplitDiffRow {
                    old_line: row.old_line,
                    old_text: Some(row.text.clone()),
                    new_line: row.new_line,
                    new_text: Some(row.text.clone()),
                    hunk: None,
                    changed: false,
                });
                index += 1;
            }
            DiffRowKind::Hunk => {
                result.push(SplitDiffRow {
                    old_line: None,
                    old_text: None,
                    new_line: None,
                    new_text: None,
                    hunk: Some(rows[index].text.clone()),
                    changed: false,
                });
                index += 1;
            }
            DiffRowKind::Addition | DiffRowKind::Deletion => {
                let block_start = index;
                while index < rows.len()
                    && matches!(
                        rows[index].kind,
                        DiffRowKind::Addition | DiffRowKind::Deletion
                    )
                {
                    index += 1;
                }
                let block = &rows[block_start..index];
                let deletions = block
                    .iter()
                    .filter(|row| row.kind == DiffRowKind::Deletion)
                    .collect::<Vec<_>>();
                let additions = block
                    .iter()
                    .filter(|row| row.kind == DiffRowKind::Addition)
                    .collect::<Vec<_>>();
                for offset in 0..deletions.len().max(additions.len()) {
                    let old = deletions.get(offset).copied();
                    let new = additions.get(offset).copied();
                    result.push(SplitDiffRow {
                        old_line: old.and_then(|row| row.old_line),
                        old_text: old.map(|row| row.text.clone()),
                        new_line: new.and_then(|row| row.new_line),
                        new_text: new.map(|row| row.text.clone()),
                        hunk: None,
                        changed: true,
                    });
                }
            }
        }
    }
    result
}

fn line_number(value: usize) -> Option<usize> {
    (value > 0).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::diff::parse_unified;

    #[test]
    fn converts_a_unified_diff_to_numbered_editor_rows() {
        let diff = parse_unified(
            "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,3 @@\n same\n-old\n+new\n+more\n",
        )
        .unwrap();
        let document = DiffDocument::from_unified(
            PathBuf::from("repo"),
            PathBuf::from("a.txt"),
            DiffTarget::IndexToWorktree,
            diff,
        );

        assert_eq!(document.rows.len(), 4);
        assert_eq!(document.rows[0].old_line, Some(1));
        assert_eq!(document.rows[0].new_line, Some(1));
        assert_eq!(document.rows[1].kind, DiffRowKind::Deletion);
        assert_eq!(document.rows[1].new_line, None);
        assert_eq!(document.rows[2].kind, DiffRowKind::Addition);
        assert_eq!(document.rows[2].new_line, Some(2));
        assert_eq!(document.split_rows.len(), 3);
        assert_eq!(document.split_rows[1].old_text.as_deref(), Some("old"));
        assert_eq!(document.split_rows[1].new_text.as_deref(), Some("new"));
        assert_eq!(document.split_rows[2].old_text, None);
        assert_eq!(document.split_rows[2].new_text.as_deref(), Some("more"));
        assert_eq!(document.change_starts(false), vec![1]);
        assert_eq!(document.change_starts(true), vec![1]);
        assert_eq!(document.title(), "a.txt (Working Tree)");
    }
}
