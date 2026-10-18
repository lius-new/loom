//! Coalesced viewport work. Lexing and index mutation never hold the UI lock.
use crate::{
    domain::{
        document::DocumentStamp,
        identity::{FileId, PaneId},
        language::ResolvedLanguage,
    },
    features::syntax::{
        Index, IndexStats, Source, TokenSpan, TreeIndex, TreeStats, Window, WindowTokens,
    },
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

const WINDOW_BUDGET: usize = 4 * 1024 * 1024;
// Cache pages are anchored to source lines, independently of caret, scroll,
// viewport width and language. Screen clipping never invalidates known tokens.
const PAGE_BYTES: usize = 1024;

fn pages(source: &Source, window: &Window) -> Vec<Window> {
    if window.range.is_empty() {
        return vec![window.clone()];
    }
    let first = (window.range.start - window.line.start) / PAGE_BYTES;
    let last = (window.range.end - 1 - window.line.start) / PAGE_BYTES;
    (first..=last)
        .map(|page| {
            let floor = |mut offset| {
                while !source.text.is_char_boundary(offset) {
                    offset -= 1;
                }
                offset
            };
            Window {
                line: window.line.clone(),
                range: floor(window.line.start + page * PAGE_BYTES)
                    ..floor((window.line.start + (page + 1) * PAGE_BYTES).min(window.line.end)),
            }
        })
        .filter(|page| page.range.start < window.range.end && page.range.end > window.range.start)
        .collect()
}
#[derive(Clone)]
struct Request {
    stamp: DocumentStamp,
    document: FileId,
    source: Source,
    lang: ResolvedLanguage,
    windows: Vec<Window>,
}

struct Epoch {
    identity: usize,
    version: u64,
    incarnation: u64,
    language_generation: u64,
    lang: ResolvedLanguage,
    cancel: Arc<AtomicBool>,
}
impl Epoch {
    fn matches(&self, request: &Request) -> bool {
        self.identity == request.source.identity()
            && self.version == request.source.version
            && self.incarnation == request.stamp.incarnation
            && self.language_generation == request.stamp.language_generation
            && self.lang == request.lang
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    document: FileId,
    incarnation: u64,
    language_generation: u64,
    identity: usize,
    version: u64,
    lang: ResolvedLanguage,
    start: usize,
    end: usize,
}
impl Key {
    fn new(request: &Request, window: &Window) -> Self {
        Self {
            document: request.document,
            incarnation: request.stamp.incarnation,
            language_generation: request.stamp.language_generation,
            identity: request.source.identity(),
            version: request.source.version,
            lang: request.lang,
            start: window.range.start,
            end: window.range.end,
        }
    }
}
struct Cached {
    window: WindowTokens,
    used: u64,
    bytes: usize,
}
impl Cached {
    fn intersecting(&self, range: &std::ops::Range<usize>) -> &[TokenSpan] {
        let first = self
            .window
            .spans
            .partition_point(|span| span.range.end <= range.start);
        let last = self
            .window
            .spans
            .partition_point(|span| span.range.start < range.end);
        &self.window.spans[first..last]
    }
}
#[derive(Default)]
struct State {
    targets: HashMap<PaneId, Request>,
    pending: VecDeque<FileId>,
    running: Option<(Request, Vec<Window>, Arc<AtomicBool>)>,
    epochs: HashMap<FileId, Epoch>,
    ready: HashMap<Key, Cached>,
    bytes: usize,
    clock: u64,
    hits: usize,
    evictions: usize,
    prune: Option<Vec<FileId>>,
    index_stats: IndexStats,
    tree_stats: TreeStats,
}
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    stopped: AtomicBool,
}
struct Owner(Arc<Shared>);
impl Drop for Owner {
    fn drop(&mut self) {
        stop(&self.0);
    }
}
fn stop(shared: &Shared) {
    shared.stopped.store(true, Ordering::Release);
    let state = shared.state.lock().unwrap();
    for epoch in state.epochs.values() {
        epoch.cancel.store(true, Ordering::Release);
    }
    shared.wake.notify_all();
}

#[derive(Clone)]
pub(crate) struct Worker(Arc<Owner>);
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Stats {
    pub index: IndexStats,
    pub tree: TreeStats,
    pub cache_bytes: usize,
    pub cache_hits: usize,
    pub evictions: usize,
}
impl Worker {
    pub fn new(ready: impl Fn(DocumentStamp) + Send + Sync + 'static) -> Self {
        Self::start(ready, || {})
    }
    fn start(
        ready: impl Fn(DocumentStamp) + Send + Sync + 'static,
        before_prepare: impl Fn() + Send + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            stopped: AtomicBool::new(false),
        });
        let background = shared.clone();
        thread::Builder::new()
            .name("loom-syntax".into())
            .spawn(move || {
                let mut index = Index::default();
                let mut trees = TreeIndex::default();
                loop {
                    let job = {
                        let mut state = background.state.lock().unwrap();
                        while state.pending.is_empty()
                            && state.prune.is_none()
                            && !background.stopped.load(Ordering::Acquire)
                        {
                            state = background.wake.wait(state).unwrap();
                        }
                        if background.stopped.load(Ordering::Acquire) {
                            break;
                        }
                        if let Some(open) = state.prune.take() {
                            drop(state);
                            index.retain(&open);
                            trees.retain(&open);
                            let mut state = background.state.lock().unwrap();
                            state.index_stats = index.stats;
                            state.tree_stats = trees.stats;
                            continue;
                        }
                        let document = state.pending.pop_front().unwrap();
                        let Some(epoch) = state.epochs.get(&document) else {
                            continue;
                        };
                        let Some(request) = state
                            .targets
                            .values()
                            .find(|r| r.document == document && epoch.matches(r))
                            .cloned()
                        else {
                            continue;
                        };
                        let mut windows = Vec::new();
                        for target in state
                            .targets
                            .values()
                            .filter(|r| r.document == document && epoch.matches(r))
                        {
                            for window in &target.windows {
                                if !windows.contains(window)
                                    && !state.ready.contains_key(&Key::new(target, window))
                                {
                                    windows.push(window.clone());
                                }
                            }
                        }
                        if windows.is_empty() {
                            continue;
                        }
                        let cancel = epoch.cancel.clone();
                        state.running = Some((request.clone(), windows.clone(), cancel.clone()));
                        (request, windows, cancel)
                    };
                    let (request, windows, cancel) = job;
                    before_prepare();
                    // Languages with an enabled grammar are read from a syntax
                    // tree; the rest still go through the line lexer.
                    let result = if TreeIndex::handles(request.lang) {
                        trees.prepare(
                            request.document,
                            request.source.clone(),
                            request.lang,
                            &windows,
                            &cancel,
                        )
                    } else {
                        index.prepare(
                            request.document,
                            request.source.clone(),
                            request.lang,
                            &windows,
                            &cancel,
                        )
                    };
                    let mut state = background.state.lock().unwrap();
                    state.running = None;
                    state.index_stats = index.stats;
                    state.tree_stats = trees.stats;
                    if background.stopped.load(Ordering::Acquire) {
                        break;
                    }
                    if cancel.load(Ordering::Acquire)
                        || !state.epochs.get(&request.document).is_some_and(|epoch| {
                            epoch.matches(&request) && Arc::ptr_eq(&epoch.cancel, &cancel)
                        })
                    {
                        continue;
                    }
                    let Some(result) = result else {
                        continue;
                    };
                    for (window, tokens) in windows.iter().zip(result) {
                        let key = Key::new(&request, window);
                        let bytes = std::mem::size_of::<Cached>()
                            + tokens.spans.len()
                                * std::mem::size_of::<crate::features::syntax::TokenSpan>();
                        state.clock += 1;
                        let used = state.clock;
                        if let Some(old) = state.ready.insert(
                            key,
                            Cached {
                                window: tokens,
                                used,
                                bytes,
                            },
                        ) {
                            state.bytes -= old.bytes;
                        }
                        state.bytes += bytes;
                    }
                    while state.bytes > WINDOW_BUDGET || state.ready.len() > 2048 {
                        let key = *state
                            .ready
                            .iter()
                            .min_by_key(|(_, entry)| entry.used)
                            .unwrap()
                            .0;
                        state.bytes -= state.ready.remove(&key).unwrap().bytes;
                        state.evictions += 1;
                    }
                    drop(state);
                    ready(request.stamp);
                }
            })
            .expect("could not start syntax worker");
        Self(Arc::new(Owner(shared)))
    }
    pub fn shutdown(&self) {
        stop(&self.0.0);
    }
    pub fn query(
        &self,
        pane: PaneId,
        stamp: DocumentStamp,
        source: Source,
        lang: ResolvedLanguage,
        windows: Vec<Window>,
    ) -> Vec<WindowTokens> {
        let shared = &self.0.0;
        if shared.stopped.load(Ordering::Acquire) {
            return Vec::new();
        }
        // Only request bookkeeping and immutable viewport results share this
        // lock. The scanner owns its index outside it, even during cancellation.
        let mut state = shared.state.lock().unwrap();
        debug_assert_eq!(
            (stamp.version, stamp.snapshot_identity),
            (source.version, source.identity()),
            "the stamp and the source must describe the same text"
        );
        let document = stamp.document;
        let request = Request {
            stamp,
            document,
            source,
            lang,
            windows: Vec::new(),
        };
        let changed_data = !state
            .epochs
            .get(&document)
            .is_some_and(|epoch| epoch.matches(&request));
        let changed = changed_data
            || state
                .epochs
                .get(&document)
                .is_some_and(|epoch| epoch.cancel.load(Ordering::Relaxed));
        if changed {
            if let Some(old) = state.epochs.insert(
                document,
                Epoch {
                    identity: request.source.identity(),
                    version: request.source.version,
                    incarnation: request.stamp.incarnation,
                    language_generation: request.stamp.language_generation,
                    lang,
                    cancel: Arc::new(AtomicBool::new(false)),
                },
            ) {
                old.cancel.store(true, Ordering::Release);
            }
            if changed_data {
                state.ready.retain(|key, _| key.document != document);
                state.bytes = state.ready.values().map(|entry| entry.bytes).sum();
            }
        }
        let mut result = Vec::new();
        let mut missing = false;
        let mut request = request;
        for window in windows {
            if window.range.is_empty() {
                result.push(WindowTokens {
                    range: window.range,
                    spans: Arc::from([]),
                    complete: true,
                });
                continue;
            }
            let blocks = pages(&request.source, &window);
            let mut complete = true;
            let mut found = false;
            let mut exact = None;
            let mut spans = Vec::new();
            for block in &blocks {
                if !request.windows.contains(block) {
                    request.windows.push(block.clone());
                }
                state.clock += 1;
                let used = state.clock;
                if let Some(entry) = state.ready.get_mut(&Key::new(&request, block)) {
                    entry.used = used;
                    found = true;
                    if block.range == window.range {
                        exact = Some(entry.window.clone());
                    } else {
                        spans.extend_from_slice(entry.intersecting(&window.range));
                    }
                    state.hits += 1;
                } else {
                    complete = false;
                }
            }
            missing |= !complete;
            if !complete {
                // A token may span several pages (strings, comments, names).
                // Its full semantic range stays valid beyond the page that
                // first published it, even while the next page is pending.
                state.clock += 1;
                let used = state.clock;
                for (key, entry) in &mut state.ready {
                    if key.document != document
                        || key.incarnation != request.stamp.incarnation
                        || key.language_generation != request.stamp.language_generation
                        || key.identity != request.source.identity()
                        || key.version != request.source.version
                        || key.lang != lang
                    {
                        continue;
                    }
                    let available = entry.intersecting(&window.range);
                    if !available.is_empty() {
                        found = true;
                        spans.extend_from_slice(available);
                        entry.used = used;
                    }
                }
            }
            if let Some(exact) = exact {
                result.push(exact);
            } else if found {
                spans.sort_unstable_by_key(|span| (span.range.start, span.range.end));
                spans.dedup();
                let mut covered = window.range.start;
                for span in &spans {
                    if span.range.start > covered {
                        break;
                    }
                    covered = covered.max(span.range.end);
                }
                complete = covered >= window.range.end;
                result.push(WindowTokens {
                    range: window.range,
                    spans: spans.into(),
                    complete,
                });
            }
        }
        let in_flight = state
            .running
            .as_ref()
            .is_some_and(|(running, windows, cancel)| {
                !cancel.load(Ordering::Acquire)
                    && running.document == document
                    && running.source.same(&request.source)
                    && running.lang == lang
                    && request.windows.iter().all(|window| {
                        state.ready.contains_key(&Key::new(&request, window))
                            || windows.contains(window)
                    })
            });
        let previous = state.targets.insert(pane, request);
        if let Some(previous) = previous
            && previous.document != document
            && !state
                .targets
                .values()
                .any(|r| r.document == previous.document)
        {
            if let Some(epoch) = state.epochs.get(&previous.document) {
                epoch.cancel.store(true, Ordering::Release);
            }
        }
        if missing && !in_flight && !state.pending.contains(&document) {
            state.pending.push_front(document);
            shared.wake.notify_one();
        }
        result
    }
    pub fn retain(&self, open: Vec<FileId>) {
        let mut state = self.0.0.state.lock().unwrap();
        state.targets.retain(|_, r| open.contains(&r.document));
        state.epochs.retain(|id, epoch| {
            if open.contains(id) {
                true
            } else {
                epoch.cancel.store(true, Ordering::Release);
                false
            }
        });
        state.ready.retain(|key, _| open.contains(&key.document));
        state.bytes = state.ready.values().map(|entry| entry.bytes).sum();
        state.prune = Some(open);
        self.0.0.wake.notify_one();
    }
    pub fn stats(&self) -> Stats {
        let state = self.0.0.state.lock().unwrap();
        Stats {
            index: state.index_stats,
            tree: state.tree_stats,
            cache_bytes: state.bytes,
            cache_hits: state.hits,
            evictions: state.evictions,
        }
    }
}

#[cfg(test)]
#[path = "syntax_worker_bench.rs"]
mod bench;
#[cfg(test)]
#[path = "syntax_worker_tests.rs"]
mod tests;
