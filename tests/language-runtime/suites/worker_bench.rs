//! P3a measurement of the first colour through the real worker: queue, parse
//! and query together. Run in release mode:
//!
//! `cargo test --release --locked -p loom treesitter_bench -- --ignored --nocapture --test-threads=1`
use super::*;
use crate::domain::language::builtin;
use crate::features::syntax::baseline::generate;
use crate::features::syntax::bench_support::{MIB, ms, report, timed, viewport};
use loom_editor_core::text::TextBuffer;
use std::time::Instant;
use std::{sync::mpsc, time::Duration};

/// Request to ready through the real worker: queue, parse and query together.
#[test]
#[ignore = "P3a measurement; run in release mode"]
fn treesitter_bench_first_colour_through_the_worker() {
    for name in ["rust", "javascript"] {
        let language = builtin(name);
        let text = generate(name, MIB);
        let ask = |worker: &Worker, id: u64, buffer: &TextBuffer| {
            let windows = viewport(buffer, 0, 50);
            let stamp = DocumentStamp::new(FileId::new(id), 1, 0, buffer);
            worker.query(
                PaneId::new(1),
                stamp,
                Source::new(buffer),
                language,
                windows,
            )
        };
        let buffer = TextBuffer::new(text.clone());

        // Cold: a new worker has no parser and no compiled query.
        let cold = timed(|| {
            let (send, receive) = mpsc::channel();
            let worker = Worker::new(move |_| {
                send.send(()).ok();
            });
            let started = Instant::now();
            assert!(ask(&worker, 1, &buffer).is_empty());
            receive.recv_timeout(Duration::from_secs(20)).unwrap();
            let ready = started.elapsed();
            assert_eq!(ask(&worker, 1, &buffer).len(), 50);
            let stats = worker.stats().tree;
            assert_eq!(stats.queries_compiled, 1);
            worker.shutdown();
            ready
        });
        report(
            &format!("first colour, cold worker (compile + parse + query), {name}"),
            cold,
        );

        // Warm: parser and query exist, the document's tree does not.
        let (send, receive) = mpsc::channel();
        let worker = Worker::new(move |_| {
            send.send(()).ok();
        });
        let small = TextBuffer::new(generate(name, 2 * 1024));
        ask(&worker, 1, &small);
        receive.recv_timeout(Duration::from_secs(20)).unwrap();
        let mut next_id = 100;
        let warm = timed(|| {
            next_id += 1;
            let started = Instant::now();
            ask(&worker, next_id, &buffer);
            receive.recv_timeout(Duration::from_secs(20)).unwrap();
            let ready = started.elapsed();
            worker.retain(vec![]);
            ready
        });
        report(
            &format!("first colour, warm worker (parse + query), {name}"),
            warm,
        );
        let stats = worker.stats().tree;
        println!(
            "BENCH {name} worker stats: full={} parse_time={:.1}ms query_time={:.1}ms compile_time={:.1}ms compiled={}",
            stats.full_parses,
            ms(stats.parse_time),
            ms(stats.query_time),
            ms(stats.query_compile_time),
            stats.queries_compiled
        );
        worker.shutdown();
    }
}

/// P3b acceptance: continuous typing in a 1 MiB document while every parse is
/// delayed by 150 ms. Ten commits a second for five seconds; each commit is
/// followed by the frame the editor would draw. No frame may fall back to plain
/// text for lines that were coloured before, and the frame must stay cheap.
#[test]
#[ignore = "acceptance measurement; run in release mode"]
fn treesitter_bench_continuous_typing_in_one_megabyte() {
    use crate::features::syntax::{Freshness, TokenSpan};
    use loom_editor_core::text::{EditKind, EditorMut, Selection};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    let (send, receive) = mpsc::channel();
    let slow = Arc::new(AtomicBool::new(false));
    let delay = slow.clone();
    let worker = Worker::start(
        move |stamp| {
            send.send(stamp.version).ok();
        },
        move || {
            if delay.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(150));
            }
        },
    );
    let language = builtin("rust");
    let mut buffer = TextBuffer::new(generate("rust", MIB));
    let middle = buffer.line_count() / 2;
    let windows = |buffer: &TextBuffer| viewport(buffer, middle - 25, 50);
    let ask = |buffer: &TextBuffer| {
        let stamp = DocumentStamp::new(FileId::new(1), 1, 0, buffer);
        worker.query(
            PaneId::new(1),
            stamp,
            Source::new(buffer),
            language,
            windows(buffer),
        )
    };
    assert!(ask(&buffer).is_empty());
    receive.recv_timeout(Duration::from_secs(30)).unwrap();
    let anchor = ask(&buffer);
    assert_eq!(anchor.len(), 50);
    let coloured_lines = |frame: &[crate::features::syntax::WindowTokens]| {
        frame
            .iter()
            .filter(|t| t.spans.iter().any(|s: &TokenSpan| s.class.is_some()))
            .count()
    };
    let baseline_lines = coloured_lines(&anchor);
    assert!(baseline_lines > 30);

    slow.store(true, Ordering::Relaxed);
    let edit_line = middle;
    let mut frame_times = Vec::new();
    let mut provisional = 0;
    for tick in 0..50 {
        let at = buffer.line_range(edit_line).end;
        EditorMut::new(&mut buffer, &mut Selection::default(), Vec::new())
            .replace(at..at, "x", EditKind::Other)
            .unwrap();
        let started = Instant::now();
        let frame = ask(&buffer);
        frame_times.push(started.elapsed());
        assert_eq!(
            frame.len(),
            50,
            "tick {tick}: every visible line is answered"
        );
        provisional += frame
            .iter()
            .filter(|t| t.freshness == Freshness::Provisional)
            .count();
        assert!(
            coloured_lines(&frame) >= baseline_lines - 1,
            "tick {tick}: lines that were coloured stayed coloured ({} of {baseline_lines})",
            coloured_lines(&frame)
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let p95 = report(
        "frame time while typing in 1 MiB (150 ms parse delay)",
        frame_times,
    );
    let stats = worker.stats();
    println!(
        "BENCH provisional windows={} (of {} drawn) refused={} broken_chain={} projection_time={:.2}ms parse_time={:.1}ms full={} incremental={}",
        stats.provisional.windows,
        50 * 50,
        stats.provisional.refused,
        stats.provisional.broken_chain,
        ms(stats.provisional.time),
        ms(stats.tree.parse_time),
        stats.tree.full_parses,
        stats.tree.incremental_parses
    );
    assert!(
        provisional > 0,
        "the delay made carried-over colours necessary"
    );
    assert!(ms(p95) <= 2.0, "a frame costs {:.2} ms at p95", ms(p95));
    assert_eq!(
        stats.provisional.refused + stats.provisional.broken_chain,
        0
    );
}
