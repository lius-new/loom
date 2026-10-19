//! Embedded languages: HTML with script and style, Markdown with code fences.
use super::layers::{MAX_DEPTH, MAX_LIVE_LAYERS};
use super::treesitter::TreeIndex;
use super::{Source, TokenSpan, Window};
use crate::domain::identity::FileId;
use crate::domain::language::builtin;
use crate::features::highlighting::HighlightId;
use loom_editor_core::text::{Edit, EditKind, EditorMut, Selection, TextBuffer};
use std::sync::atomic::AtomicBool;

fn spans(index: &mut TreeIndex, buffer: &TextBuffer, language: &str) -> Vec<TokenSpan> {
    let window = Window {
        line: 0..buffer.len(),
        range: 0..buffer.len(),
    };
    index
        .prepare(
            FileId::new(1),
            Source::new(buffer),
            builtin(language),
            &[window],
            &AtomicBool::new(false),
        )
        .expect("not cancelled")
        .remove(0)
        .spans
        .to_vec()
}

/// The class names of the span that holds the first `needle`.
fn class_at(spans: &[TokenSpan], text: &str, needle: &str) -> Option<&'static str> {
    let at = text
        .find(needle)
        .unwrap_or_else(|| panic!("`{needle}` is not in the sample"));
    spans
        .iter()
        .find(|span| span.range.contains(&at))
        .and_then(|span| span.class)
        .map(HighlightId::name)
}

fn class_of(language: &str, text: &str, needle: &str) -> Option<&'static str> {
    let buffer = TextBuffer::new(text);
    class_at(
        &spans(
            &mut crate::features::syntax::test_support::index(),
            &buffer,
            language,
        ),
        text,
        needle,
    )
}

#[test]
fn html_script_and_style_use_javascript_and_css() {
    let text =
        "<p class=\"a\">hi</p>\n<script>const n = 42;</script>\n<style>a { color: red; }</style>\n";
    assert_eq!(class_of("html", text, "<p"), Some("punctuation"));
    assert_eq!(class_of("html", text, "p class"), Some("tag"));
    assert_eq!(class_of("html", text, "const"), Some("keyword"));
    assert_eq!(class_of("html", text, "42"), Some("number"));
    assert_eq!(class_of("html", text, "color"), Some("property"));
    assert_eq!(
        class_of("html", text, "red"),
        None,
        "plain values stay plain"
    );
}

#[test]
fn markdown_fences_use_the_language_named_on_the_fence() {
    let text = "# Title\n\n```rust\nfn main() { let x = 1; }\n```\n\n```js\nconst y = 2;\n```\n";
    assert_eq!(class_of("markdown", text, "Title"), Some("keyword"));
    assert_eq!(class_of("markdown", text, "fn main"), Some("keyword"));
    assert_eq!(class_of("markdown", text, "main"), Some("function"));
    assert_eq!(class_of("markdown", text, "const"), Some("keyword"));
    assert_eq!(class_of("markdown", text, "2;"), Some("number"));
}

#[test]
fn fence_names_are_matched_loosely() {
    for (info, expected) in [
        ("rs", "keyword"),
        ("RUST", "keyword"),
        ("rust,ignore", "keyword"),
        ("rust title=x", "keyword"),
    ] {
        let text = format!("```{info}\nfn a() {{}}\n```\n");
        assert_eq!(class_of("markdown", &text, "fn"), expected.into(), "{info}");
    }
    for info in ["nope", "{.rust}", ""] {
        let text = format!("```{info}\nfn a() {{}}\n```\n");
        assert_eq!(
            class_of("markdown", &text, "fn"),
            None,
            "{info:?} keeps the host colours"
        );
    }
}

#[test]
fn inline_markdown_is_coloured_by_the_inline_grammar() {
    let text = "Some *emphasis*, **strong**, `code` and [a link](http://x.y).\n";
    assert_eq!(class_of("markdown", text, "emphasis"), Some("constant"));
    assert_eq!(class_of("markdown", text, "strong"), Some("type"));
    assert_eq!(class_of("markdown", text, "code"), Some("string"));
    assert_eq!(class_of("markdown", text, "a link"), Some("function"));
    assert_eq!(class_of("markdown", text, "http"), Some("string"));
}

#[test]
fn inline_text_is_split_into_small_layers_and_only_the_visible_ones_are_parsed() {
    let mut text = String::new();
    for n in 0..200 {
        text.push_str(&format!(
            "Paragraph {n} with *emphasis*.

"
        ));
    }
    let buffer = TextBuffer::new(text.as_str());
    let mut index = crate::features::syntax::test_support::index();
    // Colour only the first line: just the first group is parsed.
    let first = Window {
        line: 0..30,
        range: 0..30,
    };
    index
        .prepare(
            FileId::new(1),
            Source::new(&buffer),
            builtin("markdown"),
            &[first],
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(
        index.layer_summary(FileId::new(1)),
        [("markdown_inline", 1)]
    );

    // Colour a line far down: its group is parsed, and the first may be let go.
    let far = text.find("Paragraph 190").unwrap();
    let window = Window {
        line: far..far + 30,
        range: far..far + 30,
    };
    let tokens = index
        .prepare(
            FileId::new(1),
            Source::new(&buffer),
            builtin("markdown"),
            &[window],
            &AtomicBool::new(false),
        )
        .unwrap();
    let emphasis = text[far..].find("emphasis").unwrap() + far;
    assert!(
        tokens[0]
            .spans
            .iter()
            .any(|s| s.range.contains(&emphasis) && s.class == Some(HighlightId::CONSTANT))
    );
    assert!(index.layer_summary(FileId::new(1)).len() <= 2);

    // The whole document needs every group.
    let all = spans(&mut index, &buffer, "markdown");
    assert!(index.layer_summary(FileId::new(1)).len() >= 5);
    assert!(all.iter().any(|s| s.class == Some(HighlightId::CONSTANT)));
}

#[test]
fn quoted_fences_do_not_treat_the_quote_marks_as_code() {
    let text = "> ```rust\n> fn a() {}\n> ```\n";
    assert_eq!(class_of("markdown", text, "fn"), Some("keyword"));
    let buffer = TextBuffer::new(text);
    let all = spans(
        &mut crate::features::syntax::test_support::index(),
        &buffer,
        "markdown",
    );
    // The `>` before the code is the block quote's, never the rust grammar's.
    let quote = text.find("> fn").unwrap();
    assert_ne!(class_at(&all, text, "> fn"), Some("keyword"));
    assert!(all.iter().all(|s| !(s.range.start < quote + 1
        && s.range.end > quote
        && s.class == Some(HighlightId::KEYWORD))));
}

#[test]
fn languages_nest_up_to_the_depth_limit() {
    // markdown > html > javascript
    let text = "```html\n<script>let a = 1;</script>\n```\n";
    assert_eq!(class_of("markdown", text, "let"), Some("keyword"));
    assert_eq!(class_of("markdown", text, "1;"), Some("number"));
    let buffer = TextBuffer::new(text);
    let mut index = crate::features::syntax::test_support::index();
    spans(&mut index, &buffer, "markdown");
    let summary = index.layer_summary(FileId::new(1));
    assert!(
        summary.contains(&("html", 1)) && summary.contains(&("javascript", 2)),
        "{summary:?}"
    );
}

#[test]
fn deeper_nesting_than_the_limit_keeps_the_host_colours() {
    // Markdown fences inside markdown fences, one more level than allowed.
    let levels = MAX_DEPTH as usize + 2;
    let mut text = String::new();
    for level in 0..levels {
        text.push_str(&"`".repeat(3 + levels - level));
        text.push_str("markdown\n");
    }
    text.push_str("fn deep() {}\n");
    for level in (0..levels).rev() {
        text.push_str(&"`".repeat(3 + levels - level));
        text.push('\n');
    }
    let buffer = TextBuffer::new(text.as_str());
    let mut index = crate::features::syntax::test_support::index();
    spans(&mut index, &buffer, "markdown");
    let summary = index.layer_summary(FileId::new(1));
    assert!(
        summary.iter().all(|(_, depth)| *depth <= MAX_DEPTH),
        "{summary:?}"
    );
    assert!(
        summary.iter().any(|(_, depth)| *depth == MAX_DEPTH),
        "{summary:?}"
    );
}

#[test]
fn too_many_regions_are_left_out_and_counted() {
    let mut text = String::new();
    for n in 0..MAX_LIVE_LAYERS + 20 {
        text.push_str(&format!("```rust\nfn f{n}() {{}}\n```\n\n"));
    }
    let buffer = TextBuffer::new(text.as_str());
    let mut index = crate::features::syntax::test_support::index();
    let all = spans(&mut index, &buffer, "markdown");
    assert_eq!(index.layer_summary(FileId::new(1)).len(), MAX_LIVE_LAYERS);
    assert!(index.stats.layer_overflows >= 20);
    assert_eq!(class_at(&all, &text, "fn f0"), Some("keyword"));
    let last = text.rfind("fn f").unwrap();
    assert_ne!(
        all.iter()
            .find(|s| s.range.contains(&last))
            .and_then(|s| s.class),
        Some(HighlightId::KEYWORD),
        "the region past the limit keeps its host colours"
    );
}

fn replace(buffer: &mut TextBuffer, from: usize, to: usize, with: &str) {
    EditorMut::new(buffer, &mut Selection::default(), Vec::new())
        .edit(
            EditKind::Other,
            vec![Edit::replace(from..to, with)],
            |_, _| Selection::default(),
        )
        .expect("a valid edit");
}

#[test]
fn editing_a_fence_marker_moves_the_region_to_another_language() {
    let text = "```rust\nfn a() {}\n```\n";
    let mut buffer = TextBuffer::new(text);
    let mut index = crate::features::syntax::test_support::index();
    let before = spans(&mut index, &buffer, "markdown");
    assert_eq!(class_at(&before, text, "fn"), Some("keyword"));

    replace(&mut buffer, 3, 7, "js");
    let now = buffer.text().to_owned();
    let after = spans(&mut index, &buffer, "markdown");
    assert_ne!(
        class_at(&after, &now, "fn"),
        Some("keyword"),
        "`fn` is not a javascript keyword"
    );
    // The incrementally updated result equals a fresh parse of the same text.
    assert_eq!(
        after,
        spans(
            &mut crate::features::syntax::test_support::index(),
            &TextBuffer::new(now.as_str()),
            "markdown"
        )
    );
}

#[test]
fn removing_the_closing_fence_extends_the_region_to_the_end() {
    let text = "```rust\nfn a() {}\n```\n\nfn b() {}\n";
    let mut buffer = TextBuffer::new(text);
    let mut index = crate::features::syntax::test_support::index();
    let before = spans(&mut index, &buffer, "markdown");
    let second = text.rfind("fn b").unwrap();
    assert_ne!(
        before
            .iter()
            .find(|s| s.range.contains(&second))
            .and_then(|s| s.class),
        Some(HighlightId::KEYWORD)
    );

    let close = text.find("```\n\n").unwrap();
    replace(&mut buffer, close, close + 4, "");
    let now = buffer.text().to_owned();
    let after = spans(&mut index, &buffer, "markdown");
    let second = now.rfind("fn b").unwrap();
    assert_eq!(
        after
            .iter()
            .find(|s| s.range.contains(&second))
            .and_then(|s| s.class),
        Some(HighlightId::KEYWORD),
        "now inside the open fence"
    );
    assert_eq!(
        after,
        spans(
            &mut crate::features::syntax::test_support::index(),
            &TextBuffer::new(now.as_str()),
            "markdown"
        )
    );
}

#[test]
fn an_unchanged_region_is_not_parsed_again() {
    let text = "```rust\nfn a() {}\n```\n\nSome text.\n";
    let mut buffer = TextBuffer::new(text);
    let mut index = crate::features::syntax::test_support::index();
    spans(&mut index, &buffer, "markdown");
    let parses = index.stats.layer_parses;
    assert!(parses >= 2, "the fence and the inline text");

    let end = buffer.len();
    replace(&mut buffer, end - 1, end - 1, "!");
    spans(&mut index, &buffer, "markdown");
    assert!(index.stats.layer_reuses >= 1, "the fence was reused");
    assert_eq!(
        index.stats.layer_parses,
        parses + 1,
        "only the inline text was parsed again"
    );
}
