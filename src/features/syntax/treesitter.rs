//! Per-document syntax trees and the highlight spans read from them.
//!
//! The worker owns one [`TreeIndex`] and calls it with the same shape as the
//! former line index: a document, its text snapshot and the windows to colour.
//! A tree always describes a whole document, so a comment that starts above the
//! window is still a comment inside it; only the query is limited to the window.
//!
//! The tree is kept up to date from the text's edit journal when it is complete
//! and still follows from the tree's version; otherwise the document is parsed
//! again. Editing happens on a private copy and replaces the stored tree only
//! when the new parse finished, so a cancelled parse leaves the old state intact.
use super::queries::{self, CompiledQuery};
use super::{Source, TokenSpan, Window, WindowTokens, grammars};
use crate::domain::{
    identity::FileId,
    language::{GrammarId, Registry, ResolvedLanguage},
};
use crate::features::highlighting::HighlightId;
use loom_editor_core::text::{BytePoint, RevisionEdit};
use std::{
    cmp::Reverse,
    collections::HashMap,
    ops::{ControlFlow, Range},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use streaming_iterator::StreamingIterator;
use tree_sitter::{InputEdit, ParseOptions, ParseState, Parser, Point, QueryCursor, Tree};

/// Documents larger than this are shown as plain text rather than parsed.
pub(crate) const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
/// Trees kept at once; the least recently used one is dropped first.
const MAX_TREES: usize = 16;
/// Captures read for one window; a window needing more is drawn partly plain.
const MAX_CAPTURES: usize = 200_000;
/// Grammars the tree-sitter path handles. Languages whose grammar is not listed
/// still use the former lexer; P3b lists them all and deletes the lexer.
pub(crate) const ENABLED: &[&str] = &["javascript", "rust"];

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TreeStats {
    pub full_parses: usize,
    pub incremental_parses: usize,
    pub parsed_bytes: usize,
    pub parse_time: Duration,
    pub query_runs: usize,
    pub query_time: Duration,
    pub queries_compiled: usize,
    pub query_compile_time: Duration,
    pub plain_fallbacks: usize,
    pub trees: usize,
}

struct DocTree {
    tree: Tree,
    source: Source,
    grammar: GrammarId,
    used: u64,
}

#[derive(Default)]
pub(crate) struct TreeIndex {
    parsers: HashMap<GrammarId, Parser>,
    /// `None` remembers a query that failed to compile, so it is not retried.
    queries: HashMap<GrammarId, Option<Arc<CompiledQuery>>>,
    docs: HashMap<FileId, DocTree>,
    clock: u64,
    pub stats: TreeStats,
}

impl TreeIndex {
    /// Whether this path colours `language`.
    pub(crate) fn handles(language: ResolvedLanguage) -> bool {
        language
            .grammar
            .is_some_and(|grammar| ENABLED.contains(&Registry::builtin().grammar_name(grammar)))
    }

    pub(crate) fn retain(&mut self, open: &[FileId]) {
        self.docs.retain(|id, _| open.contains(id));
        self.stats.trees = self.docs.len();
    }

    /// Spans for each window, or `None` when the work was cancelled or `windows`
    /// do not fit the text.
    pub(crate) fn prepare(
        &mut self,
        id: FileId,
        source: Source,
        language: ResolvedLanguage,
        windows: &[Window],
        cancel: &AtomicBool,
    ) -> Option<Vec<WindowTokens>> {
        let grammar = language.grammar?;
        self.clock += 1;
        if windows
            .iter()
            .any(|window| window.range.end > source.text.len())
        {
            return None;
        }
        if source.text.len() > MAX_DOCUMENT_BYTES {
            self.docs.remove(&id);
            self.stats.plain_fallbacks += 1;
            return Some(windows.iter().map(plain).collect());
        }
        let Some(query) = self.query(grammar) else {
            self.stats.plain_fallbacks += 1;
            return Some(windows.iter().map(plain).collect());
        };
        let current = self
            .docs
            .get(&id)
            .is_some_and(|doc| doc.grammar == grammar && doc.source.same(&source));
        if !current && !self.reparse(id, &source, grammar, cancel) {
            return None;
        }
        let doc = self.docs.get_mut(&id).expect("parsed or current");
        doc.used = self.clock;
        let started = Instant::now();
        let mut result = Vec::with_capacity(windows.len());
        for window in windows {
            let spans = highlight(
                &doc.tree,
                &doc.source.text,
                &query,
                window.range.clone(),
                cancel,
            )?;
            self.stats.query_runs += 1;
            result.push(WindowTokens {
                range: window.range.clone(),
                spans: spans.into(),
                complete: true,
            });
        }
        self.stats.query_time += started.elapsed();
        self.evict(id);
        Some(result)
    }

    fn query(&mut self, grammar: GrammarId) -> Option<Arc<CompiledQuery>> {
        let name = Registry::builtin().grammar_name(grammar);
        let stats = &mut self.stats;
        self.queries
            .entry(grammar)
            .or_insert_with(|| {
                let started = Instant::now();
                let compiled = queries::compile(name);
                stats.queries_compiled += 1;
                stats.query_compile_time += started.elapsed();
                compiled.ok().map(Arc::new)
            })
            .clone()
    }

    /// Brings the stored tree up to date with `source`. False when cancelled.
    fn reparse(
        &mut self,
        id: FileId,
        source: &Source,
        grammar: GrammarId,
        cancel: &AtomicBool,
    ) -> bool {
        let name = Registry::builtin().grammar_name(grammar);
        let Some(language) = grammars::language(name) else {
            return false;
        };
        let parser = self.parsers.entry(grammar).or_insert_with(|| {
            let mut parser = Parser::new();
            parser
                .set_language(&language)
                .expect("a built-in grammar matches the linked Tree-sitter version");
            parser
        });
        let started = Instant::now();
        let previous = self.docs.get(&id).filter(|doc| doc.grammar == grammar);
        let edited = previous.and_then(|doc| {
            let revisions = source.changes_since(&doc.source)?;
            let mut tree = doc.tree.clone();
            for revision in &revisions {
                // One revision's edits all describe the text before it, so they
                // are applied from the last to the first.
                for edit in revision.edits.iter().rev() {
                    tree.edit(&input_edit(edit));
                }
            }
            Some(tree)
        });
        let incremental = edited.is_some();
        let Some(tree) = parse(parser, &source.text, edited.as_ref(), cancel) else {
            return false;
        };
        self.stats.parse_time += started.elapsed();
        self.stats.parsed_bytes += source.text.len();
        if incremental {
            self.stats.incremental_parses += 1;
        } else {
            self.stats.full_parses += 1;
        }
        self.docs.insert(
            id,
            DocTree {
                tree,
                source: source.clone(),
                grammar,
                used: self.clock,
            },
        );
        true
    }

    fn evict(&mut self, keep: FileId) {
        while self.docs.len() > MAX_TREES {
            let Some(oldest) = self
                .docs
                .iter()
                .filter(|(id, _)| **id != keep)
                .min_by_key(|(_, doc)| doc.used)
                .map(|(id, _)| *id)
            else {
                break;
            };
            self.docs.remove(&oldest);
        }
        self.stats.trees = self.docs.len();
    }
}

fn point(point: BytePoint) -> Point {
    Point {
        row: point.row,
        column: point.column,
    }
}

/// The Tree-sitter edit for one journal entry. `new_end_byte` comes from the
/// inserted extent, not from the entry's final-text range, because the edits of
/// one revision are applied one after another.
pub(crate) fn input_edit(edit: &RevisionEdit) -> InputEdit {
    InputEdit {
        start_byte: edit.old.start,
        old_end_byte: edit.old.end,
        new_end_byte: edit.old.start + edit.inserted_extent.bytes,
        start_position: point(edit.old_start_point),
        old_end_position: point(edit.old_end_point),
        new_end_position: point(edit.new_end_point()),
    }
}

/// Parses `text`, giving up when `cancel` is set. A cancelled parse is reset so
/// the parser can be used for another document.
pub(crate) fn parse(
    parser: &mut Parser,
    text: &str,
    old: Option<&Tree>,
    cancel: &AtomicBool,
) -> Option<Tree> {
    let bytes = text.as_bytes();
    let mut progress = |_: &ParseState| {
        if cancel.load(Ordering::Relaxed) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let tree = parser.parse_with_options(
        &mut |offset, _| {
            if offset < bytes.len() {
                &bytes[offset..]
            } else {
                &[][..]
            }
        },
        old,
        Some(ParseOptions::new().progress_callback(&mut progress)),
    );
    if tree.is_none() {
        parser.reset();
    }
    tree
}

fn plain(window: &Window) -> WindowTokens {
    let spans = if window.range.is_empty() {
        Vec::new()
    } else {
        vec![TokenSpan {
            range: window.range.clone(),
            class: None,
        }]
    };
    WindowTokens {
        range: window.range.clone(),
        spans: spans.into(),
        complete: true,
    }
}

pub(crate) struct Capture {
    pub(crate) range: Range<usize>,
    pub(crate) class: HighlightId,
    /// (pattern, position within the query's results): later wins a tie.
    pub(crate) rank: (usize, usize),
}

/// The spans for `window`, tiling it completely: text no pattern colours is a
/// span with no class. `None` when cancelled.
pub(crate) fn highlight(
    tree: &Tree,
    text: &str,
    query: &CompiledQuery,
    window: Range<usize>,
    cancel: &AtomicBool,
) -> Option<Vec<TokenSpan>> {
    if window.is_empty() {
        return Some(Vec::new());
    }
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(window.clone());
    let mut matches = cursor.matches(&query.query, tree.root_node(), text.as_bytes());
    let mut captures = Vec::new();
    let mut seen = 0usize;
    while let Some(found) = matches.next() {
        seen += 1;
        if seen.is_multiple_of(1024) && cancel.load(Ordering::Relaxed) {
            return None;
        }
        for capture in found.captures() {
            let Some(class) = query.class_of(capture.index) else {
                continue;
            };
            let node = capture.node.byte_range();
            let range = node.start.max(window.start)..node.end.min(window.end);
            if range.start < range.end {
                captures.push(Capture {
                    range,
                    class,
                    rank: (found.pattern_index, captures.len()),
                });
            }
        }
        if captures.len() >= MAX_CAPTURES {
            break;
        }
    }
    Some(tile(window, captures))
}

/// Resolves overlapping captures into consecutive spans. At each point the
/// shortest capture wins, and among equal ranges the later pattern.
pub(crate) fn tile(window: Range<usize>, mut captures: Vec<Capture>) -> Vec<TokenSpan> {
    let mut cuts = vec![window.start, window.end];
    for capture in &captures {
        cuts.push(capture.range.start);
        cuts.push(capture.range.end);
    }
    cuts.sort_unstable();
    cuts.dedup();
    captures.sort_by_key(|capture| capture.range.start);

    let mut spans: Vec<TokenSpan> = Vec::new();
    let mut active: Vec<usize> = Vec::new();
    let mut next = 0;
    for pair in cuts.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        while next < captures.len() && captures[next].range.start <= from {
            active.push(next);
            next += 1;
        }
        active.retain(|&index| captures[index].range.end > from);
        let class = active
            .iter()
            .copied()
            .min_by_key(|&index| (captures[index].range.len(), Reverse(captures[index].rank)))
            .map(|index| captures[index].class);
        match spans.last_mut() {
            Some(last) if last.class == class => last.range.end = to,
            _ => spans.push(TokenSpan {
                range: from..to,
                class,
            }),
        }
    }
    spans
}
