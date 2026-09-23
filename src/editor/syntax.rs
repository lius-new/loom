//! Lightweight syntax highlighter.
//!
//! A tiny hand-rolled lexer (no regex/parser dependency) that classifies each
//! token and maps it to a `theme` color. It is intentionally simple and can be
//! replaced by a real tree-sitter grammar without touching the editor view.

use lgui::prelude::Color;

use crate::model::document::Language;
use crate::theme;

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
        Tok::Kw => theme::KW,
        Tok::Fn => theme::FUNC,
        Tok::Type => theme::TYPE,
        Tok::Str => theme::STR,
        Tok::Comment => theme::COMMENT,
        Tok::Num => theme::NUM,
        Tok::Prop => theme::PROP,
        Tok::Plain => theme::ZINC_200,
    }
}

/// Split one source line into `(text, color)` spans.
pub fn highlight_line(line: &str, lang: Language) -> Vec<(String, Color)> {
    tokenize(line, lang)
        .into_iter()
        .map(|(text, kind)| (text, color_for(kind)))
        .collect()
}

fn tokenize(line: &str, lang: Language) -> Vec<(String, Tok)> {
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();
    let mut out: Vec<(String, Tok)> = Vec::new();
    let mut i = 0;

    while i < n {
        let c = chars[i];

        // Line comment
        if c == '/' && i + 1 < n && chars[i + 1] == '/' {
            let rest: String = chars[i..].iter().collect();
            out.push((rest, Tok::Comment));
            break;
        }

        // String literal (single/double quote, incl. C# `$"` interpolation)
        if c == '"' || c == '\'' || (c == '$' && i + 1 < n && chars[i + 1] == '"') {
            let start = i;
            let quote = if c == '$' { '"' } else { c };
            if c == '$' {
                i += 1;
            }
            i += 1;
            while i < n && chars[i] != quote {
                i += 1;
            }
            if i < n {
                i += 1; // closing quote
            }
            let text: String = chars[start..i].iter().collect();
            out.push((text, Tok::Str));
            continue;
        }

        // Number
        if c.is_ascii_digit() {
            let start = i;
            while i < n && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            out.push((text, Tok::Num));
            continue;
        }

        // Identifier / keyword
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < n && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            let kind = classify(&chars, start, i, &text, lang);
            out.push((text, kind));
            continue;
        }

        // Plain run (whitespace + punctuation)
        let start = i;
        while i < n && !is_special_start(&chars, i) {
            i += 1;
        }
        let text: String = chars[start..i].iter().collect();
        if !text.is_empty() {
            out.push((text, Tok::Plain));
        }
    }

    out
}

fn is_special_start(chars: &[char], i: usize) -> bool {
    let c = chars[i];
    if c.is_alphanumeric() || c == '_' || c == '"' || c == '\'' {
        return true;
    }
    if c == '$' && i + 1 < chars.len() && chars[i + 1] == '"' {
        return true;
    }
    c == '/' && i + 1 < chars.len() && chars[i + 1] == '/'
}

fn classify(chars: &[char], start: usize, end: usize, text: &str, lang: Language) -> Tok {
    if is_keyword(text, lang) {
        return Tok::Kw;
    }
    if is_builtin_type(text, lang) {
        return Tok::Type;
    }
    if prev_word_is(chars, start, "new") {
        return Tok::Type; // e.g. `new SignJWT(` / `new Uint8Array(`
    }
    if end < chars.len() && chars[end] == '(' {
        return Tok::Fn;
    }
    if text.chars().next().is_some_and(|c| c.is_uppercase()) {
        return Tok::Type;
    }
    if text.starts_with('_') {
        return Tok::Prop; // convention: `_cache`, `_logger`
    }
    if prev_significant_is_dot(chars, start) {
        return Tok::Prop;
    }
    Tok::Plain
}

fn prev_word_is(chars: &[char], start: usize, word: &str) -> bool {
    let mut i = start;
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    let end = i;
    while i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
        i -= 1;
    }
    let prev: String = chars[i..end].iter().collect();
    prev == word
}

fn prev_significant_is_dot(chars: &[char], start: usize) -> bool {
    let mut i = start;
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    i > 0 && chars[i - 1] == '.'
}

fn is_keyword(text: &str, lang: Language) -> bool {
    let list: &[&str] = match lang {
        Language::CSharp => &[
            "namespace", "public", "sealed", "class", "private", "readonly", "async", "if",
            "throw", "new", "return", "await", "using", "static", "var", "true", "false",
            "nameof", "this", "else", "null", "for", "foreach", "while", "get", "set",
            "override", "virtual", "out", "ref", "in", "params", "is", "as", "internal",
            "protected", "interface", "struct", "enum",
        ],
        Language::Rust => &[
            "pub", "struct", "impl", "fn", "self", "let", "mut", "use", "mod", "if", "else",
            "match", "for", "while", "loop", "return", "true", "false", "where", "type",
            "trait", "enum", "const", "static", "ref", "async", "await", "move", "in", "as",
            "box", "dyn", "unsafe", "extern", "crate", "super",
        ],
        Language::TypeScript | Language::JavaScript => &[
            "import", "from", "export", "async", "function", "return", "await", "new", "const",
            "let", "var", "if", "else", "for", "while", "throw", "try", "catch", "typeof",
            "interface", "type", "class", "extends", "implements", "public", "private",
            "readonly", "static", "true", "false", "null", "undefined", "this", "in", "of",
            "as",
        ],
        Language::PlainText => &[],
    };
    list.contains(&text)
}

fn is_builtin_type(text: &str, lang: Language) -> bool {
    let list: &[&str] = match lang {
        Language::CSharp => &[
            "string", "decimal", "bool", "void", "int", "long", "double", "float", "byte",
            "char", "object", "dynamic",
        ],
        Language::Rust => &[
            "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64", "isize", "f32",
            "f64", "bool", "str", "char",
        ],
        Language::TypeScript | Language::JavaScript => &[
            "string", "number", "boolean", "any", "void", "unknown", "never", "object",
            "symbol", "bigint",
        ],
        Language::PlainText => &[],
    };
    list.contains(&text)
}
