//! Deterministic source samples for tests and measurements. The hashes pin the
//! bytes so later runs measure the same input (design sections 13-14).

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

/// Sample lines for a built-in language by canonical name.
pub(crate) fn template(language: &str) -> &'static [&'static str] {
    match language {
        "rust" => RUST,
        "csharp" => CSHARP,
        "javascript" => JAVASCRIPT,
        "typescript" => TYPESCRIPT,
        "plaintext" => &["Label {n} 中é \"value\" 456 /* note */"],
        other => panic!("no baseline sample for `{other}`"),
    }
}

/// Whole lines only, so every fixture ends on a line boundary and is >= `bytes`.
pub(crate) fn generate(language: &str, bytes: usize) -> String {
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
    use crate::domain::language::builtin;

    const LANGUAGES: [&str; 5] = ["rust", "csharp", "javascript", "typescript", "plaintext"];

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
                    builtin(language).display_name(),
                    size,
                    sha256_hex(&text)
                ));
            }
        }
        let expected = include_str!("../../../tests/fixtures/languages/generated.sha256");
        // A Windows checkout may have converted the file to CRLF.
        assert_eq!(report, expected.replace("\r\n", "\n"));
    }
}
