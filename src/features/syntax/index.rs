use super::{TokenSpan, Tokens};
use crate::domain::{identity::FileId, language::ResolvedLanguage};
use loom_editor_core::text::{TextBuffer, TextRevision};
use std::{
    collections::{HashMap, VecDeque},
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone)]
pub(crate) struct Source {
    pub text: Arc<String>,
    pub version: u64,
    revisions: Arc<VecDeque<TextRevision>>,
}
impl Source {
    pub fn new(buffer: &TextBuffer) -> Self {
        Self {
            text: buffer.text_snapshot(),
            version: buffer.version(),
            revisions: buffer.revision_log(),
        }
    }
    pub fn same(&self, other: &Self) -> bool {
        self.version == other.version && Arc::ptr_eq(&self.text, &other.text)
    }
    pub fn identity(&self) -> usize {
        Arc::as_ptr(&self.text) as usize
    }
    fn changes_since(&self, old: &Self) -> Option<Vec<TextRevision>> {
        let mut current = (old.version, old.identity());
        let mut changes = Vec::new();
        for revision in self
            .revisions
            .iter()
            .filter(|r| r.after_version > old.version)
        {
            if (revision.before_version, revision.before_snapshot) != current {
                return None;
            }
            current = (revision.after_version, revision.after_snapshot);
            changes.push(revision.clone());
        }
        (current == (self.version, self.identity())).then_some(changes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Window {
    pub line: Range<usize>,
    pub range: Range<usize>,
}
#[derive(Clone, Debug)]
pub(crate) struct WindowTokens {
    pub range: Range<usize>,
    pub spans: Arc<[TokenSpan]>,
    /// Known spans remain useful while another part of this range is pending.
    pub complete: bool,
}

#[derive(Default, Clone, Copy, Debug)]
pub(crate) struct IndexStats {
    pub scanned_bytes: usize,
    pub reused_tokens: usize,
    pub reused_lines: usize,
    pub evictions: usize,
    pub bytes: usize,
}
struct Line {
    range: Range<usize>,
    tokens: Vec<TokenSpan>,
    future: VecDeque<TokenSpan>,
    trusted_tail: bool,
}
impl Line {
    fn new(range: Range<usize>) -> Self {
        Self {
            range,
            tokens: Vec::new(),
            future: VecDeque::new(),
            trusted_tail: false,
        }
    }
    fn scanned(&self) -> usize {
        self.tokens
            .last()
            .map_or(self.range.start, |span| span.range.end)
    }
    fn rebase(&mut self, revision: &TextRevision) -> bool {
        let touching: Vec<_> = revision
            .edits
            .iter()
            .filter(|edit| edit.old.start <= self.range.end && edit.old.end >= self.range.start)
            .collect();
        if touching.iter().any(|edit| edit.line_breaks) {
            return false;
        }
        let spans: Vec<_> = self.tokens.drain(..).chain(self.future.drain(..)).collect();
        let mut invalid = vec![false; spans.len()];
        for edit in touching {
            let first = spans.partition_point(|span| span.range.end < edit.old.start);
            let end = spans.partition_point(|span| span.range.start <= edit.old.end);
            // Token classification can depend on the preceding word/dot and
            // the following parenthesis; re-evaluate neighbours as well.
            let lo = first.saturating_sub(2);
            let hi = (end + 2).min(spans.len());
            invalid[lo..hi].fill(true);
        }
        self.range = revision.map(self.range.start, false)..revision.map(self.range.end, true);
        let mut cursor = self.range.start;
        for (span, invalid) in spans.into_iter().zip(invalid) {
            if invalid {
                continue;
            }
            let span = TokenSpan {
                range: revision.map(span.range.start, false)..revision.map(span.range.end, true),
                kind: span.kind,
            };
            if self.future.is_empty() && span.range.start == cursor {
                cursor = span.range.end;
                self.tokens.push(span);
            } else {
                self.future.push_back(span);
            }
        }
        self.trusted_tail = false;
        true
    }
    fn ensure(
        &mut self,
        source: &str,
        lang: ResolvedLanguage,
        end: usize,
        cancel: &AtomicBool,
        stats: &mut IndexStats,
    ) -> bool {
        while self.scanned() < end {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            let cursor = self.scanned();
            while self
                .future
                .front()
                .is_some_and(|span| span.range.end <= cursor)
            {
                self.future.pop_front();
            }
            if self.trusted_tail
                && self
                    .future
                    .front()
                    .is_some_and(|span| span.range.start == cursor)
            {
                self.tokens.push(self.future.pop_front().unwrap());
                stats.reused_tokens += 1;
                continue;
            }
            let text = &source[self.range.clone()];
            let Some((range, kind)) = Tokens::new(text, lang, cursor - self.range.start)
                .cancellable(cancel)
                .next()
            else {
                return !cancel.load(Ordering::Relaxed);
            };
            stats.scanned_bytes += range.len();
            let span = TokenSpan {
                range: self.range.start + range.start..self.range.start + range.end,
                kind,
            };
            while self
                .future
                .front()
                .is_some_and(|old| old.range.start < span.range.start)
            {
                self.future.pop_front();
            }
            self.trusted_tail = self.future.front() == Some(&span);
            while self
                .future
                .front()
                .is_some_and(|old| old.range.start < span.range.end)
            {
                self.future.pop_front();
            }
            self.tokens.push(span);
        }
        true
    }
    fn window(&self, range: Range<usize>) -> WindowTokens {
        let first = self
            .tokens
            .partition_point(|span| span.range.end <= range.start);
        WindowTokens {
            complete: true,
            spans: self.tokens[first..]
                .iter()
                .take_while(|span| span.range.start < range.end)
                .cloned()
                .collect::<Vec<_>>()
                .into(),
            range,
        }
    }
}

struct Document {
    source: Source,
    lang: ResolvedLanguage,
    lines: HashMap<Range<usize>, Line>,
    used: u64,
}
pub(crate) struct Index {
    documents: HashMap<FileId, Document>,
    clock: u64,
    budget: usize,
    pub stats: IndexStats,
}
impl Default for Index {
    fn default() -> Self {
        Self::new(16 * 1024 * 1024)
    }
}
impl Index {
    pub fn new(budget: usize) -> Self {
        Self {
            documents: HashMap::new(),
            clock: 0,
            budget,
            stats: IndexStats::default(),
        }
    }
    pub fn retain(&mut self, open: &[FileId]) {
        self.documents.retain(|id, _| open.contains(id));
        self.account();
    }
    pub fn prepare(
        &mut self,
        id: FileId,
        source: Source,
        lang: ResolvedLanguage,
        windows: &[Window],
        cancel: &AtomicBool,
    ) -> Option<Vec<WindowTokens>> {
        self.clock += 1;
        let doc = self.documents.entry(id).or_insert_with(|| Document {
            source: source.clone(),
            lang,
            lines: HashMap::new(),
            used: self.clock,
        });
        doc.used = self.clock;
        if !doc.source.same(&source) || doc.lang != lang {
            if doc.lang == lang
                && let Some(changes) = source.changes_since(&doc.source)
            {
                for revision in &changes {
                    doc.lines = std::mem::take(&mut doc.lines)
                        .into_values()
                        .filter_map(|mut line| {
                            line.rebase(revision).then(|| (line.range.clone(), line))
                        })
                        .collect();
                }
            } else {
                doc.lines.clear();
            }
            doc.source = source;
            doc.lang = lang;
        }
        let mut ready = Vec::with_capacity(windows.len());
        for window in windows {
            if window.range.start < window.line.start
                || window.range.end > window.line.end
                || window.line.end > doc.source.text.len()
            {
                return None;
            }
            let line = doc
                .lines
                .entry(window.line.clone())
                .or_insert_with(|| Line::new(window.line.clone()));
            if line.scanned() >= window.range.end {
                self.stats.reused_lines += 1;
            }
            if !line.ensure(
                &doc.source.text,
                lang,
                window.range.end,
                cancel,
                &mut self.stats,
            ) {
                return None;
            }
            ready.push(line.window(window.range.clone()));
        }
        self.account();
        while self.stats.bytes > self.budget || self.documents.len() > 32 {
            let Some(oldest) = self
                .documents
                .iter()
                .min_by_key(|(_, doc)| doc.used)
                .map(|(id, _)| *id)
            else {
                break;
            };
            self.documents.remove(&oldest);
            self.stats.evictions += 1;
            self.account();
        }
        Some(ready)
    }
    fn account(&mut self) {
        self.stats.bytes = self
            .documents
            .values()
            .map(|doc| {
                doc.lines
                    .values()
                    .map(|line| {
                        (line.tokens.capacity() + line.future.capacity())
                            * std::mem::size_of::<TokenSpan>()
                    })
                    .sum::<usize>()
            })
            .sum();
        // Bound retained source allocations too, including documents closed
        // before the next application prune notification arrives.
        let retained: usize = self
            .documents
            .values()
            .map(|doc| doc.source.text.len())
            .sum();
        if retained > 128 * 1024 * 1024 {
            self.stats.bytes = self.budget.saturating_add(1);
        }
    }
}
