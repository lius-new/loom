//! Lightweight syntax highlighter.
//!
//! A tiny hand-rolled lexer (no regex/parser dependency) that classifies each
//! token and maps it to a `theme` color. It is intentionally simple and can be
//! replaced by a real tree-sitter grammar without touching the editor view.

use lgui::prelude::Color;

use crate::domain::identity::FileId;
use crate::domain::language::Language;
use crate::ui::theme;
use loom_editor_core::text::TextBuffer;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    ops::Range,
    sync::Arc,
};
use unicode_segmentation::UnicodeSegmentation;

type ColoredSpans = Vec<(String, Color)>;
#[derive(Default)]
struct HighlightCache {
    palette: Option<[Color; 8]>,
    lines: HashMap<Language, HashMap<String, ColoredSpans>>,
    len: usize,
}
thread_local! {
    static HIGHLIGHTS: RefCell<HighlightCache> = RefCell::new(HighlightCache::default());
    static WINDOWS: RefCell<VecDeque<IndexedLine>> = const { RefCell::new(VecDeque::new()) };
}

struct IndexedLine {
    document: FileId,
    snapshot: Arc<String>,
    range: Range<usize>,
    lang: Language,
    tokens: Vec<(Range<usize>, Tok)>,
    scanned: usize,
}

/// Tokenize from real line context, retaining token checkpoints as the viewport
/// advances. Only intersecting spans are copied into the render tree. A window
/// inside a string or comment must not be lexed as a new source line.
pub(crate) fn highlight_window(
    buffer: &TextBuffer,
    document: FileId,
    line: usize,
    window: Range<usize>,
    lang: Language,
) -> ColoredSpans {
    if window.is_empty() {
        return Vec::new();
    }
    let snapshot = buffer.text_snapshot();
    let source_range = buffer.line_range(line);
    let local = window.start - source_range.start..window.end - source_range.start;
    WINDOWS.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.retain(|entry| entry.document != document || Arc::ptr_eq(&entry.snapshot, &snapshot));
        let position = cache.iter().position(|entry| {
            entry.document == document
                && Arc::ptr_eq(&entry.snapshot, &snapshot)
                && entry.range == source_range
                && entry.lang == lang
        });
        let mut entry = position
            .and_then(|index| cache.remove(index))
            .unwrap_or_else(|| IndexedLine {
                document,
                snapshot,
                range: source_range,
                lang,
                tokens: Vec::new(),
                scanned: 0,
            });
        let source = &entry.snapshot[entry.range.clone()];
        if entry.scanned < local.end {
            for (range, kind) in Tokens::new(source, lang, entry.scanned) {
                entry.scanned = range.end;
                entry.tokens.push((range, kind));
                if entry.scanned >= local.end {
                    break;
                }
            }
        }
        let first = entry
            .tokens
            .partition_point(|(range, _)| range.end <= local.start);
        let mut result: ColoredSpans = Vec::new();
        for (range, kind) in entry.tokens[first..]
            .iter()
            .take_while(|(range, _)| range.start < local.end)
        {
            // A lexer boundary inside a combining sequence inherits the first
            // character's color, matching whole-line highlighting.
            let ceil = |offset| {
                let floor = buffer.grapheme_floor(offset);
                if floor == offset {
                    offset
                } else {
                    buffer.next_grapheme(floor)
                }
            };
            let from = ceil(entry.range.start + range.start).max(window.start);
            let to = ceil(entry.range.start + range.end).min(window.end);
            if from >= to {
                continue;
            }
            let color = color_for(*kind);
            let text = &buffer.text()[from..to];
            if let Some((previous, previous_color)) = result.last_mut()
                && *previous_color == color
            {
                previous.push_str(text);
            } else {
                result.push((text.to_owned(), color));
            }
        }
        cache.push_back(entry);
        while cache.len() > 256 {
            cache.pop_front();
        }
        result
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tok {
    Kw,
    Fn,
    Type,
    Str,
    Comment,
    Num,
    Prop,
    Plain,
}

fn color_for(kind: Tok) -> Color {
    match kind {
        Tok::Kw => theme::c().syntax.keyword,
        Tok::Fn => theme::c().syntax.function,
        Tok::Type => theme::c().syntax.type_name,
        Tok::Str => theme::c().syntax.string,
        Tok::Comment => theme::c().syntax.comment,
        Tok::Num => theme::c().syntax.number,
        Tok::Prop => theme::c().syntax.property,
        Tok::Plain => theme::c().text,
    }
}

/// Split one source line into `(text, color)` spans.
pub fn highlight_line(line: &str, lang: Language) -> Vec<(String, Color)> {
    let palette = [
        Tok::Kw,
        Tok::Fn,
        Tok::Type,
        Tok::Str,
        Tok::Comment,
        Tok::Num,
        Tok::Prop,
        Tok::Plain,
    ]
    .map(color_for);
    if let Some(spans) = HIGHLIGHTS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.palette != Some(palette) {
            *cache = HighlightCache {
                palette: Some(palette),
                ..Default::default()
            };
        }
        cache
            .lines
            .get(&lang)
            .and_then(|lines| lines.get(line))
            .cloned()
    }) {
        return spans;
    }
    let spans = highlight_uncached(line, lang);
    if line.len() <= 4096 {
        HIGHLIGHTS.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.len >= 256 {
                cache.lines.clear();
                cache.len = 0;
            }
            cache
                .lines
                .entry(lang)
                .or_default()
                .insert(line.to_owned(), spans.clone());
            cache.len += 1;
        });
    }
    spans
}

fn highlight_uncached(line: &str, lang: Language) -> Vec<(String, Color)> {
    let boundaries: std::collections::HashSet<usize> =
        line.grapheme_indices(true).map(|(i, _)| i).collect();
    let mut spans: Vec<(String, Color)> = Vec::new();
    let mut offset = 0;
    for (range, kind) in Tokens::new(line, lang, 0) {
        let text = &line[range];
        let color = color_for(kind);
        let len = text.len();
        if let Some((previous, previous_color)) = spans.last_mut()
            && (*previous_color == color || !boundaries.contains(&offset))
        {
            previous.push_str(&text);
        } else {
            spans.push((text.to_owned(), color));
        }
        offset += len;
    }
    spans
}

struct Tokens<'a> {
    source: &'a str,
    lang: Language,
    offset: usize,
}
impl<'a> Tokens<'a> {
    fn new(source: &'a str, lang: Language, offset: usize) -> Self {
        Self {
            source,
            lang,
            offset,
        }
    }
    fn peek(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }
    fn advance(&mut self) {
        if let Some(c) = self.peek() {
            self.offset += c.len_utf8();
        }
    }
    fn consume(&mut self, predicate: impl Fn(char) -> bool) {
        while self.peek().is_some_and(&predicate) {
            self.advance();
        }
    }
}
impl Iterator for Tokens<'_> {
    type Item = (Range<usize>, Tok);
    fn next(&mut self) -> Option<Self::Item> {
        let start = self.offset;
        let c = self.peek()?;
        let rest = &self.source[start..];
        let kind = if rest.starts_with("//") {
            self.offset = self.source.len();
            Tok::Comment
        } else if rest.starts_with("/*") {
            self.offset = rest[2..]
                .find("*/")
                .map_or(self.source.len(), |end| start + end + 4);
            Tok::Comment
        } else if c == '"' || c == '\'' || c == '`' || rest.starts_with("$\"") {
            let quote = if rest.starts_with("$\"") {
                self.advance();
                '"'
            } else {
                c
            };
            self.advance();
            while let Some(next) = self.peek() {
                self.advance();
                if next == '\\' {
                    self.advance();
                } else if next == quote {
                    break;
                }
            }
            Tok::Str
        } else if c.is_ascii_digit() {
            self.consume(|c| c.is_ascii_digit() || c == '.');
            Tok::Num
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            self.consume(|c| c.is_alphanumeric() || c == '_' || c == '$');
            let text = &self.source[start..self.offset];
            let before = self.source[..start].trim_end();
            let prev = before
                .rsplit(|c: char| !c.is_alphanumeric() && c != '_' && c != '$')
                .next()
                .unwrap_or("");
            if is_keyword(text, self.lang) {
                Tok::Kw
            } else if is_builtin_type(text, self.lang) || prev == "new" {
                Tok::Type
            } else if self.peek() == Some('(') {
                Tok::Fn
            } else if c.is_uppercase() {
                Tok::Type
            } else if c == '_' || before.ends_with('.') {
                Tok::Prop
            } else {
                Tok::Plain
            }
        } else {
            // Consume at least one character, including non-ASCII numerals.
            self.advance();
            while let Some(next) = self.peek() {
                let rest = &self.source[self.offset..];
                if next.is_ascii_digit()
                    || next.is_alphabetic()
                    || matches!(next, '_' | '$' | '"' | '\'' | '`')
                    || rest.starts_with("//")
                    || rest.starts_with("/*")
                {
                    break;
                }
                self.advance();
            }
            Tok::Plain
        };
        Some((start..self.offset, kind))
    }
}
fn is_keyword(text: &str, lang: Language) -> bool {
    let list: &[&str] = match lang {
        Language::CSharp => &[
            "namespace",
            "public",
            "sealed",
            "class",
            "private",
            "readonly",
            "async",
            "if",
            "throw",
            "new",
            "return",
            "await",
            "using",
            "static",
            "var",
            "true",
            "false",
            "nameof",
            "this",
            "else",
            "null",
            "for",
            "foreach",
            "while",
            "get",
            "set",
            "override",
            "virtual",
            "out",
            "ref",
            "in",
            "params",
            "is",
            "as",
            "internal",
            "protected",
            "interface",
            "struct",
            "enum",
        ],
        Language::Rust => &[
            "pub", "struct", "impl", "fn", "self", "let", "mut", "use", "mod", "if", "else",
            "match", "for", "while", "loop", "return", "true", "false", "where", "type", "trait",
            "enum", "const", "static", "ref", "async", "await", "move", "in", "as", "box", "dyn",
            "unsafe", "extern", "crate", "super",
        ],
        Language::TypeScript | Language::JavaScript => &[
            "import",
            "from",
            "export",
            "async",
            "function",
            "return",
            "await",
            "new",
            "const",
            "let",
            "var",
            "if",
            "else",
            "for",
            "while",
            "throw",
            "try",
            "catch",
            "typeof",
            "interface",
            "type",
            "class",
            "extends",
            "implements",
            "public",
            "private",
            "readonly",
            "static",
            "true",
            "false",
            "null",
            "undefined",
            "this",
            "in",
            "of",
            "as",
        ],
        Language::PlainText => &[],
    };
    list.contains(&text)
}

fn is_builtin_type(text: &str, lang: Language) -> bool {
    let list: &[&str] = match lang {
        Language::CSharp => &[
            "string", "decimal", "bool", "void", "int", "long", "double", "float", "byte", "char",
            "object", "dynamic",
        ],
        Language::Rust => &[
            "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64", "isize", "f32", "f64",
            "bool", "str", "char",
        ],
        Language::TypeScript | Language::JavaScript => &[
            "string", "number", "boolean", "any", "void", "unknown", "never", "object", "symbol",
            "bigint",
        ],
        Language::PlainText => &[],
    };
    list.contains(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn byte_colors(spans: ColoredSpans) -> Vec<Color> {
        spans
            .into_iter()
            .flat_map(|(text, color)| vec![color; text.len()])
            .collect()
    }

    #[test]
    fn window_colors_match_whole_line_across_strings_comments_and_identifiers() {
        let source = format!(
            "const _value=\"{}\\\"tail\";/*{}*/function Get(a){{return a.value+123;}}//{}",
            "中e\u{301}😀abcdefgh ".repeat(90),
            " comment ".repeat(180),
            "tail ".repeat(80)
        );
        let buffer = TextBuffer::new(source);
        for lang in [Language::PlainText, Language::JavaScript, Language::Rust] {
            let colors = byte_colors(highlight_line(buffer.line(0), lang));
            for column in [
                0,
                30,
                900,
                1500,
                2100,
                buffer.line_display_width(0).saturating_sub(80),
                60,
            ] {
                let range = buffer.display_column_range(0, column, column + 80).unwrap();
                let spans = highlight_window(&buffer, FileId::new(99), 0, range.clone(), lang);
                assert_eq!(
                    spans
                        .iter()
                        .map(|(text, _)| text.as_str())
                        .collect::<String>(),
                    &buffer.text()[range.clone()]
                );
                assert_eq!(byte_colors(spans), colors[range]);
            }
        }
    }

    #[test]
    fn viewport_inside_a_string_or_comment_keeps_the_original_token_color() {
        for (source, marker, expected) in [
            (
                format!("const value=\"{}\";", "abcdefgh".repeat(500)),
                "cdef",
                Tok::Str,
            ),
            (
                format!("const value=1;/*{}*/", "abcdefgh".repeat(500)),
                "cdef",
                Tok::Comment,
            ),
        ] {
            let buffer = TextBuffer::new(source);
            let start = buffer.text().find(marker).unwrap() + 800;
            let spans = highlight_window(
                &buffer,
                FileId::new(100),
                0,
                start..start + 20,
                Language::JavaScript,
            );
            assert!(spans.iter().all(|(_, color)| *color == color_for(expected)));
        }
    }

    #[test]
    fn changed_snapshot_invalidates_token_positions_and_unicode_numbers_make_progress() {
        let mut buffer = TextBuffer::new("\"abcd\"".to_owned());
        let id = FileId::new(101);
        let before = highlight_window(&buffer, id, 0, 0..buffer.len(), Language::JavaScript);
        buffer.reload("123456".into());
        let after = highlight_window(&buffer, id, 0, 0..buffer.len(), Language::JavaScript);
        assert_ne!(byte_colors(before), byte_colors(after));
        let text = "①２٣e\u{301}";
        assert_eq!(
            Tokens::new(text, Language::JavaScript, 0)
                .map(|(range, _)| &text[range])
                .collect::<String>(),
            text
        );
    }
}
