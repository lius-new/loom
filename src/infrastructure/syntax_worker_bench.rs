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
