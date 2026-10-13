//! Pure matching, glob policy, previews and stale-line location.
use super::MAX_MATCHES;
use super::types::*;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use regex::{Regex, RegexBuilder};
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use unicode_segmentation::UnicodeSegmentation;
const PREVIEW_GRAPHEMES: usize = 160;

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

    pub(crate) fn excludes(&self, relative: &Path) -> bool {
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
    crate::domain::document_snapshot::content_hash(text)
}
