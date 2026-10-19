use super::grammars;
use super::queries::compile_source;
use super::test_support::{compile, grammar_file};
use super::treesitter::{Capture, MAX_DOCUMENT_BYTES, TreeIndex, input_edit, tile};
use super::{Source, TokenSpan, Window};
use crate::domain::identity::FileId;
use crate::domain::language::{Registry, ResolvedLanguage, builtin, grammar_names};
use crate::features::highlighting::HighlightId;
use crate::features::syntax::baseline::generate;
use loom_editor_core::text::{Edit, EditKind, EditorMut, Selection, TextBuffer};
use std::sync::atomic::AtomicBool;

fn whole(buffer: &TextBuffer) -> Window {
    Window {
        line: 0..buffer.len(),
        range: 0..buffer.len(),
    }
}

fn spans_of(
    index: &mut TreeIndex,
    id: u64,
    buffer: &TextBuffer,
    language: ResolvedLanguage,
) -> Vec<TokenSpan> {
    let windows = [whole(buffer)];
    index
        .prepare(
            FileId::new(id),
            Source::new(buffer),
            language,
            &windows,
            &AtomicBool::new(false),
        )
        .expect("not cancelled")
        .remove(0)
        .spans
        .to_vec()
}

/// (text, class name) for each span of a freshly parsed document.
fn classes(language: &str, text: &str) -> Vec<(String, Option<&'static str>)> {
    let buffer = TextBuffer::new(text);
    spans_of(
        &mut crate::features::syntax::test_support::index(),
        1,
        &buffer,
        builtin(language),
    )
    .into_iter()
    .map(|span| {
        (
            text[span.range].to_owned(),
            span.class.map(HighlightId::name),
        )
    })
    .collect()
}

/// The class name of the span that holds the first `needle` in `text`.
fn class_of(language: &str, text: &str, needle: &str) -> Option<&'static str> {
    let at = text
        .find(needle)
        .unwrap_or_else(|| panic!("`{needle}` not in the sample"));
    let buffer = TextBuffer::new(text);
    spans_of(
        &mut crate::features::syntax::test_support::index(),
        1,
        &buffer,
        builtin(language),
    )
    .into_iter()
    .find(|span| span.range.contains(&at))
    .and_then(|span| span.class)
    .map(HighlightId::name)
}

fn rank(
    range: std::ops::Range<usize>,
    class: HighlightId,
    pattern: usize,
    order: usize,
) -> Capture {
    Capture {
        range,
        class,
        depth: 0,
        rank: (pattern, order),
    }
}

#[test]
fn grammar_table_resources_and_descriptions_agree() {
    let table: Vec<&str> = grammar_names();
    let mut sorted = table.clone();
    sorted.sort_unstable();
    assert_eq!(table, sorted, "NAMES is kept sorted");
    assert_eq!(
        table,
        super::test_support::names(),
        "the discovered packages match the registry"
    );

    let registry = Registry::builtin();
    let mut referenced: Vec<&str> = registry
        .languages()
        .iter()
        .flat_map(|language| language.variants.iter())
        .filter_map(|variant| variant.grammar)
        .map(|grammar| registry.grammar_name(grammar))
        .collect();
    // Grammars that exist only to be embedded in another language.
    referenced.extend(grammars::EMBEDDED_ONLY);
    referenced.sort_unstable();
    referenced.dedup();
    assert_eq!(
        table, referenced,
        "every grammar is used by a description or is embedded-only, and the reverse"
    );

    for name in &table {
        let language =
            super::test_support::language(name).unwrap_or_else(|| panic!("no factory for {name}"));
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&language)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        for file in ["highlights.scm", "SOURCE.md"] {
            assert!(
                grammar_file(name, file).is_some_and(|text| !text.trim().is_empty()),
                "{name}/{file}"
            );
        }
        let source = grammar_file(name, "SOURCE.md").unwrap();
        for needed in ["Grammar crate", "Upstream", "License", "Query", "Modified"] {
            assert!(source.contains(needed), "{name}/SOURCE.md lacks `{needed}`");
        }
    }
    assert!(super::test_support::language("brainfuck").is_none());
}

#[test]
fn every_built_in_query_compiles_and_every_capture_has_a_meaning() {
    for name in crate::domain::language::grammar_names() {
        let compiled = compile(name).unwrap_or_else(|error| panic!("{error}"));
        let captures = compiled.query.capture_names();
        assert!(!captures.is_empty(), "{name}");
        let coloured = (0..captures.len() as u32)
            .filter(|&index| compiled.class_of(index).is_some())
            .count();
        assert!(
            coloured * 2 > captures.len(),
            "{name}: most captures should resolve to a class"
        );
    }
}

#[test]
fn queries_using_unsupported_features_are_rejected_with_a_reason() {
    let language = super::test_support::language("rust").unwrap();
    let cases = [
        ("(identifier) @keyword", None),
        ("(identifier) @_helper", None),
        ("(identifier) @embedded", None),
        ("(identifier) @escape", None),
        ("(identifier) @keyword.control.import", None),
        ("((identifier) @keyword (#eq? @keyword \"x\"))", None),
        ("((identifier) @keyword (#match? @keyword \"^x\"))", None),
        (
            "((identifier) @keyword (#any-of? @keyword \"x\" \"y\"))",
            None,
        ),
        ("((identifier) @keyword (#is-not? local))", None),
        ("(identifier) @wibble", Some("@wibble")),
        ("(identifier) @local.definition", Some("@local.definition")),
        (
            "((identifier) @keyword (#wibble? @keyword))",
            Some("#wibble?"),
        ),
        (
            "((identifier) @keyword (#set! priority \"1\"))",
            Some("#set!"),
        ),
        ("((identifier) @keyword (#is? local))", Some("#is?")),
        (
            "((identifier) @keyword (#is-not? other))",
            Some("#is-not? other"),
        ),
        ("(no_such_node) @keyword", Some("no_such_node")),
        ("(identifier @keyword", Some("")),
    ];
    for (source, expected) in cases {
        match (compile_source(&language, source), expected) {
            (Ok(_), None) => {}
            (Err(error), Some(needle)) => {
                assert!(error.contains(needle), "{source}: {error}");
            }
            (result, expected) => panic!("{source}: {:?} vs {expected:?}", result.err()),
        }
    }
}

#[test]
fn rust_is_classified_across_lines_and_through_tricky_literals() {
    let text = "fn main() {\n    /* outer /* inner */ still comment */\n    let raw = r#\"has \"quote\" inside\"#;\n    let bytes = b\"bytes\\n\";\n    let x: &'a str = \"s\"; println!(\"{}\", 42u32);\n}\n";
    assert_eq!(class_of("rust", text, "fn"), Some("keyword"));
    assert_eq!(class_of("rust", text, "main"), Some("function"));
    assert_eq!(class_of("rust", text, "outer"), Some("comment"));
    assert_eq!(class_of("rust", text, "inner"), Some("comment"));
    assert_eq!(
        class_of("rust", text, "still comment"),
        Some("comment"),
        "nested block comments"
    );
    assert_eq!(class_of("rust", text, "has"), Some("string"));
    assert_eq!(
        class_of("rust", text, "quote"),
        Some("string"),
        "a raw string keeps its quotes"
    );
    assert_eq!(class_of("rust", text, "bytes\\"), Some("string"));
    assert_eq!(class_of("rust", text, "\\n"), Some("string.escape"));
    assert_eq!(
        class_of("rust", text, "a str"),
        Some("type"),
        "a lifetime name"
    );
    assert_eq!(class_of("rust", text, "'a"), Some("operator"), "its tick");
    assert_eq!(class_of("rust", text, "str"), Some("type.builtin"));
    assert_eq!(class_of("rust", text, "println"), Some("function.macro"));
    assert_eq!(class_of("rust", text, "let"), Some("keyword"));
}

#[test]
fn rust_survives_incomplete_input_instead_of_falling_back_to_plain_text() {
    let text =
        "fn broken( {\n    let s = \"unterminated;\n    struct S { a: u32,\n}\nfn after() {}\n";
    assert_eq!(class_of("rust", text, "let"), Some("keyword"));
    assert_eq!(class_of("rust", text, "struct"), Some("keyword"));
    assert_eq!(class_of("rust", text, "u32"), Some("type.builtin"));
    assert_eq!(
        class_of("rust", text, "after"),
        Some("function"),
        "later code is still understood"
    );
}

#[test]
fn javascript_handles_templates_regexes_comments_and_jsx() {
    let text = "const a = `x ${b + 1} y`; // line\n/* block\n   spans */ let r = /ab+c/gi.test(s) / 2;\nconst t = <div className=\"k\">{v}</div>;\nclass Foo { method() { return this.x; } }\n";
    assert_eq!(class_of("javascript", text, "const"), Some("keyword"));
    assert_eq!(class_of("javascript", text, "x $"), Some("string"));
    assert_eq!(class_of("javascript", text, "line"), Some("comment"));
    assert_eq!(
        class_of("javascript", text, "spans"),
        Some("comment"),
        "a block comment over two lines"
    );
    assert_eq!(
        class_of("javascript", text, "ab+c"),
        Some("string"),
        "regex literal"
    );
    assert_eq!(class_of("javascript", text, "div"), Some("tag"));
    assert_eq!(class_of("javascript", text, "className"), Some("attribute"));
    assert_eq!(class_of("javascript", text, "Foo"), Some("constructor"));
    assert_eq!(
        class_of("javascript", text, "method"),
        Some("function.method")
    );
    assert_eq!(
        class_of("javascript", text, "this"),
        Some("variable.builtin")
    );
}

#[test]
fn typescript_and_tsx_add_types_and_use_their_own_grammar() {
    let text = "interface Item<T extends object = {}> { readonly id: number; value: T }\nfunction build<T>(items: Item<T>[], index: number): T | undefined { return items[index]?.value as T }\n";
    assert_eq!(class_of("typescript", text, "interface"), Some("keyword"));
    assert_eq!(class_of("typescript", text, "Item"), Some("type"));
    assert_eq!(class_of("typescript", text, "number"), Some("type.builtin"));
    assert_eq!(class_of("typescript", text, "build"), Some("function"));
    assert_eq!(class_of("typescript", text, "readonly"), Some("keyword"));

    let registry = Registry::builtin();
    let typescript = registry.language_by_name("typescript").unwrap();
    let tsx = registry.resolve_variant(typescript, "tsx").unwrap();
    let component = "const App = () => <div title=\"t\"><Box />{count as number}</div>;\n";
    let buffer = TextBuffer::new(component);
    let spans = spans_of(
        &mut crate::features::syntax::test_support::index(),
        1,
        &buffer,
        tsx,
    );
    let class_at = |needle: &str| {
        let at = component.find(needle).unwrap();
        spans
            .iter()
            .find(|s| s.range.contains(&at))
            .and_then(|s| s.class)
            .map(HighlightId::name)
    };
    assert_eq!(class_at("div"), Some("tag"), "a lowercase element is a tag");
    assert_eq!(
        class_at("Box"),
        Some("type"),
        "a capitalised one is a component, so a type"
    );
    assert_eq!(class_at("title"), Some("attribute"));
    assert_eq!(class_at("number"), Some("type.builtin"));
}

#[test]
fn csharp_strings_comments_and_directives() {
    let text = "#if DEBUG\nusing System;\n#endif\nnamespace Demo {\n  public sealed class Service {\n    private readonly string _v = @\"verbatim \"\"q\"\"\";\n    public string Hi(string n) => $\"hi {n}\" + \"\"\"raw\"\"\"; // note\n    /* multi\n       line */\n    public int Count { get; set; }\n  }\n}\n";
    assert_eq!(class_of("csharp", text, "using"), Some("keyword"));
    assert_eq!(class_of("csharp", text, "sealed"), Some("keyword"));
    assert_eq!(class_of("csharp", text, "verbatim"), Some("string"));
    assert_eq!(class_of("csharp", text, "note"), Some("comment"));
    assert_eq!(
        class_of("csharp", text, "line */"),
        Some("comment"),
        "comment over two lines"
    );
    assert_eq!(class_of("csharp", text, "string _v"), Some("type.builtin"));
    assert_eq!(class_of("csharp", text, "Service"), Some("type"));
    let broken = "public class A { void M( {\n  var x = \"open;\n}\nclass B {}\n";
    assert_eq!(
        class_of("csharp", broken, "class B"),
        Some("keyword"),
        "incomplete input still highlights"
    );
}

#[test]
fn plain_text_has_no_grammar_and_a_grammar_less_window_is_one_plain_span() {
    assert!(builtin("plaintext").grammar.is_none());
    let text = "just words, 42, \"quoted\" // not a comment";
    let buffer = TextBuffer::new(text);
    // Direct use with a document over the size limit gives the same shape.
    let big = TextBuffer::new("a".repeat(MAX_DOCUMENT_BYTES + 1));
    let mut index = crate::features::syntax::test_support::index();
    let spans = spans_of(&mut index, 1, &big, builtin("rust"));
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].class, None);
    assert_eq!(index.stats.plain_fallbacks, 1);
    assert_eq!(
        index.stats.full_parses, 0,
        "an oversized document is never parsed"
    );
    let _ = buffer;
}

#[test]
fn tiling_prefers_the_shorter_range_then_the_later_pattern_and_fills_gaps() {
    type H = HighlightId;
    let class = |spans: &[TokenSpan]| {
        spans
            .iter()
            .map(|s| (s.range.clone(), s.class))
            .collect::<Vec<_>>()
    };
    // Equal ranges: the later pattern wins.
    let spans = tile(
        0..4,
        vec![rank(0..4, H::VARIABLE, 0, 0), rank(0..4, H::FUNCTION, 5, 1)],
    );
    assert_eq!(class(&spans), [(0..4, Some(H::FUNCTION))]);
    // Nested: the inner capture wins inside, the outer shows around it.
    let spans = tile(
        0..10,
        vec![
            rank(0..10, H::STRING, 9, 0),
            rank(3..5, H::STRING_ESCAPE, 0, 1),
        ],
    );
    assert_eq!(
        class(&spans),
        [
            (0..3, Some(H::STRING)),
            (3..5, Some(H::STRING_ESCAPE)),
            (5..10, Some(H::STRING))
        ]
    );
    // Gaps are plain; neighbours of one class merge; captures are clipped.
    let spans = tile(
        2..12,
        vec![
            rank(2..4, H::KEYWORD, 0, 0),
            rank(4..6, H::KEYWORD, 0, 1),
            rank(8..12, H::NUMBER, 0, 2),
        ],
    );
    assert_eq!(
        class(&spans),
        [
            (2..6, Some(H::KEYWORD)),
            (6..8, None),
            (8..12, Some(H::NUMBER))
        ]
    );
    assert_eq!(class(&tile(5..9, Vec::new())), [(5..9, None)]);
    // A partial overlap: each point takes the shorter capture covering it.
    let spans = tile(
        0..8,
        vec![rank(0..6, H::TYPE, 0, 0), rank(4..8, H::KEYWORD, 0, 1)],
    );
    assert_eq!(
        class(&spans),
        [(0..4, Some(H::TYPE)), (4..8, Some(H::KEYWORD))]
    );
}

#[test]
fn spans_tile_every_window_without_gaps_or_overlaps_and_rebuild_the_text() {
    for language in ["rust", "javascript", "typescript", "csharp"] {
        let text = generate(language, 100 * 1024);
        let buffer = TextBuffer::new(text.clone());
        let mut index = crate::features::syntax::test_support::index();
        let spans = spans_of(&mut index, 1, &buffer, builtin(language));
        assert_eq!(spans.first().unwrap().range.start, 0);
        assert_eq!(spans.last().unwrap().range.end, text.len());
        for pair in spans.windows(2) {
            assert_eq!(pair[0].range.end, pair[1].range.start, "{language}");
            assert_ne!(
                pair[0].class, pair[1].class,
                "{language}: equal neighbours are merged"
            );
        }
        let rebuilt: String = spans.iter().map(|s| &text[s.range.clone()]).collect();
        assert_eq!(rebuilt, text);
        assert!(
            spans.iter().any(|s| s.class.is_some()),
            "{language} colours something"
        );
        // A window in the middle sees the same classes as the whole document.
        let middle = text.len() / 2;
        let middle = text
            .char_indices()
            .map(|(i, _)| i)
            .find(|&i| i >= middle)
            .unwrap();
        let window = Window {
            line: 0..text.len(),
            range: middle..middle + 500.min(text.len() - middle),
        };
        let part = index
            .prepare(
                FileId::new(1),
                Source::new(&buffer),
                builtin(language),
                std::slice::from_ref(&window),
                &AtomicBool::new(false),
            )
            .unwrap()
            .remove(0);
        for span in part.spans.iter() {
            let whole_class = spans
                .iter()
                .find(|s| s.range.contains(&span.range.start))
                .unwrap()
                .class;
            assert_eq!(
                span.class,
                whole_class,
                "{language} at {} (window {:?}, part {:?}, text {:?})",
                span.range.start,
                window.range,
                part.spans.iter().take(4).collect::<Vec<_>>(),
                &text[span.range.start.saturating_sub(6)..(span.range.start + 6).min(text.len())]
            );
        }
        assert_eq!(
            index.stats.full_parses, 1,
            "windows of one version share one parse"
        );
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as usize
    }
}

#[test]
fn incremental_updates_give_the_same_spans_as_parsing_again() {
    let pieces = [
        "x", " ", "\n", "\"", "'", "/*", "*/", "//", "{", "}", "(", ")", "中", "😀", "\r\n", "fn ",
        "let ", "=", ";", "`", "${", "<", ">", "\\", "r#\"",
    ];
    let mut compared = 0;
    for language in ["rust", "javascript", "typescript", "csharp"] {
        let mut per_language = 0;
        let mut buffer = TextBuffer::new(generate(language, 4 * 1024));
        let mut kept = crate::features::syntax::test_support::index();
        let mut rng = Rng(0x2545_f491_4f6c_dd1d ^ language.len() as u64);
        let lang = builtin(language);
        spans_of(&mut kept, 1, &buffer, lang);
        for step in 0..60 {
            // One transaction of up to three disjoint edits.
            let len = buffer.len();
            let mut edits: Vec<Edit> = Vec::new();
            let mut cursor = 0;
            for _ in 0..1 + rng.next(3) {
                if cursor >= len {
                    break;
                }
                let mut start = cursor + rng.next(len - cursor);
                while !buffer.text().is_char_boundary(start) {
                    start += 1;
                }
                let mut end = (start + rng.next(6)).min(len);
                while !buffer.text().is_char_boundary(end) {
                    end += 1;
                }
                let text: String = (0..rng.next(3))
                    .map(|_| pieces[rng.next(pieces.len())])
                    .collect();
                if !(start == end && text.is_empty()) {
                    edits.push(Edit::replace(start..end, text));
                }
                cursor = end + 1;
            }
            let version = buffer.version();
            if EditorMut::new(&mut buffer, &mut Selection::default(), Vec::new())
                .edit(EditKind::Other, edits, |_, _| Selection::default())
                .is_err()
                || buffer.version() == version
            {
                // Rejected, or a no-op that left the text and version alone.
                continue;
            }
            let incremental = spans_of(&mut kept, 1, &buffer, lang);
            let fresh = spans_of(
                &mut crate::features::syntax::test_support::index(),
                1,
                &buffer,
                lang,
            );
            assert_eq!(incremental, fresh, "{language} step {step}");
            compared += 1;
            per_language += 1;
        }
        assert_eq!(
            kept.stats.incremental_parses, per_language,
            "{language}: every edited version was parsed from the previous tree"
        );
        assert!(per_language > 40, "{language}: enough edits were applied");
        assert_eq!(
            kept.stats.full_parses, 1,
            "{language}: only the first parse was complete"
        );
    }
    assert!(compared > 200);
}

#[test]
fn the_journal_edit_matches_the_text_for_multi_edit_transactions() {
    // abcdef with b -> XX and e -> YYY: the spec example from the design (5.3).
    let mut buffer = TextBuffer::new("abcdef");
    let since = buffer.clone();
    EditorMut::new(&mut buffer, &mut Selection::default(), Vec::new())
        .edit(
            EditKind::Other,
            vec![Edit::replace(1..2, "XX"), Edit::replace(4..5, "YYY")],
            |_, _| Selection::default(),
        )
        .unwrap();
    let revisions = buffer
        .revisions_since(since.version(), since.snapshot_identity())
        .unwrap();
    let edits: Vec<_> = revisions[0].edits.iter().rev().map(input_edit).collect();
    assert_eq!(
        edits
            .iter()
            .map(|e| (e.start_byte, e.old_end_byte, e.new_end_byte))
            .collect::<Vec<_>>(),
        [(4, 5, 7), (1, 2, 3)]
    );
}

#[test]
fn a_cancelled_parse_leaves_the_stored_tree_usable() {
    let language = builtin("rust");
    let mut buffer = TextBuffer::new(generate("rust", 64 * 1024));
    let mut index = crate::features::syntax::test_support::index();
    let before = spans_of(&mut index, 1, &buffer, language);
    EditorMut::new(&mut buffer, &mut Selection::default(), Vec::new())
        .replace(0..0, "// edit\n", EditKind::Other)
        .unwrap();
    let cancelled = AtomicBool::new(true);
    let windows = [whole(&buffer)];
    assert!(
        index
            .prepare(
                FileId::new(1),
                Source::new(&buffer),
                language,
                &windows,
                &cancelled
            )
            .is_none()
    );
    assert_eq!(
        index.stats.incremental_parses + index.stats.full_parses,
        1,
        "the cancelled parse is not counted"
    );
    // The next attempt continues from the old tree and matches a fresh parse.
    let after = spans_of(&mut index, 1, &buffer, language);
    let fresh = spans_of(
        &mut crate::features::syntax::test_support::index(),
        1,
        &buffer,
        language,
    );
    assert_eq!(after, fresh);
    assert_ne!(after, before);
    assert_eq!(index.stats.incremental_parses, 1);
}

#[test]
fn a_reload_a_language_change_and_a_long_chain_all_start_from_a_full_parse() {
    let rust = builtin("rust");
    let mut index = crate::features::syntax::test_support::index();
    let mut buffer = TextBuffer::new(generate("rust", 8 * 1024));
    spans_of(&mut index, 1, &buffer, rust);
    buffer.reload(generate("rust", 9 * 1024));
    spans_of(&mut index, 1, &buffer, rust);
    assert_eq!(
        (index.stats.full_parses, index.stats.incremental_parses),
        (2, 0),
        "reload breaks the chain"
    );
    spans_of(&mut index, 1, &buffer, builtin("javascript"));
    assert_eq!(
        index.stats.full_parses, 3,
        "another grammar never reuses the tree"
    );
    // More than the journal keeps: the chain is gone and the document is parsed again.
    for _ in 0..70 {
        EditorMut::new(&mut buffer, &mut Selection::default(), Vec::new())
            .replace(0..0, "x", EditKind::Other)
            .unwrap();
    }
    spans_of(&mut index, 1, &buffer, builtin("javascript"));
    assert_eq!(index.stats.full_parses, 4);
}

#[test]
fn only_the_most_recent_trees_are_kept_and_closed_documents_are_dropped() {
    let rust = builtin("rust");
    let mut index = crate::features::syntax::test_support::index();
    let buffer = TextBuffer::new(generate("rust", 2 * 1024));
    for id in 1..=20 {
        spans_of(&mut index, id, &buffer, rust);
    }
    assert_eq!(index.stats.trees, 16);
    // The oldest were dropped, so they parse again; recent ones do not.
    let before = index.stats.full_parses;
    spans_of(&mut index, 20, &buffer, rust);
    assert_eq!(index.stats.full_parses, before);
    spans_of(&mut index, 1, &buffer, rust);
    assert_eq!(index.stats.full_parses, before + 1);
    index.retain(&[FileId::new(1)]);
    assert_eq!(index.stats.trees, 1);
}

#[test]
fn windows_that_do_not_fit_the_text_are_refused() {
    let buffer = TextBuffer::new("fn a() {}");
    let window = Window {
        line: 0..100,
        range: 0..100,
    };
    let result = crate::features::syntax::test_support::index().prepare(
        FileId::new(1),
        Source::new(&buffer),
        builtin("rust"),
        &[window],
        &AtomicBool::new(false),
    );
    assert!(result.is_none());
}

#[test]
fn a_language_without_a_grammar_is_one_plain_span_and_builds_no_tree() {
    let text = "fn a() { 42 } // looks like code";
    assert_eq!(classes("plaintext", text), [(text.to_owned(), None)]);
    let mut index = crate::features::syntax::test_support::index();
    let buffer = TextBuffer::new(text);
    spans_of(&mut index, 1, &buffer, builtin("plaintext"));
    assert_eq!((index.stats.full_parses, index.stats.trees), (0, 0));
    // Every language that has a grammar colours something in this sample.
    for language in ["rust", "javascript", "typescript", "csharp"] {
        assert!(
            classes(language, "int x = 42; // note")
                .iter()
                .any(|(_, class)| class.is_some()),
            "{language}"
        );
    }
}
