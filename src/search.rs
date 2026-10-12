//! Bounded, cancellable workspace content search. No UI or Explorer cache access.
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use regex::{Regex, RegexBuilder};
use unicode_segmentation::UnicodeSegmentation;

pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_MATCHES: usize = 10_000;
pub const MAX_FILES: usize = 5_000;
const PREVIEW_GRAPHEMES: usize = 160;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchSpec {
    pub query: String,
    pub include: String,
    pub exclude: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub include_hidden: bool,
    pub include_ignored: bool,
}

#[derive(Clone)]
pub struct BufferSnapshot {
    pub path: PathBuf,
    pub text: Arc<String>,
    pub version: u64,
}

#[derive(Clone, Debug)]
pub struct Match {
    pub range: Range<usize>,
    pub line: usize,
    pub column: usize,
    pub preview: String,
    pub highlight: Range<usize>,
    pub line_hash: u64,
}

#[derive(Clone, Debug)]
pub struct FileMatches {
    pub path: PathBuf,
    pub root: PathBuf,
    pub matches: Vec<Match>,
    pub content_hash: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Results {
    pub files: Vec<Arc<FileMatches>>,
    pub matches: usize,
    pub searched: usize,
    pub skipped: usize,
    pub skipped_binary: usize,
    pub skipped_encoding: usize,
    pub skipped_large: usize,
    pub errors: usize,
    pub first_error: Option<String>,
    pub limit_hit: bool,
    pub cancelled: bool,
    pub complete: bool,
}

pub struct Request {
    pub spec: SearchSpec,
    pub roots: Vec<PathBuf>,
    pub buffers: Vec<BufferSnapshot>,
}

pub struct Matcher {
    regex: Regex,
    include: GlobSet,
    exclude: GlobSet,
    has_include: bool,
}

impl Matcher {
    pub fn new(spec: &SearchSpec) -> Result<Self, String> {
        let pattern = if spec.regex {
            spec.query.clone()
        } else {
            regex::escape(&spec.query)
        };
        let pattern = if spec.whole_word {
            format!(r"\b(?:{pattern})\b")
        } else {
            pattern
        };
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(!spec.case_sensitive)
            .multi_line(true)
            .crlf(true)
            .size_limit(8 * 1024 * 1024)
            .build()
            .map_err(|error| format!("Invalid search pattern: {error}"))?;
        let include = compile_globs(&spec.include)?;
        let exclude = compile_globs(&spec.exclude)?;
        Ok(Self {
            regex,
            include,
            exclude,
            has_include: !spec.include.trim().is_empty(),
        })
    }

    pub fn includes(&self, relative: &Path) -> bool {
        (!self.has_include || self.include.is_match(relative)) && !self.excludes(relative)
    }

    fn excludes(&self, relative: &Path) -> bool {
        relative
            .ancestors()
            .filter(|path| !path.as_os_str().is_empty())
            .any(|path| self.exclude.is_match(path))
    }

    /// First version is line oriented; a regex cannot match across line breaks.
    pub fn matches(&self, text: &str, cancel: &AtomicBool, limit: usize) -> Vec<Match> {
        let mut matches = Vec::new();
        let mut offset = 0;
        for (line, with_break) in text.split_inclusive('\n').enumerate() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let value = with_break.strip_suffix('\n').unwrap_or(with_break);
            let value = value.strip_suffix('\r').unwrap_or(value);
            let mut found_matches = self.regex.find_iter(value).peekable();
            if found_matches.peek().is_none() {
                offset += with_break.len();
                continue;
            }
            let boundaries = value
                .grapheme_indices(true)
                .map(|(offset, _)| offset)
                .chain(std::iter::once(value.len()))
                .collect::<Vec<_>>();
            let line_hash = hash(value);
            for found in found_matches {
                if cancel.load(Ordering::Relaxed) || matches.len() >= limit {
                    return matches;
                }
                let (preview, highlight) = preview(value, found.range(), &boundaries);
                matches.push(Match {
                    range: offset + found.start()..offset + found.end(),
                    line,
                    column: found.start(),
                    preview,
                    highlight,
                    line_hash,
                });
            }
            offset += with_break.len();
        }
        // split_inclusive has no last empty line. Permit $/^ matches there.
        if (text.is_empty() || text.ends_with('\n'))
            && matches.len() < limit
            && !cancel.load(Ordering::Relaxed)
            && self.regex.is_match("")
        {
            matches.push(Match {
                range: text.len()..text.len(),
                line: text.bytes().filter(|b| *b == b'\n').count(),
                column: 0,
                preview: String::new(),
                highlight: 0..0,
                line_hash: hash(""),
            });
        }
        matches
    }

    /// Unchanged lines can move after an edit. Never select an unrelated line.
    pub fn locate(&self, text: &str, file: &FileMatches, found: &Match) -> Option<Range<usize>> {
        if hash(text) == file.content_hash {
            return Some(found.range.clone());
        }
        let candidates = self.matches(text, &AtomicBool::new(false), MAX_MATCHES + 1);
        candidates
            .into_iter()
            .filter(|candidate| {
                candidate.line_hash == found.line_hash && candidate.column == found.column
            })
            .min_by_key(|candidate| candidate.line.abs_diff(found.line))
            .map(|candidate| candidate.range)
    }
}

fn compile_globs(value: &str) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    for pattern in split_globs(value) {
        let pattern = pattern.replace('\\', "/");
        let glob = GlobBuilder::new(&pattern)
            .literal_separator(false)
            .build()
            .map_err(|error| format!("Invalid file filter: {error}"))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|error| format!("Invalid file filter: {error}"))
}

fn split_globs(value: &str) -> Vec<&str> {
    let mut start = 0;
    let mut depth: usize = 0;
    let mut output = Vec::new();
    for (index, ch) in value.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                output.push(value[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    output.push(value[start..].trim());
    output
        .into_iter()
        .filter(|pattern| !pattern.is_empty())
        .collect()
}

fn preview(line: &str, found: Range<usize>, boundaries: &[usize]) -> (String, Range<usize>) {
    let at = boundaries
        .partition_point(|offset| *offset <= found.start)
        .saturating_sub(1);
    let first = at.saturating_sub(40);
    let last = (first + PREVIEW_GRAPHEMES).min(boundaries.len() - 1);
    let start = boundaries[first];
    let end = boundaries[last];
    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if end < line.len() { "…" } else { "" };
    let preview = format!("{prefix}{}{suffix}", &line[start..end]);
    let highlight = prefix.len() + found.start.saturating_sub(start).min(end - start)
        ..prefix.len() + found.end.saturating_sub(start).min(end - start);
    (preview, highlight)
}

pub fn hash(text: &str) -> u64 {
    crate::model::document::content_hash(text)
}

pub fn normalized_path(path: &Path) -> PathBuf {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    #[cfg(windows)]
    {
        let value = path.to_string_lossy();
        if let Some(value) = value.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{value}"));
        }
        if let Some(value) = value.strip_prefix(r"\\?\") {
            return PathBuf::from(value);
        }
    }
    path
}

/// Bound the actual read as well as metadata: a file may grow during searching.
pub fn read_text(path: &Path) -> Result<String, String> {
    let file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Not a regular file".into());
    }
    if metadata.len() > MAX_FILE_BYTES as u64 {
        return Err("File is larger than 4 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("File is larger than 4 MiB".into());
    }
    if bytes.contains(&0) {
        return Err("Binary file".into());
    }
    String::from_utf8(bytes).map_err(|_| "File is not UTF-8".into())
}

fn search_walk(root: &Path, spec: &SearchSpec, matcher: Arc<Matcher>) -> WalkBuilder {
    let mut walk = WalkBuilder::new(root);
    walk.hidden(!spec.include_hidden)
        .git_ignore(!spec.include_ignored)
        .git_global(!spec.include_ignored)
        .git_exclude(!spec.include_ignored)
        .ignore(!spec.include_ignored)
        .parents(!spec.include_ignored)
        .require_git(false)
        .follow_links(false);
    if !spec.include_ignored {
        walk.add_custom_ignore_filename(".rgignore");
    }
    let root = root.to_path_buf();
    walk.filter_entry(move |entry| {
        entry.depth() == 0
            || (entry.file_name() != ".git"
                && !matcher.excludes(entry.path().strip_prefix(&root).unwrap_or(entry.path())))
    });
    walk
}

/// Evaluate watcher paths off the UI thread, using the same ignore rules as
/// searching. Only walk their ancestor branches; never scan file contents.
/// Missing paths conservatively use their nearest remaining parent so removing
/// or renaming a directory cannot leave results for its children behind.
pub fn paths_affect_search(spec: &SearchSpec, roots: &[PathBuf], paths: &[PathBuf]) -> bool {
    let Ok(matcher) = Matcher::new(spec).map(Arc::new) else {
        return false;
    };
    for root in roots {
        let root = normalized_path(root);
        let mut candidates = HashSet::new();
        for path in paths {
            let path = normalized_path(path);
            let Ok(relative) = path.strip_prefix(&root) else {
                continue;
            };
            if relative.components().any(|part| part.as_os_str() == ".git") {
                continue;
            }
            if !spec.include_ignored
                && path.file_name().is_some_and(|name| {
                    name == ".gitignore" || name == ".ignore" || name == ".rgignore"
                })
            {
                return true;
            }
            if matcher.excludes(relative)
                || (!spec.include_hidden
                    && relative
                        .components()
                        .any(|part| part.as_os_str().to_string_lossy().starts_with('.')))
            {
                continue;
            }
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => continue,
                Ok(metadata) if metadata.is_file() && !matcher.includes(relative) => continue,
                _ => {}
            }
            let candidate = path
                .ancestors()
                .find(|parent| parent.exists())
                .unwrap_or(&root);
            candidates.insert(candidate.to_path_buf());
        }
        if candidates.is_empty() {
            continue;
        }
        let ancestors = candidates
            .iter()
            .flat_map(|path| path.ancestors().map(Path::to_path_buf))
            .collect::<HashSet<_>>();
        let mut walk = search_walk(&root, spec, matcher.clone());
        // filter_entry replaces the previous callback, so retain its explicit
        // exclusion checks as well as pruning unrelated ancestor branches.
        let filter = matcher.clone();
        let filter_root = root.clone();
        walk.filter_entry(move |entry| {
            ancestors.contains(entry.path())
                && (entry.depth() == 0
                    || (entry.file_name() != ".git"
                        && !filter.excludes(
                            entry
                                .path()
                                .strip_prefix(&filter_root)
                                .unwrap_or(entry.path()),
                        )))
        });
        for entry in walk.build() {
            match entry {
                Ok(entry) if candidates.contains(entry.path()) => return true,
                Err(_) => return true,
                _ => {}
            }
        }
    }
    false
}

/// Publications are cumulative snapshots, safe for a latest-only UI mailbox.
pub fn run(
    request: Request,
    cancel: &AtomicBool,
    mut publish: impl FnMut(Results),
) -> Result<(), String> {
    let matcher = Arc::new(Matcher::new(&request.spec)?);
    let buffers = request
        .buffers
        .into_iter()
        .map(|buffer| (normalized_path(&buffer.path), buffer))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let mut results = Results::default();
    let mut last_publish = Instant::now();
    for root in request.roots {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let root = normalized_path(&root);
        if !root.is_dir() {
            record_error(
                &mut results,
                format!("{}: folder is unavailable", root.display()),
            );
            continue;
        }
        let walk = search_walk(&root, &request.spec, matcher.clone());
        let stop = cancel;
        for entry in walk.build() {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    record_error(&mut results, error.to_string());
                    continue;
                }
            };
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let relative = entry.path().strip_prefix(&root).unwrap_or(entry.path());
            if !matcher.includes(relative) {
                continue;
            }
            let mut path = normalized_path(entry.path());
            if !seen.insert(path.clone()) {
                continue;
            }
            let text = if let Some(buffer) = buffers.get(&path) {
                path = buffer.path.clone();
                buffer.text.clone()
            } else {
                match read_text(&path) {
                    Ok(text) => Arc::new(text),
                    Err(error)
                        if matches!(
                            error.as_str(),
                            "Binary file" | "File is not UTF-8" | "File is larger than 4 MiB"
                        ) =>
                    {
                        results.skipped += 1;
                        match error.as_str() {
                            "Binary file" => results.skipped_binary += 1,
                            "File is not UTF-8" => results.skipped_encoding += 1,
                            _ => results.skipped_large += 1,
                        }
                        continue;
                    }
                    Err(error) => {
                        record_error(&mut results, error);
                        continue;
                    }
                }
            };
            if text.len() > MAX_FILE_BYTES {
                results.skipped += 1;
                results.skipped_large += 1;
                continue;
            }
            if text.as_bytes().contains(&0) {
                results.skipped += 1;
                results.skipped_binary += 1;
                continue;
            }
            results.searched += 1;
            let remaining = MAX_MATCHES.saturating_sub(results.matches);
            let mut matches = matcher.matches(&text, cancel, remaining + 1);
            if matches.len() > remaining {
                results.limit_hit = true;
                matches.truncate(remaining);
            }
            if !matches.is_empty() {
                if results.files.len() >= MAX_FILES {
                    results.limit_hit = true;
                    break;
                }
                results.matches += matches.len();
                results.files.push(Arc::new(FileMatches {
                    path,
                    root: root.clone(),
                    matches,
                    content_hash: hash(&text),
                }));
            }
            if last_publish.elapsed() >= Duration::from_millis(60) {
                publish(results.clone());
                last_publish = Instant::now();
            }
            if results.limit_hit {
                break;
            }
        }
        if results.limit_hit {
            break;
        }
    }
    results.files.sort_by(|a, b| a.path.cmp(&b.path));
    results.cancelled = cancel.load(Ordering::Relaxed);
    results.complete = true;
    publish(results);
    Ok(())
}

fn record_error(results: &mut Results, error: String) {
    results.errors += 1;
    if results.first_error.is_none() {
        results.first_error = Some(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn spec(query: &str) -> SearchSpec {
        SearchSpec {
            query: query.into(),
            include: String::new(),
            exclude: String::new(),
            case_sensitive: false,
            whole_word: false,
            regex: false,
            include_hidden: false,
            include_ignored: false,
        }
    }
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "loom-search-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn scan(root: &Path, spec: SearchSpec, buffers: Vec<BufferSnapshot>) -> Results {
        let mut result = Results::default();
        run(
            Request {
                spec,
                roots: vec![root.into(), root.join("nested")],
                buffers,
            },
            &AtomicBool::new(false),
            |next| result = next,
        )
        .unwrap();
        result
    }
    #[test]
    fn watcher_scope_uses_search_filters_and_ignore_rules() {
        let temp = Temp::new();
        let root = &temp.0;
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join(".gitignore"), "target/\nignored.rs\n").unwrap();
        std::fs::write(root.join("nested/.rgignore"), "skip.rs\n").unwrap();
        for name in [
            "a.rs",
            "ignored.rs",
            ".hidden.rs",
            "target/generated.rs",
            "nested/skip.rs",
            "nested/b.rs",
            "b.txt",
        ] {
            std::fs::write(root.join(name), "needle").unwrap();
        }
        let mut query = spec("needle");
        query.include = "*.rs".into();
        let affects = |query: &SearchSpec, name: &str| {
            paths_affect_search(query, std::slice::from_ref(root), &[root.join(name)])
        };
        assert!(affects(&query, "a.rs"));
        assert!(affects(&query, "nested/b.rs"));
        for name in [
            "ignored.rs",
            ".hidden.rs",
            "target/generated.rs",
            "nested/skip.rs",
            "b.txt",
            ".git/index",
        ] {
            assert!(
                !affects(&query, name),
                "{name} should not invalidate search"
            );
        }
        assert!(affects(&query, ".gitignore"));
        query.include_hidden = true;
        assert!(affects(&query, ".hidden.rs"));
        query.include_ignored = true;
        assert!(affects(&query, "target/generated.rs"));
        assert!(affects(&query, "nested/skip.rs"));
        query.exclude = "target/**, nested/**".into();
        assert!(!affects(&query, "target/generated.rs"));
        assert!(!affects(&query, "nested/b.rs"));
        query.include_ignored = false;
        std::fs::remove_file(root.join("target/generated.rs")).unwrap();
        assert!(!affects(&query, "target/generated.rs"));
        query.exclude.clear();
        assert!(!affects(&query, "target/generated.rs"));
        std::fs::remove_file(root.join("a.rs")).unwrap();
        assert!(affects(&query, "a.rs"));
        std::fs::remove_dir_all(root.join("nested")).unwrap();
        assert!(affects(&query, "nested"));
    }
    #[test]
    fn unopened_nested_files_ignore_filters_and_buffer_override() {
        let temp = Temp::new();
        let root = &temp.0;
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        for (path, text) in [
            ("nested/a.rs", "disk"),
            ("ignored.txt", "needle"),
            (".hidden", "needle"),
            ("other.txt", "needle"),
            ("binary", "needle\0"),
        ] {
            std::fs::write(root.join(path), text).unwrap();
        }
        let buffer = BufferSnapshot {
            path: root.join("nested/a.rs"),
            text: Arc::new("needle 中文".into()),
            version: 1,
        };
        let mut query = spec("needle");
        query.include = "*.rs".into();
        let result = scan(root, query, vec![buffer.clone()]);
        assert_eq!(result.matches, 1);
        assert_eq!(result.files.len(), 1);
        let result = scan(root, spec("needle"), vec![buffer]);
        assert_eq!(result.matches, 2);
        assert_eq!(result.skipped, 1);
        let mut query = spec("needle");
        query.include_hidden = true;
        query.include_ignored = true;
        assert_eq!(scan(root, query, vec![]).matches, 3);
    }
    #[test]
    fn chinese_crlf_literal_regex_and_relocation() {
        let mut query = spec("中文");
        query.case_sensitive = true;
        let matcher = Matcher::new(&query).unwrap();
        let text = "abc\r\n中文😀中文\r\n";
        let matches = matcher.matches(text, &AtomicBool::new(false), 20);
        assert_eq!(matches[0].range, 5..11);
        assert_eq!(matches[0].line, 1);
        let file = FileMatches {
            path: PathBuf::new(),
            root: PathBuf::new(),
            matches: matches.clone(),
            content_hash: hash(text),
        };
        assert_eq!(
            matcher.locate(&format!("new\n{text}"), &file, &matches[1]),
            Some(19..25)
        );
        assert!(
            matcher
                .locate("different 中文", &file, &matches[0])
                .is_none()
        );
        query.query = "[".into();
        query.regex = true;
        assert!(Matcher::new(&query).is_err());
        query.regex = false;
        assert!(Matcher::new(&query).is_ok());
    }
    #[test]
    fn filters_whole_word_zero_width_and_result_limit() {
        let mut query = spec("one");
        query.whole_word = true;
        query.include = "src/**/*.{rs,ts}, *.txt".into();
        query.exclude = "vendor/**".into();
        let matcher = Matcher::new(&query).unwrap();
        assert!(matcher.includes(Path::new("src/lib/main.rs")));
        assert!(matcher.includes(Path::new("nested/a.txt")));
        assert!(!matcher.includes(Path::new("vendor/a.txt")));
        assert_eq!(
            matcher
                .matches("ONE someone one", &AtomicBool::new(false), 1)
                .len(),
            1
        );
        query.query = "^".into();
        query.regex = true;
        query.whole_word = false;
        assert_eq!(
            Matcher::new(&query)
                .unwrap()
                .matches("中文\n", &AtomicBool::new(false), 10)
                .len(),
            2
        );
        let temp = Temp::new();
        std::fs::write(temp.0.join("many"), "a ".repeat(MAX_MATCHES + 2)).unwrap();
        let result = scan(&temp.0, spec("a"), vec![]);
        assert_eq!(result.matches, MAX_MATCHES);
        assert!(result.limit_hit);
    }
    #[test]
    fn cancellation_is_observed_between_files() {
        let temp = Temp::new();
        std::fs::write(temp.0.join("a"), "needle").unwrap();
        let mut result = Results::default();
        run(
            Request {
                spec: spec("needle"),
                roots: vec![temp.0.clone()],
                buffers: vec![],
            },
            &AtomicBool::new(true),
            |next| result = next,
        )
        .unwrap();
        assert!(result.cancelled);
        assert_eq!(result.searched, 0);
    }
}
