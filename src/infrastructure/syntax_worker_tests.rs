use super::*;

fn stamp(id: u64, buffer: &TextBuffer) -> DocumentStamp {
    DocumentStamp::new(FileId::new(id), 1, 0, buffer)
}
use loom_editor_core::text::{EditKind, EditorMut, Selection, TextBuffer};
use std::{sync::mpsc, time::Duration};

#[test]
fn moving_viewports_preserve_known_tokens_while_adjacent_pages_are_pending() {
    use crate::features::syntax::{TokenSpan, Tokens};
    use std::sync::atomic::AtomicUsize;
    for language in [
        Language::PlainText,
        Language::JavaScript,
        Language::Rust,
        Language::CSharp,
    ] {
        let buffer = TextBuffer::new("Alpha123 中e\u{301}=\"value\"; 456 /*note*/ ".repeat(200));
        let (send, receive) = mpsc::channel();
        let (enter, entered) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let jobs = AtomicUsize::new(0);
        let worker = Worker::start(
            move |stamp| {
                send.send((stamp.document, stamp.snapshot_identity, stamp.version))
                    .ok();
            },
            move || {
                if jobs.fetch_add(1, Ordering::Relaxed) == 1 {
                    enter.send(()).unwrap();
                    gate.recv_timeout(Duration::from_secs(5)).unwrap();
                }
            },
        );
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
        let reference: Vec<_> = Tokens::new(buffer.text(), language, 0)
            .map(|(range, kind)| TokenSpan { range, kind })
            .collect();
        let expected = |window: &Window| {
            reference
                .iter()
                .filter(|span| {
                    span.range.end > window.range.start && span.range.start < window.range.end
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        let first = window(PAGE_BYTES - 160, PAGE_BYTES - 80);
        assert!(query(&first).is_empty());
        wait(&receive);
        let before = worker.stats();
        for shift in 0..70 {
            let current = window(PAGE_BYTES - 160 + shift, PAGE_BYTES - 80 + shift);
            let actual = query(&current);
            assert!(actual[0].complete);
            assert_eq!(actual[0].spans.as_ref(), expected(&current));
        }
        assert_eq!(
            worker.stats().index.scanned_bytes,
            before.index.scanned_bytes
        );
        let crossed = window(PAGE_BYTES - 40, PAGE_BYTES + 100);
        let pending = query(&crossed);
        assert!(!pending[0].complete);
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        for shift in 0..20 {
            let current = window(PAGE_BYTES - 40 + shift, PAGE_BYTES + 100 + shift);
            let actual = query(&current);
            assert!(!actual[0].complete);
            for known in &pending[0].spans[..] {
                if known.range.end > current.range.start && known.range.start < current.range.end {
                    assert!(
                        actual[0].spans.contains(known),
                        "moving the viewport must not erase a known token"
                    );
                }
            }
            assert!(actual[0].spans.iter().all(|span| reference.contains(span)));
        }
        release.send(()).unwrap();
        wait(&receive);
        let completed = query(&crossed);
        assert!(completed[0].complete);
        assert_eq!(completed[0].spans.as_ref(), expected(&crossed));
    }
}

#[test]
fn known_tokens_crossing_pages_remain_colored_when_the_original_page_leaves_the_viewport() {
    let buffer = TextBuffer::new(format!("\"{}\"", "中e\u{301}abc".repeat(PAGE_BYTES)));
    let (send, receive) = mpsc::channel();
    let (enter, entered) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let jobs = std::sync::atomic::AtomicUsize::new(0);
    let worker = Worker::start(
        move |stamp| {
            send.send((stamp.document, stamp.snapshot_identity, stamp.version))
                .ok();
        },
        move || {
            if jobs.fetch_add(1, Ordering::Relaxed) == 1 {
                enter.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        },
    );
    let query = |start, end| {
        worker.query(
            PaneId::new(1),
            stamp(1, &buffer),
            Source::new(&buffer),
            Language::PlainText,
            vec![Window {
                line: buffer.line_range(0),
                range: buffer.grapheme_floor(start)..buffer.grapheme_floor(end),
            }],
        )
    };
    query(0, 100);
    wait(&receive);
    let known = query(0, 100).remove(0);
    let moved = query(PAGE_BYTES + 40, PAGE_BYTES + 120).remove(0);
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(moved.complete);
    assert_eq!(moved.spans, known.spans);
    assert_eq!(moved.spans[0].kind, crate::features::syntax::TokenKind::Str);
    let further = query(2 * PAGE_BYTES + 40, 2 * PAGE_BYTES + 120).remove(0);
    assert!(further.complete);
    assert_eq!(further.spans, known.spans);
    release.send(()).unwrap();
    wait(&receive);
}

fn query(worker: &Worker, pane: u64, id: u64, buffer: &TextBuffer) -> Vec<WindowTokens> {
    worker.query(
        PaneId::new(pane),
        stamp(id, buffer),
        Source::new(buffer),
        Language::JavaScript,
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
    let worker = Worker::start(
        move |stamp| {
            send.send((stamp.document, stamp.snapshot_identity, stamp.version))
                .ok();
        },
        move || {
            if first.swap(false, Ordering::Relaxed) {
                entered.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(5))
                    .expect("release worker gate");
            }
        },
    );
    (worker, entering, release, receive)
}

#[test]
fn ui_queries_never_wait_for_lexing_and_duplicate_frames_are_coalesced() {
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
    let mut reference = Index::default();
    let expected = reference
        .prepare(
            FileId::new(1),
            Source::new(&buffer),
            Language::JavaScript,
            &[Window {
                line: buffer.line_range(0),
                range: buffer.line_range(0),
            }],
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(actual[0].spans, expected[0].spans);
    assert!(receive.try_recv().is_err());
}

#[test]
fn switching_files_and_split_panes_reuse_completed_windows_and_index() {
    let (send, receive) = mpsc::channel();
    let worker = Worker::new(move |stamp| {
        send.send((stamp.document, stamp.snapshot_identity, stamp.version))
            .ok();
    });
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
    assert_eq!(
        worker.stats().index.scanned_bytes,
        before.index.scanned_bytes
    );
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
    let worker = Worker::new(move |stamp| {
        send.send((stamp.document, stamp.snapshot_identity, stamp.version))
            .ok();
    });
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
            Language::JavaScript,
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
        "a reopened document must not see the old windows"
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
