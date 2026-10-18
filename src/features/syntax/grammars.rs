//! The one place that names grammar crates. A language description refers to a
//! grammar by directory name (`assets/grammars/<name>`); this table is how that
//! name becomes a native parser. Nothing else may use a `tree_sitter_*` crate.
use tree_sitter::Language;

/// Every grammar with a native parser, sorted. A test keeps this in step with
/// the resource directories and with the grammars the language descriptions use.
pub(crate) const NAMES: &[&str] = &[
    "c_sharp",
    "css",
    "html",
    "javascript",
    "json",
    "markdown",
    "markdown_inline",
    "rust",
    "tsx",
    "typescript",
];

/// Grammars no language description uses: they only parse text embedded in
/// another language (Markdown's inline text).
pub(crate) const EMBEDDED_ONLY: &[&str] = &["markdown_inline"];

pub(crate) fn language(grammar: &str) -> Option<Language> {
    Some(match grammar {
        "c_sharp" => tree_sitter_c_sharp::LANGUAGE.into(),
        "css" => tree_sitter_css::LANGUAGE.into(),
        "html" => tree_sitter_html::LANGUAGE.into(),
        "javascript" => tree_sitter_javascript::LANGUAGE.into(),
        "json" => tree_sitter_json::LANGUAGE.into(),
        "markdown" => tree_sitter_md::LANGUAGE.into(),
        "markdown_inline" => tree_sitter_md::INLINE_LANGUAGE.into(),
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        _ => return None,
    })
}
