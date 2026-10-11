use std::path::PathBuf;

use super::error::{GitError, GitErrorKind, GitResult};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnifiedDiff {
    pub files: Vec<FileDiff>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    pub old_path: Option<PathBuf>,
    pub new_path: Option<PathBuf>,
    pub hunks: Vec<DiffHunk>,
    pub binary: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffLine {
    Context(String),
    Addition(String),
    Deletion(String),
    NoNewline,
}

pub fn parse_unified(input: &str) -> GitResult<UnifiedDiff> {
    let mut result = UnifiedDiff::default();
    let mut file = None::<FileDiff>;
    let mut hunk = None::<DiffHunk>;
    for line in input.lines() {
        if line.starts_with("diff --git ") {
            finish_hunk(&mut file, &mut hunk);
            if let Some(previous) = file.take() {
                result.files.push(previous);
            }
            file = Some(FileDiff::default());
        } else if let Some(path) = line.strip_prefix("--- ") {
            file.get_or_insert_with(FileDiff::default).old_path = parse_diff_path(path);
        } else if let Some(path) = line.strip_prefix("+++ ") {
            file.get_or_insert_with(FileDiff::default).new_path = parse_diff_path(path);
        } else if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            file.get_or_insert_with(FileDiff::default).binary = true;
        } else if line.starts_with("@@ ") {
            finish_hunk(&mut file, &mut hunk);
            hunk = Some(parse_hunk_header(line)?);
        } else if let Some(current) = hunk.as_mut() {
            let parsed = match line.as_bytes().first() {
                Some(b'+') => DiffLine::Addition(line[1..].to_owned()),
                Some(b'-') => DiffLine::Deletion(line[1..].to_owned()),
                Some(b' ') => DiffLine::Context(line[1..].to_owned()),
                Some(b'\\') => DiffLine::NoNewline,
                _ => DiffLine::Context(line.to_owned()),
            };
            current.lines.push(parsed);
        }
    }
    finish_hunk(&mut file, &mut hunk);
    if let Some(file) = file {
        result.files.push(file);
    }
    Ok(result)
}

/// One hunk of a raw diff: its ranges and its exact text, header included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawHunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub text: String,
}

/// A single-file raw diff: the file header and its hunks, with line endings
/// kept byte for byte so a hunk can be applied back as a patch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawFileDiff {
    pub header: String,
    pub hunks: Vec<RawHunk>,
}

impl RawFileDiff {
    /// A patch of `hunk` alone, or `None` for diffs whose header does more
    /// than change lines (creating, deleting or renaming the file).
    pub fn patch(&self, hunk: &RawHunk) -> Option<String> {
        let plain = !self.header.lines().any(|line| {
            [
                "new file mode",
                "deleted file mode",
                "rename from",
                "copy from",
                "Binary files",
                "GIT binary patch",
            ]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        });
        plain.then(|| format!("{}{}", self.header, hunk.text))
    }
}

pub fn split_raw_diff(input: &str) -> GitResult<RawFileDiff> {
    let mut result = RawFileDiff::default();
    for line in input.split_inclusive('\n') {
        if line.starts_with("@@ ") {
            let hunk = parse_hunk_header(line.trim_end())?;
            result.hunks.push(RawHunk {
                old_start: hunk.old_start,
                old_lines: hunk.old_lines,
                new_start: hunk.new_start,
                new_lines: hunk.new_lines,
                text: line.to_owned(),
            });
        } else if let Some(hunk) = result.hunks.last_mut() {
            hunk.text.push_str(line);
        } else {
            result.header.push_str(line);
        }
    }
    Ok(result)
}

fn finish_hunk(file: &mut Option<FileDiff>, hunk: &mut Option<DiffHunk>) {
    if let Some(hunk) = hunk.take() {
        file.get_or_insert_with(FileDiff::default).hunks.push(hunk);
    }
}

fn parse_diff_path(value: &str) -> Option<PathBuf> {
    let value = value.split('\t').next().unwrap_or(value);
    if value == "/dev/null" {
        None
    } else {
        Some(PathBuf::from(
            value
                .strip_prefix("a/")
                .or_else(|| value.strip_prefix("b/"))
                .unwrap_or(value),
        ))
    }
}

fn parse_hunk_header(value: &str) -> GitResult<DiffHunk> {
    let end = value[3..]
        .find(" @@")
        .map(|offset| offset + 3)
        .ok_or_else(|| GitError::new(GitErrorKind::InvalidOutput, "Invalid diff hunk header."))?;
    let range = &value[3..end];
    let mut parts = range.split_whitespace();
    let (old_start, old_lines) = parse_range(parts.next(), '-')?;
    let (new_start, new_lines) = parse_range(parts.next(), '+')?;
    Ok(DiffHunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
        header: value[end + 3..].trim().to_owned(),
        lines: Vec::new(),
    })
}

fn parse_range(value: Option<&str>, prefix: char) -> GitResult<(usize, usize)> {
    let value = value
        .and_then(|value| value.strip_prefix(prefix))
        .ok_or_else(|| GitError::new(GitErrorKind::InvalidOutput, "Invalid diff range."))?;
    let mut values = value.split(',');
    let start = values
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| GitError::new(GitErrorKind::InvalidOutput, "Invalid diff range start."))?;
    let count = values
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    Ok((start, count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_file_and_hunk_ranges() {
        let diff = parse_unified(
            "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,3 @@ heading\n same\n-old\n+new\n+more\n",
        )
        .unwrap();
        assert_eq!(diff.files.len(), 1);
        assert_eq!(diff.files[0].new_path, Some(PathBuf::from("a.txt")));
        assert_eq!(diff.files[0].hunks[0].new_lines, 3);
    }

    #[test]
    fn raw_hunks_keep_their_bytes_and_refuse_file_level_changes() {
        let raw = "diff --git a/a.txt b/a.txt\r\nindex 1..2 100644\n--- a/a.txt\n+++ b/a.txt\n\
@@ -2 +2 @@ ctx\n-old\r\n+new\r\n@@ -5,0 +6,2 @@\n+x\n+y\n\\ No newline at end of file\n";
        let diff = split_raw_diff(raw).unwrap();
        assert_eq!(diff.hunks.len(), 2);
        assert_eq!((diff.hunks[0].old_start, diff.hunks[0].old_lines), (2, 1));
        assert_eq!((diff.hunks[1].new_start, diff.hunks[1].new_lines), (6, 2));
        let patch = diff.patch(&diff.hunks[0]).unwrap();
        assert!(patch.ends_with("@@ -2 +2 @@ ctx\n-old\r\n+new\r\n"));
        assert!(
            diff.hunks[1]
                .text
                .ends_with("\\ No newline at end of file\n")
        );

        let created = split_raw_diff(
            "diff --git a/n b/n\nnew file mode 100644\n--- /dev/null\n+++ b/n\n@@ -0,0 +1 @@\n+n\n",
        )
        .unwrap();
        assert_eq!(created.patch(&created.hunks[0]), None);
    }
}
