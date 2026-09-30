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
}
