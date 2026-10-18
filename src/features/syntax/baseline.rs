//! P0 baseline: deterministic source fixtures and the lexer's current limits.
//!
//! The generated samples are what later phases measure against (see the design
//! draft, sections 13–14). The hashes pin the bytes; the limitation tests record
//! behaviour that is deliberately NOT correct and is replaced in P3b, so they must
//! not be read as a language-correctness specification.
use super::{TokenKind, Tokens};
use crate::domain::language::Language;

const RUST: &[&str] = &[
    "pub fn compute_{n}<'a>(input: &'a str, limit: usize) -> Option<&'a str> {",
    "    let value = input.get(..limit).unwrap_or(\"fallback \\\"quoted\\\"\"); // trailing note",
    "    /* block {n} */ let raw = r#\"raw string \"{n}\"\"#;",
    "    if value.len() > {n} { return Some(value); }",
    "    None",
    "}",
];
const CSHARP: &[&str] = &[
    "public sealed class Service{n} : IDisposable",
    "{",
    "    private readonly Dictionary<string, int> _items = new();",
    "    public async Task<int> RunAsync(string name) // note {n}",
    "    {",
    "        var text = $\"item {name} \\\"{n}\\\"\"; /* inline */",
    "        return await Task.FromResult(text.Length + {n});",
    "    }",
    "}",
];
const JAVASCRIPT: &[&str] = &[
    "export async function handler{n}(request, options = {}) {",
    "  const pattern = /ab+c/gi; const ratio = request.total / {n};",
    "  const message = `value ${request.id} \\` end {n}`; // note",
    "  /* block {n} */ return { ok: true, count: ratio > 1 ? 2 : 3 };",
    "}",
];
const TYPESCRIPT: &[&str] = &[
    "export interface Item{n}<T extends object = {}> { readonly id: number; value: T; }",
    "export function build{n}<T>(items: Item{n}<T>[], index: number): T | undefined {",
    "  const found = items[index]?.value as T; // note",
    "  return found;",
    "}",
];

pub(crate) fn template(language: Language) -> &'static [&'static str] {
    match language {
        Language::Rust => RUST,
        Language::CSharp => CSHARP,
        Language::JavaScript => JAVASCRIPT,
        Language::TypeScript => TYPESCRIPT,
        Language::PlainText => &["Label {n} 中e\u{301} \"value\" 456 /* note */"],
    }
}

/// Whole lines only, so every fixture ends on a line boundary and is >= `bytes`.
pub(crate) fn generate(language: Language, bytes: usize) -> String {
    let lines = template(language);
    let mut out = String::with_capacity(bytes + 128);
    let mut n = 0usize;
    while out.len() < bytes {
        for line in lines {
            out.push_str(&line.replace("{n}", &n.to_string()));
            out.push('\n');
            n += 1;
        }
    }
    out
}

pub(crate) fn sha256_hex(text: &str) -> String {
    ring::digest::digest(&ring::digest::SHA256, text.as_bytes())
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) const KIB: usize = 1024;
pub(crate) const SIZES: [usize; 3] = [10 * KIB, 100 * KIB, 1024 * KIB];

#[cfg(test)]
mod tests {
    use super::*;

    const LANGUAGES: [Language; 5] = [
        Language::Rust,
        Language::CSharp,
        Language::JavaScript,
        Language::TypeScript,
        Language::PlainText,
    ];

    #[test]
    fn generated_fixtures_are_byte_stable() {
        let mut report = String::new();
        for language in LANGUAGES {
            for size in SIZES {
                let text = generate(language, size);
                assert!(text.len() >= size && text.len() < size + 4 * KIB);
                assert!(text.ends_with('\n') && text.is_char_boundary(text.len()));
                assert_eq!(text, generate(language, size));
                report.push_str(&format!(
                    "{} {} {}\n",
                    language.long_name(),
                    size,
                    sha256_hex(&text)
                ));
            }
        }
        let expected = include_str!("../../../tests/fixtures/languages/generated.sha256");
        // A Windows checkout may have converted the file to CRLF.
        assert_eq!(report, expected.replace("\r\n", "\n"));
    }

    fn kinds(source: &str, language: Language) -> Vec<(&str, TokenKind)> {
        Tokens::new(source, language, 0)
            .map(|(range, kind)| (&source[range], kind))
            .collect()
    }

    /// Baseline of KNOWN-WRONG behaviour (design section 2): the lexer runs per
    /// line and carries no state across lines. Replaced by Tree-sitter in P3b.
    #[test]
    fn legacy_lexer_has_no_cross_line_state() {
        let source = "/* first\nsecond line */ let x = 1;";
        let second = &source[source.find('\n').unwrap() + 1..];
        let tokens = kinds(second, Language::JavaScript);
        assert!(
            !tokens.iter().any(|(_, kind)| *kind == TokenKind::Comment),
            "line two is not seen as a comment: {tokens:?}"
        );
    }

    /// Classification here is heuristic (design section 2): the next token decides.
    #[test]
    fn legacy_lexer_classifies_by_neighbouring_text() {
        let tokens = kinds("value(1); Value; x.y", Language::Rust);
        assert!(tokens.contains(&("value", TokenKind::Fn)));
        assert!(tokens.contains(&("Value", TokenKind::Type)));
        assert!(tokens.contains(&("y", TokenKind::Prop)));
    }
}
