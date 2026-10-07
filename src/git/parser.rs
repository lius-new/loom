use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use super::error::{GitError, GitErrorKind, GitResult};
use super::types::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedStatus {
    pub head: HeadState,
    pub upstream: Option<UpstreamState>,
    pub ahead: u32,
    pub behind: u32,
    pub files: BTreeMap<PathBuf, FileState>,
    pub ignored: BTreeSet<PathBuf>,
}

pub fn parse_porcelain_v2(input: &[u8]) -> GitResult<ParsedStatus> {
    let records = input.split(|byte| *byte == 0).collect::<Vec<_>>();
    let mut status = ParsedStatus::default();
    let mut branch_oid = None;
    let mut branch_name = None;
    let mut index = 0;
    while index < records.len() {
        let record = records[index];
        index += 1;
        if record.is_empty() {
            continue;
        }
        if record.starts_with(b"# ") {
            parse_header(record, &mut status, &mut branch_oid, &mut branch_name)?;
            continue;
        }
        match record[0] {
            b'1' => {
                let fields = split_prefix(record, 8)?;
                let xy = fields[1];
                status.files.insert(
                    bytes_to_path(fields[8]),
                    FileState {
                        index: change_at(xy, 0),
                        worktree: change_at(xy, 1),
                        ..FileState::default()
                    },
                );
            }
            b'2' => {
                let fields = split_prefix(record, 9)?;
                let original = records.get(index).copied().ok_or_else(|| invalid(record))?;
                index += 1;
                let xy = fields[1];
                status.files.insert(
                    bytes_to_path(fields[9]),
                    FileState {
                        index: change_at(xy, 0),
                        worktree: change_at(xy, 1),
                        original_path: Some(bytes_to_path(original)),
                        ..FileState::default()
                    },
                );
            }
            b'u' => {
                let fields = split_prefix(record, 10)?;
                let xy = fields[1];
                status.files.insert(
                    bytes_to_path(fields[10]),
                    FileState {
                        index: change_at(xy, 0),
                        worktree: change_at(xy, 1),
                        conflict: Some(conflict_kind(xy)),
                        ..FileState::default()
                    },
                );
            }
            b'?' | b'!' => {
                if record.len() < 3 || record[1] != b' ' {
                    return Err(invalid(record));
                }
                let path = bytes_to_path(&record[2..]);
                if record[0] == b'!' {
                    // Ignored paths are not changes. Keeping them out of
                    // `files` keeps dirty counts and Source Control unaware.
                    status.ignored.insert(path);
                    continue;
                }
                status.files.insert(
                    path,
                    FileState {
                        worktree: ChangeKind::Untracked,
                        ..FileState::default()
                    },
                );
            }
            _ => return Err(invalid(record)),
        }
    }

    status.head = match (branch_name.as_deref(), branch_oid.as_deref()) {
        (Some(name), Some("(initial)")) => HeadState::Unborn(name.to_owned()),
        (Some("(detached)"), Some(oid)) => HeadState::Detached(oid.to_owned()),
        (Some(name), _) => HeadState::Branch(name.to_owned()),
        (None, Some(oid)) => HeadState::Detached(oid.to_owned()),
        _ => HeadState::default(),
    };
    Ok(status)
}

/// Parse `git diff --numstat -z`. Rename/copy records have an empty path in
/// the first record followed by the old and new paths as separate NUL records;
/// the statistics belong to the new path shown by porcelain status.
pub fn parse_numstat(input: &[u8]) -> GitResult<BTreeMap<PathBuf, DiffStat>> {
    let records = input.split(|byte| *byte == 0).collect::<Vec<_>>();
    let mut stats = BTreeMap::new();
    let mut index = 0;
    while index < records.len() {
        let record = records[index];
        index += 1;
        if record.is_empty() {
            continue;
        }
        let fields = record.splitn(3, |byte| *byte == b'\t').collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(invalid(record));
        }
        let binary = fields[0] == b"-" || fields[1] == b"-";
        let additions = if binary {
            0
        } else {
            String::from_utf8_lossy(fields[0])
                .parse()
                .map_err(|_| invalid(record))?
        };
        let deletions = if binary {
            0
        } else {
            String::from_utf8_lossy(fields[1])
                .parse()
                .map_err(|_| invalid(record))?
        };
        let path = if fields[2].is_empty() {
            // Skip the old path and associate the stat with the new path.
            index += 1;
            let new_path = records.get(index).copied().ok_or_else(|| invalid(record))?;
            index += 1;
            bytes_to_path(new_path)
        } else {
            bytes_to_path(fields[2])
        };
        stats.insert(
            path,
            DiffStat {
                additions,
                deletions,
                binary,
            },
        );
    }
    Ok(stats)
}

fn parse_header(
    record: &[u8],
    status: &mut ParsedStatus,
    branch_oid: &mut Option<String>,
    branch_name: &mut Option<String>,
) -> GitResult<()> {
    let text = std::str::from_utf8(record).map_err(|_| invalid(record))?;
    if let Some(value) = text.strip_prefix("# branch.oid ") {
        *branch_oid = Some(value.to_owned());
    } else if let Some(value) = text.strip_prefix("# branch.head ") {
        *branch_name = Some(value.to_owned());
    } else if let Some(value) = text.strip_prefix("# branch.upstream ") {
        status.upstream = Some(UpstreamState {
            name: value.to_owned(),
        });
    } else if let Some(value) = text.strip_prefix("# branch.ab ") {
        for item in value.split_whitespace() {
            if let Some(value) = item.strip_prefix('+') {
                status.ahead = value.parse().map_err(|_| invalid(record))?;
            } else if let Some(value) = item.strip_prefix('-') {
                status.behind = value.parse().map_err(|_| invalid(record))?;
            }
        }
    }
    Ok(())
}

fn split_prefix(record: &[u8], spaces: usize) -> GitResult<Vec<&[u8]>> {
    let mut fields = Vec::with_capacity(spaces + 1);
    let mut start = 0;
    for (index, byte) in record.iter().enumerate() {
        if *byte == b' ' && fields.len() < spaces {
            fields.push(&record[start..index]);
            start = index + 1;
        }
    }
    fields.push(&record[start..]);
    if fields.len() != spaces + 1 {
        return Err(invalid(record));
    }
    Ok(fields)
}

fn change_at(xy: &[u8], index: usize) -> ChangeKind {
    xy.get(index)
        .copied()
        .map(ChangeKind::from_porcelain)
        .unwrap_or_default()
}

fn conflict_kind(xy: &[u8]) -> ConflictKind {
    match xy {
        b"AA" => ConflictKind::BothAdded,
        b"DD" => ConflictKind::BothDeleted,
        b"UU" => ConflictKind::BothModified,
        b"AU" => ConflictKind::AddedByUs,
        b"UA" => ConflictKind::AddedByThem,
        b"DU" => ConflictKind::DeletedByUs,
        b"UD" => ConflictKind::DeletedByThem,
        _ => ConflictKind::Other,
    }
}

fn invalid(record: &[u8]) -> GitError {
    GitError::new(GitErrorKind::InvalidOutput, "Invalid porcelain v2 output.")
        .with_detail(String::from_utf8_lossy(record))
}

#[cfg(unix)]
fn bytes_to_path(value: &[u8]) -> PathBuf {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(OsString::from_vec(value.to_vec()))
}

#[cfg(not(unix))]
fn bytes_to_path(value: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(value).into_owned())
}

pub fn parse_log(input: &str) -> GitResult<Vec<CommitSummary>> {
    input
        .split('\x1e')
        .filter(|record| !record.trim().is_empty())
        .map(|record| {
            let fields = record
                .trim_start_matches(['\r', '\n'])
                .split('\x1f')
                .collect::<Vec<_>>();
            if fields.len() < 7 {
                return Err(GitError::new(
                    GitErrorKind::InvalidOutput,
                    "Invalid Git log output.",
                ));
            }
            Ok(CommitSummary {
                oid: fields[0].to_owned(),
                parents: fields[1].split_whitespace().map(str::to_owned).collect(),
                author: fields[2].to_owned(),
                author_email: fields[3].to_owned(),
                timestamp: fields[4].parse().unwrap_or_default(),
                subject: fields[5].to_owned(),
                decorations: fields[6]
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect(),
            })
        })
        .collect()
}

pub fn parse_branches(input: &[u8]) -> GitResult<Vec<BranchInfo>> {
    input
        .split(|byte| *byte == b'\n')
        .map(|record| record.strip_suffix(b"\r").unwrap_or(record))
        .filter(|record| !record.is_empty())
        .map(|record| {
            let text = String::from_utf8_lossy(record);
            let fields = text.split('\t').collect::<Vec<_>>();
            if fields.len() < 5 {
                return Err(invalid(record));
            }
            Ok(BranchInfo {
                name: fields[1].to_owned(),
                remote: fields[0].starts_with("refs/remotes/"),
                current: fields[2] == "*",
                oid: fields[3].to_owned(),
                upstream: (!fields[4].is_empty()).then(|| fields[4].to_owned()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn parses_headers_xy_rename_conflict_and_special_paths() {
        let input = b"# branch.oid abcdef\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -3\0\
1 .M N... 100644 100644 100644 a b src/main.rs\0\
2 R. N... 100644 100644 100644 a b R100 new name.rs\0old name.rs\0\
u UU N... 100644 100644 100644 100644 a b c file conflict.rs\0\
? \xe4\xb8\xad\xe6\x96\x87.txt\0";
        let parsed = parse_porcelain_v2(input).unwrap();
        assert_eq!(parsed.head, HeadState::Branch("main".into()));
        assert_eq!(parsed.ahead, 2);
        assert_eq!(parsed.behind, 3);
        assert_eq!(
            parsed.files[&PathBuf::from("src/main.rs")].worktree,
            ChangeKind::Modified
        );
        assert_eq!(
            parsed.files[&PathBuf::from("new name.rs")].original_path,
            Some(PathBuf::from("old name.rs"))
        );
        assert_eq!(
            parsed.files[&PathBuf::from("file conflict.rs")].conflict,
            Some(ConflictKind::BothModified)
        );
        assert!(parsed.files.contains_key(&PathBuf::from("中文.txt")));
    }

    #[test]
    fn parses_unborn_and_detached_heads() {
        assert_eq!(
            parse_porcelain_v2(b"# branch.oid (initial)\0# branch.head topic\0")
                .unwrap()
                .head,
            HeadState::Unborn("topic".into())
        );
        assert_eq!(
            parse_porcelain_v2(b"# branch.oid abc123\0# branch.head (detached)\0")
                .unwrap()
                .head,
            HeadState::Detached("abc123".into())
        );
    }

    #[test]
    fn ignored_paths_stay_out_of_changed_files() {
        let parsed = parse_porcelain_v2(b"? newdir/\0! target/\0! src/a.log\0").unwrap();
        assert_eq!(parsed.files.len(), 1);
        assert!(parsed.ignored.contains(Path::new("target")));
        assert!(parsed.ignored.contains(Path::new("src/a.log")));
    }
}
