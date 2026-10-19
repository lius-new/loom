//! P3a measurements for the 100 ms first-colour budget (design 10.5, 14.4).
//! Run in release mode, they print raw samples and p50/p95/max:
//!
//! `cargo test --release --locked -p loom treesitter_bench -- --ignored --nocapture --test-threads=1`
//!
//! Every group uses the fixed 1 MiB sample from `baseline::generate` (see
//! tests/fixtures/languages/generated.sha256) and 50 samples after one warm-up.
use super::Source;
use super::bench_support::{MIB, SAMPLES, ms, report, timed, viewport};
use super::test_support::compile;
use super::treesitter::parse;
use crate::domain::identity::FileId;
use crate::domain::language::{ResolvedLanguage, builtin};
use crate::features::syntax::baseline::generate;
use loom_editor_core::text::{EditKind, EditorMut, Selection, TextBuffer};
use std::{sync::atomic::AtomicBool, time::Instant};

#[test]
#[ignore = "P3a measurement; run in release mode"]
fn treesitter_bench_resources_and_one_megabyte_parses() {
    println!("BENCH environment: 1 MiB samples from baseline::generate, {SAMPLES} samples");
    for name in crate::domain::language::grammar_names() {
        let started = Instant::now();
        let language = super::test_support::language(name).unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).unwrap();
        println!(
            "BENCH parser creation {name}: {:.3}ms",
            ms(started.elapsed())
        );
    }
    for name in crate::domain::language::grammar_names() {
        let started = Instant::now();
        let compiled = compile(name).unwrap();
        println!(
            "BENCH query compile (cold) {name}: {:.2}ms, {} patterns, {} captures",
            ms(started.elapsed()),
            compiled.query.pattern_count(),
            compiled.query.capture_names().len()
        );
    }
    for name in crate::domain::language::grammar_names() {
        let started_samples = timed(|| compile(name).unwrap());
        report(&format!("query compile (repeat) {name}"), started_samples);
    }

    // Independent parser, no old tree: the first parse of a document.
    let cases = [
        ("rust", "rust"),
        ("javascript", "javascript"),
        ("typescript", "typescript"),
        ("tsx", "typescript"),
        ("c_sharp", "csharp"),
    ];
    for (grammar, sample) in cases {
        let text = generate(sample, MIB);
        let language = super::test_support::language(grammar).unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).unwrap();
        let cancel = AtomicBool::new(false);
        let samples = timed(|| parse(&mut parser, &text, None, &cancel).unwrap());
        report(
            &format!("full parse 1 MiB {grammar} ({} bytes)", text.len()),
            samples,
        );
    }
}

#[test]
#[ignore = "P3a measurement; run in release mode"]
fn treesitter_bench_viewport_query_and_incremental_update() {
    for (name, grammar_sample) in [("rust", "rust"), ("javascript", "javascript")] {
        let language: ResolvedLanguage = builtin(name);
        let mut buffer = TextBuffer::new(generate(grammar_sample, MIB));
        let mut index = crate::features::syntax::test_support::index();
        let cancel = AtomicBool::new(false);
        let id = FileId::new(1);
        let middle = buffer.line_count() / 2;
        // Warm: the tree exists, the query is compiled; only the query runs.
        index.prepare(
            id,
            Source::new(&buffer),
            language,
            &viewport(&buffer, 0, 1),
            &cancel,
        );
        let windows = viewport(&buffer, middle, 50);
        let samples = timed(|| {
            index
                .prepare(id, Source::new(&buffer), language, &windows, &cancel)
                .unwrap()
        });
        report(
            &format!("viewport query, 50 lines, warm tree, {name}"),
            samples,
        );

        // One typed character, then re-read the same viewport.
        let samples = timed(|| {
            let at = buffer.line_range(middle).start;
            EditorMut::new(&mut buffer, &mut Selection::default(), Vec::new())
                .replace(at..at, "x", EditKind::Other)
                .unwrap();
            let started = Instant::now();
            let windows = viewport(&buffer, middle, 50);
            index
                .prepare(id, Source::new(&buffer), language, &windows, &cancel)
                .unwrap();
            started.elapsed()
        });
        report(
            &format!("one-character edit, incremental parse + 50-line query, {name}"),
            samples,
        );
        let stats = index.stats;
        println!(
            "BENCH {name} stats: full={} incremental={} parse_time={:.1}ms query_time={:.1}ms",
            stats.full_parses,
            stats.incremental_parses,
            ms(stats.parse_time),
            ms(stats.query_time)
        );
    }
}

/// How much a first colour could save by parsing only the start of a large
/// document before the full parse (an optimisation considered for P3b).
#[test]
#[ignore = "P3a measurement; run in release mode"]
fn treesitter_bench_prefix_only_parse() {
    for (grammar, sample) in [
        ("rust", "rust"),
        ("javascript", "javascript"),
        ("c_sharp", "csharp"),
    ] {
        let text = generate(sample, MIB);
        let language = super::test_support::language(grammar).unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).unwrap();
        for prefix in [16 * 1024, 64 * 1024, 256 * 1024] {
            // End the prefix on a line break so the cut is a clean one.
            let end = text[prefix..]
                .find('\n')
                .map_or(text.len(), |at| prefix + at + 1);
            let line_breaks = |s: &str| s.bytes().filter(|&b| b == b'\n').count();
            let last_line = text[..end].rfind('\n').map_or(0, |at| end - at - 1);
            let range = tree_sitter::Range {
                start_byte: 0,
                end_byte: end,
                start_point: tree_sitter::Point { row: 0, column: 0 },
                end_point: tree_sitter::Point {
                    row: line_breaks(&text[..end]),
                    column: last_line,
                },
            };
            parser.set_included_ranges(&[range]).unwrap();
            let cancel = AtomicBool::new(false);
            let samples = timed(|| parse(&mut parser, &text, None, &cancel).unwrap());
            report(
                &format!("prefix-only parse {} KiB of 1 MiB {grammar}", prefix / 1024),
                samples,
            );
        }
        parser.set_included_ranges(&[]).unwrap();
    }
}

/// The cost of carrying known spans across recorded edits, for the limits in
/// `presentation` (design 10.4).
#[test]
#[ignore = "P3a measurement; run in release mode"]
fn treesitter_bench_projection_limits() {
    use super::TokenSpan;
    use super::presentation::project_window;
    use crate::features::highlighting::HighlightId;
    for spans_count in [200usize, 1_000, 4_000] {
        for (revisions, edits) in [(1usize, 1usize), (1, 8), (4, 8), (16, 4), (16, 8)] {
            // Spans of 4 bytes alternating between a class and plain text.
            let len = spans_count * 4;
            let spans: Vec<TokenSpan> = (0..spans_count)
                .map(|i| TokenSpan {
                    range: i * 4..i * 4 + 4,
                    class: (i % 2 == 0).then_some(HighlightId::KEYWORD),
                })
                .collect();
            let mut buffer = TextBuffer::new("x".repeat(len));
            let since = buffer.clone();
            let mut state = 0x9e37_79b9_u64;
            for _ in 0..revisions {
                let length = buffer.len();
                let mut list = Vec::new();
                for k in 0..edits {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    let slot = length / edits;
                    let at = k * slot + (state as usize % slot.saturating_sub(4).max(1));
                    list.push(loom_editor_core::text::Edit::replace(at..at + 1, "yy"));
                }
                EditorMut::new(&mut buffer, &mut Selection::default(), Vec::new())
                    .edit(EditKind::Other, list, |_, _| Selection::default())
                    .unwrap();
            }
            let chain = buffer
                .revisions_since(since.version(), since.snapshot_identity())
                .unwrap();
            let samples = (0..200)
                .map(|_| {
                    let started = Instant::now();
                    let projected = project_window(0..len, &spans, &chain, buffer.len());
                    let elapsed = started.elapsed();
                    std::hint::black_box(&projected);
                    elapsed
                })
                .collect::<Vec<_>>();
            let mut sorted = samples.clone();
            sorted.sort();
            let us = |q: f64| {
                sorted[((sorted.len() - 1) as f64 * q).round() as usize].as_secs_f64() * 1e6
            };
            println!(
                "BENCH projection S={spans_count} R={revisions} E/rev={edits} (total {}): p50={:.1}us p95={:.1}us max={:.1}us{}",
                revisions * edits,
                us(0.5),
                us(0.95),
                us(1.0),
                if project_window(0..len, &spans, &chain, buffer.len()).is_err() {
                    " [refused by limits]"
                } else {
                    ""
                }
            );
        }
    }
}

/// First colour by document size, without the thread hop: a fresh tree index
/// (query not compiled, no tree) parses and reads the first 50 lines.
#[test]
#[ignore = "budget check; run in release mode on the reference machine"]
fn treesitter_bench_first_colour_by_size() {
    for name in ["rust", "javascript", "typescript", "csharp"] {
        let language = builtin(name);
        for kib in [64usize, 256, 512, 1024] {
            let buffer = TextBuffer::new(generate(name, kib * 1024));
            let windows = viewport(&buffer, 0, 50);
            let cancel = AtomicBool::new(false);
            let samples = timed(|| {
                let mut index = crate::features::syntax::test_support::index();
                index
                    .prepare(
                        FileId::new(1),
                        Source::new(&buffer),
                        language,
                        &windows,
                        &cancel,
                    )
                    .unwrap()
            });
            let p95 = report(
                &format!("first colour, cold index, {name} {kib} KiB"),
                samples,
            );
            // The budgets decided in P3a (design 10.5): p95 from a cold start,
            // release build, on the reference machine.
            let budget = if kib <= 256 {
                100.0
            } else if kib <= 512 {
                150.0
            } else {
                275.0
            };
            assert!(
                ms(p95) <= budget,
                "{name} {kib} KiB: p95 {:.1} ms is over the {budget} ms budget",
                ms(p95)
            );
        }
    }
}

/// Documents with embedded languages (P5): first colour from a cold index, and a
/// one-character edit once the layers exist. The layer count is printed because
/// it, not the document size alone, decides the cost.
#[test]
#[ignore = "P5 measurement; run in release mode"]
fn treesitter_bench_embedded_languages() {
    for name in ["json", "css", "html", "markdown"] {
        let language = builtin(name);
        for kib in [64usize, 256, 1024] {
            let buffer = TextBuffer::new(generate(name, kib * 1024));
            let windows = viewport(&buffer, 0, 50);
            let cancel = AtomicBool::new(false);
            let mut layers = 0;
            let mut overflows = 0;
            let samples = timed(|| {
                let mut index = crate::features::syntax::test_support::index();
                index
                    .prepare(
                        FileId::new(1),
                        Source::new(&buffer),
                        language,
                        &windows,
                        &cancel,
                    )
                    .unwrap();
                layers = index.stats.layers;
                overflows = index.stats.layer_overflows;
            });
            let p95 = report(
                &format!(
                    "first colour, cold index, {name} {kib} KiB (regions {layers}, over limit {overflows})"
                ),
                samples,
            );
            // Measured in P5 on the reference machine, release build. Markdown is
            // limited by its block grammar (about 360 ms per MiB), so it has its
            // own tiers.
            let budget = match (name, kib) {
                ("markdown", ..=64) => 100.0,
                ("markdown", ..=256) => 250.0,
                ("markdown", _) => 900.0,
                (_, ..=256) => 100.0,
                _ => 275.0,
            };
            assert!(
                ms(p95) <= budget,
                "{name} {kib} KiB: p95 {:.1} ms is over the {budget} ms budget",
                ms(p95)
            );
        }
    }
    for name in ["html", "markdown"] {
        let language = builtin(name);
        let mut buffer = TextBuffer::new(generate(name, MIB));
        let mut index = crate::features::syntax::test_support::index();
        let cancel = AtomicBool::new(false);
        let id = FileId::new(1);
        let middle = buffer.line_count() / 2;
        index.prepare(
            id,
            Source::new(&buffer),
            language,
            &viewport(&buffer, 0, 1),
            &cancel,
        );
        let samples = timed(|| {
            let at = buffer.line_range(middle).start;
            EditorMut::new(&mut buffer, &mut Selection::default(), Vec::new())
                .replace(at..at, "x", EditKind::Other)
                .unwrap();
            let started = Instant::now();
            let windows = viewport(&buffer, middle, 50);
            index
                .prepare(id, Source::new(&buffer), language, &windows, &cancel)
                .unwrap();
            started.elapsed()
        });
        report(
            &format!("one-character edit, {name} 1 MiB with layers"),
            samples,
        );
        let stats = index.stats;
        println!(
            "BENCH {name} stats: layers={} layer_parses={} layer_reuses={} overflows={} parse_time={:.1}ms",
            stats.layers,
            stats.layer_parses,
            stats.layer_reuses,
            stats.layer_overflows,
            ms(stats.parse_time)
        );
    }
}

#[test]
#[ignore = "P5 diagnosis; run in release mode"]
fn treesitter_bench_markdown_pieces() {
    let text = generate("markdown", MIB);
    let cancel = AtomicBool::new(false);
    let mut block = tree_sitter::Parser::new();
    block
        .set_language(&super::test_support::language("markdown").unwrap())
        .unwrap();
    let started = Instant::now();
    let tree = parse(&mut block, &text, None, &cancel).unwrap();
    println!(
        "BENCH markdown block parse 1 MiB: {:.1}ms",
        ms(started.elapsed())
    );
    let mut ranges = Vec::new();
    let mut cursor = tree.walk();
    fn walk(cursor: &mut tree_sitter::TreeCursor, out: &mut Vec<tree_sitter::Range>) {
        loop {
            if cursor.node().kind() == "inline" {
                out.push(cursor.node().range());
            } else if cursor.goto_first_child() {
                walk(cursor, out);
                cursor.goto_parent();
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    walk(&mut cursor, &mut ranges);
    println!("BENCH inline ranges: {}", ranges.len());
    let mut inline = tree_sitter::Parser::new();
    inline
        .set_language(&super::test_support::language("markdown_inline").unwrap())
        .unwrap();
    inline.set_included_ranges(&ranges).unwrap();
    let started = Instant::now();
    let inline_tree = parse(&mut inline, &text, None, &cancel).unwrap();
    println!(
        "BENCH markdown inline parse (all ranges): {:.1}ms",
        ms(started.elapsed())
    );
    // The same ranges in chunks.
    let started = Instant::now();
    let mut total = 0;
    for chunk in ranges.chunks(64) {
        inline.set_included_ranges(chunk).unwrap();
        total += parse(&mut inline, &text, None, &cancel)
            .unwrap()
            .root_node()
            .child_count();
    }
    println!(
        "BENCH markdown inline parse in chunks of 64: {:.1}ms ({total})",
        ms(started.elapsed())
    );
    drop(inline_tree);
}
