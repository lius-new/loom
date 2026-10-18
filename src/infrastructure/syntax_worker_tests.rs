use super::*;
use crate::domain::language::builtin;
use crate::features::highlighting::HighlightId;
use crate::features::syntax::baseline::generate;
use loom_editor_core::text::{EditKind, EditorMut, Selection, TextBuffer};
use std::{ops::Range, sync::mpsc, time::Duration};

fn stamp(id: u64, buffer: &TextBuffer) -> DocumentStamp {
    DocumentStamp::new(FileId::new(id), 1, 0, buffer)
}

/// `spans` cut to `range`, with equal neighbours joined: the form in which
/// results from different pages can be compared.
fn clipped(spans: &[TokenSpan], range: &Range<usize>) -> Vec<TokenSpan> {
    let mut out: Vec<TokenSpan> = Vec::new();
    for span in spans {
        let start = span.range.start.max(range.start);
        let end = span.range.end.min(range.end);
        if start >= end {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.class == span.class && last.range.end == start => {
                last.range.end = end;
            }
            _ => out.push(TokenSpan {
                range: start..end,
                class: span.class,
            }),
        }
    }
    out
}

/// What a fresh parse of the whole text says about `window`.
fn reference(buffer: &TextBuffer, language: ResolvedLanguage, window: &Window) -> Vec<TokenSpan> {
    let spans = TreeIndex::default()
        .prepare(
            FileId::new(9),
            Source::new(buffer),
            language,
            std::slice::from_ref(window),
            &AtomicBool::new(false),
        )
        .unwrap()
        .remove(0)
        .spans;
    clipped(&spans, &window.range)
}

fn class_at(spans: &[TokenSpan], offset: usize) -> Option<HighlightId> {
    spans
        .iter()
        .find(|span| span.range.contains(&offset))
        .and_then(|span| span.class)
}

fn notify(
    send: mpsc::Sender<(FileId, usize, u64)>,
) -> impl Fn(DocumentStamp) + Send + Sync + 'static {
    move |stamp| {
        send.send((stamp.document, stamp.snapshot_identity, stamp.version))
            .ok();
    }
}

#[test]
fn moving_viewports_preserve_known_spans_while_adjacent_pages_are_pending() {
    use std::sync::atomic::AtomicUsize;
    for language in [
        builtin("plaintext"),
        builtin("typescript"),
        builtin("csharp"),
        builtin("rust"),
    ] {
        let buffer = TextBuffer::new("Alpha123 中e\u{301}=\"value\"; 456 /*note*/ ".repeat(200));
        let (send, receive) = mpsc::channel();
        let (enter, entered) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let jobs = AtomicUsize::new(0);
        let worker = Worker::start(notify(send), move || {
            if jobs.fetch_add(1, Ordering::Relaxed) == 1 {
                enter.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        });
        let window = |start, end| Window {
            line: buffer.line_range(0),
            range: buffer.grapheme_floor(start)..buffer.grapheme_floor(end),
        };
        let query = |window: &Window| {
            worker.query(
                PaneId::new(1),
                stamp(1, &buffer),
                Source::new(&buffer),
                language,
                vec![window.clone()],
            )
        };
        let first = window(PAGE_BYTES - 160, PAGE_BYTES - 80);
        assert!(query(&first).is_empty());
        wait(&receive);
        let before = worker.stats();
        for shift in 0..70 {
            let current = window(PAGE_BYTES - 160 + shift, PAGE_BYTES - 80 + shift);
            let actual = query(&current);
            assert!(actual[0].complete);
            assert_eq!(
                clipped(&actual[0].spans, &current.range),
                reference(&buffer, language, &current)
            );
        }
        assert_eq!(
            worker.stats().tree.parsed_bytes,
            before.tree.parsed_bytes,
            "sliding inside a known page parses nothing"
        );

        let crossed = window(PAGE_BYTES - 40, PAGE_BYTES + 100);
        let pending = query(&crossed);
        assert!(!pending[0].complete);
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        for shift in 0..20 {
            let current = window(PAGE_BYTES - 40 + shift, PAGE_BYTES + 100 + shift);
            let actual = query(&current);
            assert!(!actual[0].complete);
            for known in clipped(&pending[0].spans, &current.range) {
                assert_eq!(
                    class_at(&actual[0].spans, known.range.start),
                    known.class,
                    "moving the viewport must not erase a known span"
                );
            }
        }
        release.send(()).unwrap();
        wait(&receive);
        let completed = query(&crossed);
        assert!(completed[0].complete);
        assert_eq!(
            clipped(&completed[0].spans, &crossed.range),
            reference(&buffer, language, &crossed)
        );
    }
}

#[test]
fn a_token_longer_than_a_page_stays_known_when_the_page_that_found_it_leaves_the_viewport() {
    use std::sync::atomic::AtomicUsize;
    let buffer = TextBuffer::new(format!(
        "const S: &str = \"{}\";",
        "中e\u{301}abc".repeat(PAGE_BYTES)
    ));
    let (send, receive) = mpsc::channel();
    let (enter, entered) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let jobs = AtomicUsize::new(0);
    let worker = Worker::start(notify(send), move || {
        if jobs.fetch_add(1, Ordering::Relaxed) == 1 {
            enter.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(5)).unwrap();
        }
    });
    let query = |start, end| {
        worker.query(
            PaneId::new(1),
            stamp(1, &buffer),
            Source::new(&buffer),
            builtin("rust"),
            vec![Window {
                line: buffer.line_range(0),
                range: buffer.grapheme_floor(start)..buffer.grapheme_floor(end),
            }],
        )
    };
    query(20, 100);
    wait(&receive);
    let known = query(20, 100).remove(0);
    assert_eq!(class_at(&known.spans, 50), Some(HighlightId::STRING));
    // Two pages further on nothing has been computed, but the string that began
    // on the first page is known to run through them.
    let moved = query(PAGE_BYTES + 40, PAGE_BYTES + 120).remove(0);
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(moved.complete);
    assert_eq!(
        clipped(&moved.spans, &(PAGE_BYTES + 40..PAGE_BYTES + 120)),
        [TokenSpan {
            range: PAGE_BYTES + 40..PAGE_BYTES + 120,
            class: Some(HighlightId::STRING)
        }]
    );
    let further = query(2 * PAGE_BYTES + 40, 2 * PAGE_BYTES + 120).remove(0);
    assert!(further.complete);
    assert_eq!(
        class_at(&further.spans, 2 * PAGE_BYTES + 80),
        Some(HighlightId::STRING)
    );
    release.send(()).unwrap();
    wait(&receive);
}

fn query(worker: &Worker, pane: u64, id: u64, buffer: &TextBuffer) -> Vec<WindowTokens> {
    worker.query(
        PaneId::new(pane),
        stamp(id, buffer),
        Source::new(buffer),
        builtin("typescript"),
        vec![Window {
            line: buffer.line_range(0),
            range: buffer.line_range(0),
        }],
    )
}
fn wait(receive: &mpsc::Receiver<(FileId, usize, u64)>) -> (FileId, usize, u64) {
    receive
        .recv_timeout(Duration::from_secs(5))
        .expect("background completion")
}
type GatedWorker = (
    Worker,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
    mpsc::Receiver<(FileId, usize, u64)>,
);

fn gated() -> GatedWorker {
    let (entered, entering) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let (send, receive) = mpsc::channel();
    let first = AtomicBool::new(true);
    let worker = Worker::start(notify(send), move || {
        if first.swap(false, Ordering::Relaxed) {
            entered.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(5))
                .expect("release worker gate");
        }
    });
    (worker, entering, release, receive)
}

#[test]
fn ui_queries_never_wait_for_parsing_and_duplicate_frames_are_coalesced() {
    let (worker, entered, release, receive) = gated();
    let buffer = TextBuffer::new("const value=123;".repeat(100));
    assert!(query(&worker, 1, 1, &buffer).is_empty());
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    for _ in 0..30 {
        assert!(query(&worker, 1, 1, &buffer).is_empty());
    }
    assert!(worker.0.0.state.lock().unwrap().pending.is_empty());
    release.send(()).unwrap();
    wait(&receive);
    assert_eq!(query(&worker, 1, 1, &buffer).len(), 1);
    assert!(receive.try_recv().is_err());
}

#[test]
fn superseded_versions_are_cancelled_and_only_the_latest_is_published() {
    let (worker, entered, release, receive) = gated();
    let mut buffer = TextBuffer::new("const value=123;".repeat(100));
    query(&worker, 1, 1, &buffer);
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    EditorMut::new(&mut buffer, &mut Selection::default(), vec![])
        .replace(12..15, "4567", EditKind::Other)
        .unwrap();
    query(&worker, 1, 1, &buffer);
    release.send(()).unwrap();
    let (_, identity, version) = wait(&receive);
    assert_eq!(
        (identity, version),
        (buffer.snapshot_identity(), buffer.version())
    );
    let actual = query(&worker, 1, 1, &buffer);
    let window = Window {
        line: buffer.line_range(0),
        range: buffer.line_range(0),
    };
    assert_eq!(
        clipped(&actual[0].spans, &window.range),
        reference(&buffer, builtin("typescript"), &window)
    );
    assert_eq!(actual[0].freshness, Freshness::Fresh);
    assert!(receive.try_recv().is_err());
}

#[test]
fn switching_files_and_split_panes_reuse_completed_windows_and_trees() {
    let (send, receive) = mpsc::channel();
    let worker = Worker::new(notify(send));
    let a = TextBuffer::new("const a=1;".repeat(100));
    let b = TextBuffer::new("const b=2;".repeat(100));
    query(&worker, 1, 1, &a);
    wait(&receive);
    let first = query(&worker, 1, 1, &a).remove(0);
    query(&worker, 1, 2, &b);
    wait(&receive);
    let before = worker.stats();
    let returned = query(&worker, 1, 1, &a).remove(0);
    let split = query(&worker, 2, 1, &a).remove(0);
    assert!(Arc::ptr_eq(&first.spans, &returned.spans));
    assert!(Arc::ptr_eq(&first.spans, &split.spans));
    assert_eq!(worker.stats().tree.parsed_bytes, before.tree.parsed_bytes);
    assert!(worker.stats().cache_hits >= before.cache_hits + 2);
    worker.retain(vec![]);
    assert_eq!(worker.stats().cache_bytes, 0);
    assert!(worker.0.0.state.lock().unwrap().epochs.is_empty());
}

#[test]
fn closed_files_and_shutdown_discard_in_flight_results() {
    for shutdown in [false, true] {
        let (worker, entered, release, receive) = gated();
        let buffer = TextBuffer::new("const value=123;".repeat(100));
        query(&worker, 1, 1, &buffer);
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        if shutdown {
            worker.shutdown();
        } else {
            worker.retain(vec![]);
        }
        release.send(()).unwrap();
        // A different document acts as an ordered barrier after cancellation.
        if !shutdown {
            let b = TextBuffer::new("const b=2;");
            query(&worker, 1, 2, &b);
            assert_eq!(wait(&receive).0, FileId::new(2));
            assert!(
                worker
                    .0
                    .0
                    .state
                    .lock()
                    .unwrap()
                    .ready
                    .keys()
                    .all(|k| k.document == FileId::new(2))
            );
        }
        assert!(receive.try_recv().is_err());
    }
}

#[test]
fn a_new_incarnation_or_language_generation_never_reuses_cached_windows() {
    let (send, receive) = mpsc::channel();
    let worker = Worker::new(notify(send));
    let buffer = TextBuffer::new("const a=1;".repeat(100));
    let window = || {
        vec![Window {
            line: buffer.line_range(0),
            range: buffer.line_range(0),
        }]
    };
    let ask = |stamp: DocumentStamp| {
        worker.query(
            PaneId::new(1),
            stamp,
            Source::new(&buffer),
            builtin("typescript"),
            window(),
        )
    };
    let original = stamp(1, &buffer);
    assert!(ask(original).is_empty());
    wait(&receive);
    assert_eq!(ask(original).len(), 1, "the original stamp is served");

    // Same document id, same text, same snapshot: only the lifetime differs.
    let reopened = DocumentStamp {
        incarnation: original.incarnation + 1,
        ..original
    };
    assert!(
        ask(reopened).is_empty(),
        "a reopened document must not see the old windows or the old colours"
    );
    wait(&receive);
    assert_eq!(ask(reopened).len(), 1);

    // Same text again, but the language was switched.
    let relabelled = DocumentStamp {
        language_generation: original.language_generation + 1,
        ..reopened
    };
    assert!(ask(relabelled).is_empty());
    wait(&receive);
    assert_eq!(ask(relabelled).len(), 1);
    assert!(
        ask(original).is_empty(),
        "going back to a stale stamp serves nothing either"
    );
}

#[test]
fn text_with_a_grammar_is_read_from_a_tree_and_text_without_one_is_plain() {
    let (send, receive) = mpsc::channel();
    let worker = Worker::new(notify(send));
    let class_in = |tokens: &WindowTokens, text: &str, needle: &str| {
        class_at(&tokens.spans, text.find(needle).unwrap())
    };
    let ask = |id: u64, buffer: &TextBuffer, language: &str| {
        let window = Window {
            line: buffer.line_range(0),
            range: buffer.line_range(0),
        };
        worker.query(
            PaneId::new(1),
            stamp(id, buffer),
            Source::new(buffer),
            builtin(language),
            vec![window],
        )
    };

    let rust = TextBuffer::new("fn main() { let x = 1; } // note");
    assert!(
        ask(1, &rust, "rust").is_empty(),
        "nothing is ready on the first frame"
    );
    wait(&receive);
    let tokens = ask(1, &rust, "rust").remove(0);
    assert!(tokens.complete);
    assert_eq!(
        class_in(&tokens, rust.text(), "main"),
        Some(HighlightId::FUNCTION)
    );
    assert_eq!(
        class_in(&tokens, rust.text(), "let"),
        Some(HighlightId::KEYWORD)
    );
    assert_eq!(
        class_in(&tokens, rust.text(), "note"),
        Some(HighlightId::COMMENT)
    );

    let csharp = TextBuffer::new("public class A { } // note");
    ask(2, &csharp, "csharp");
    wait(&receive);
    let tokens = ask(2, &csharp, "csharp").remove(0);
    assert_eq!(
        class_in(&tokens, csharp.text(), "public"),
        Some(HighlightId::KEYWORD)
    );
    assert_eq!(worker.stats().tree.full_parses, 2);

    let plain = TextBuffer::new("fn main() { let x = 1; } // not code, 42 \"quoted\"");
    ask(3, &plain, "plaintext");
    wait(&receive);
    let tokens = ask(3, &plain, "plaintext").remove(0);
    assert_eq!(
        tokens.spans.as_ref(),
        [TokenSpan {
            range: plain.line_range(0),
            class: None
        }],
        "plain text has no colours at all"
    );
    assert_eq!(
        worker.stats().tree.full_parses,
        2,
        "plain text is never parsed"
    );

    worker.retain(vec![]);
    // Pruning runs on the worker thread, so give it a moment.
    let limit = std::time::Instant::now() + Duration::from_secs(5);
    while worker.stats().tree.trees != 0 {
        assert!(
            std::time::Instant::now() < limit,
            "the tree was not dropped"
        );
        std::thread::yield_now();
    }
}

fn lines(buffer: &TextBuffer, range: Range<usize>) -> Vec<Window> {
    range
        .map(|line| Window {
            line: buffer.line_range(line),
            range: buffer.line_range(line),
        })
        .collect()
}

fn ask_lines(
    worker: &Worker,
    id: u64,
    buffer: &TextBuffer,
    language: ResolvedLanguage,
    windows: &[Window],
) -> Vec<WindowTokens> {
    worker.query(
        PaneId::new(1),
        stamp(id, buffer),
        Source::new(buffer),
        language,
        windows.to_vec(),
    )
}

#[test]
fn typing_keeps_the_previous_colours_on_screen_while_each_new_version_is_parsed() {
    let (send, receive) = mpsc::channel();
    let slow = Arc::new(AtomicBool::new(false));
    let delay = slow.clone();
    // Every parse takes 150 ms once switched on, as in the design's injected delay.
    let worker = Worker::start(notify(send), move || {
        if delay.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(150));
        }
    });
    let language = builtin("rust");
    let mut buffer = TextBuffer::new(generate("rust", 40 * 1024));
    let windows = lines(&buffer, 60..100);
    assert!(ask_lines(&worker, 1, &buffer, language, &windows).is_empty());
    wait(&receive);
    let anchor = ask_lines(&worker, 1, &buffer, language, &windows);
    assert_eq!(anchor.len(), 40);
    assert!(
        anchor
            .iter()
            .all(|t| t.complete && t.freshness == Freshness::Fresh)
    );
    assert!(
        anchor
            .iter()
            .any(|t| t.spans.iter().any(|s| s.class.is_some()))
    );

    slow.store(true, Ordering::Relaxed);
    let edit_line = 80;
    let mut typed = 0;
    for _ in 0..10 {
        let at = buffer.line_range(edit_line).end;
        EditorMut::new(&mut buffer, &mut Selection::default(), vec![])
            .replace(at..at, "x", EditKind::Other)
            .unwrap();
        typed += 1;
        // The new version's windows, as the editor asks for them right now.
        let now = lines(&buffer, 60..100);
        let frame = ask_lines(&worker, 1, &buffer, language, &now);
        assert_eq!(frame.len(), 40, "every visible line has something to draw");
        assert!(
            frame.iter().all(|t| t.freshness == Freshness::Provisional),
            "the parser is still busy, so these are carried-over colours"
        );
        for (index, tokens) in frame.iter().enumerate() {
            let line = 60 + index;
            let shift = if line > edit_line { typed } else { 0 };
            let before = &anchor[index];
            // Text that did not change keeps exactly the colours it had, moved by
            // the characters typed above it.
            let expected: Vec<TokenSpan> = before
                .spans
                .iter()
                .map(|s| TokenSpan {
                    range: s.range.start + shift..s.range.end + shift,
                    class: s.class,
                })
                .collect();
            let end_grew = usize::from(line == edit_line) * typed;
            let range = now[index].range.start..now[index].range.end - end_grew;
            assert_eq!(
                clipped(&tokens.spans, &range),
                clipped(&expected, &range),
                "line {line} after {typed} typed characters"
            );
        }
    }
    let stats = worker.stats().provisional;
    assert_eq!(stats.windows, 10 * 40);
    assert_eq!(stats.refused + stats.broken_chain, 0);

    // Once the parser catches up the carried colours give way to parsed ones.
    slow.store(false, Ordering::Relaxed);
    let now = lines(&buffer, 60..100);
    let limit = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let frame = ask_lines(&worker, 1, &buffer, language, &now);
        if frame
            .iter()
            .all(|t| t.complete && t.freshness == Freshness::Fresh)
        {
            for (tokens, window) in frame.iter().zip(&now) {
                assert_eq!(
                    clipped(&tokens.spans, &window.range),
                    reference(&buffer, language, window)
                );
            }
            break;
        }
        assert!(
            std::time::Instant::now() < limit,
            "the parse never finished"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn colours_cannot_be_carried_across_a_reload_a_new_lifetime_or_a_language_change() {
    let (send, receive) = mpsc::channel();
    let worker = Worker::new(notify(send));
    let language = builtin("rust");
    let mut buffer = TextBuffer::new(generate("rust", 8 * 1024));
    let windows = lines(&buffer, 0..20);
    ask_lines(&worker, 1, &buffer, language, &windows);
    wait(&receive);

    // An edit: carried over.
    let at = buffer.line_range(5).end;
    EditorMut::new(&mut buffer, &mut Selection::default(), vec![])
        .replace(at..at, "x", EditKind::Other)
        .unwrap();
    let carried = ask_lines(&worker, 1, &buffer, language, &lines(&buffer, 0..20));
    assert_eq!(carried.len(), 20);
    assert!(
        carried
            .iter()
            .all(|t| t.freshness == Freshness::Provisional)
    );
    wait(&receive);

    // A reload leaves no journal to follow: plain until the parse is ready.
    buffer.reload(generate("rust", 8 * 1024));
    assert!(ask_lines(&worker, 1, &buffer, language, &lines(&buffer, 0..20)).is_empty());
    assert!(worker.stats().provisional.broken_chain >= 1);
    wait(&receive);

    // Another lifetime of the same document and another language start clean.
    let at = buffer.line_range(5).end;
    EditorMut::new(&mut buffer, &mut Selection::default(), vec![])
        .replace(at..at, "x", EditKind::Other)
        .unwrap();
    let reopened = DocumentStamp {
        incarnation: 2,
        ..stamp(1, &buffer)
    };
    let ask = |stamp: DocumentStamp, language: ResolvedLanguage| {
        worker.query(
            PaneId::new(1),
            stamp,
            Source::new(&buffer),
            language,
            lines(&buffer, 0..20),
        )
    };
    assert!(ask(reopened, language).is_empty(), "a reopened document");
    wait(&receive);
    let relabelled = DocumentStamp {
        language_generation: 1,
        ..reopened
    };
    assert!(
        ask(relabelled, builtin("csharp")).is_empty(),
        "another language"
    );
    wait(&receive);

    // Closing the document drops what was kept for it.
    worker.retain(vec![]);
    assert!(worker.0.0.state.lock().unwrap().retained.stats.windows >= 20);
}

#[test]
fn a_projection_beyond_its_limits_shows_plain_text_until_the_parse_is_ready() {
    use crate::features::syntax::presentation::MAX_REVISIONS;
    let (send, receive) = mpsc::channel();
    let worker = Worker::new(notify(send));
    let language = builtin("rust");
    let mut buffer = TextBuffer::new(generate("rust", 8 * 1024));
    ask_lines(&worker, 1, &buffer, language, &lines(&buffer, 0..20));
    wait(&receive);
    for _ in 0..=MAX_REVISIONS {
        let at = buffer.line_range(5).end;
        EditorMut::new(&mut buffer, &mut Selection::default(), vec![])
            .replace(at..at, "x", EditKind::Other)
            .unwrap();
    }
    let frame = ask_lines(&worker, 1, &buffer, language, &lines(&buffer, 0..20));
    assert!(frame.is_empty());
    assert_eq!(worker.stats().provisional.refused, 1);
}

#[test]
fn thirty_two_tabs_return_coloured_after_their_trees_were_dropped() {
    let (send, receive) = mpsc::channel();
    // Once switched on, no further parse may run: a tab that needs one hangs.
    let frozen = Arc::new(AtomicBool::new(false));
    let hold = frozen.clone();
    let worker = Worker::start(notify(send), move || {
        while hold.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    let language = builtin("rust");
    let buffers: Vec<TextBuffer> = (0..32)
        .map(|i| {
            TextBuffer::new(generate("rust", 16 * 1024).replace("compute_", &format!("tab{i}_")))
        })
        .collect();
    let windows: Vec<Vec<Window>> = buffers.iter().map(|b| lines(b, 0..50)).collect();
    for (index, buffer) in buffers.iter().enumerate() {
        ask_lines(&worker, index as u64 + 1, buffer, language, &windows[index]);
        wait(&receive);
    }
    assert_eq!(
        worker.stats().tree.trees,
        16,
        "only the recent trees remain"
    );
    let parses = worker.stats().tree.full_parses;

    frozen.store(true, Ordering::Relaxed);
    for (index, buffer) in buffers.iter().enumerate() {
        let frame = ask_lines(&worker, index as u64 + 1, buffer, language, &windows[index]);
        assert_eq!(
            frame.len(),
            50,
            "tab {index} has its colours at the first frame"
        );
        assert!(
            frame
                .iter()
                .all(|t| t.complete && t.freshness == Freshness::Fresh),
            "tab {index}"
        );
        assert!(frame.iter().any(|t| {
            t.spans
                .iter()
                .any(|s| s.class == Some(HighlightId::KEYWORD))
        }));
    }
    assert_eq!(
        worker.stats().tree.full_parses,
        parses,
        "no tree was rebuilt"
    );
    assert!(worker.0.0.state.lock().unwrap().pending.is_empty());
    frozen.store(false, Ordering::Relaxed);
    worker.shutdown();
}

#[test]
fn an_oversized_document_is_shown_as_plain_text_and_says_so() {
    use crate::features::syntax::TreeStats;
    let (send, receive) = mpsc::channel();
    let worker = Worker::new(notify(send));
    let buffer = TextBuffer::new(format!("fn a() {{}}\n{}", "x".repeat(8 * 1024 * 1024)));
    let windows = lines(&buffer, 0..1);
    ask_lines(&worker, 1, &buffer, builtin("rust"), &windows);
    wait(&receive);
    let frame = ask_lines(&worker, 1, &buffer, builtin("rust"), &windows);
    assert_eq!(frame[0].spans.len(), 1);
    assert_eq!(frame[0].spans[0].class, None, "the keyword is not coloured");
    let TreeStats {
        plain_fallbacks,
        full_parses,
        ..
    } = worker.stats().tree;
    assert_eq!((plain_fallbacks, full_parses), (1, 0));
}
