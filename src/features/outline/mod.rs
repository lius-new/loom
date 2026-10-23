//! Document structure shown in the Outline drawer. The tree is independent of
//! where it came from: language-server symbols or Markdown headings.
pub(crate) mod lsp;
pub(crate) mod markdown;
mod model;

#[cfg(test)]
pub(crate) use self::model::Item;
pub(crate) use self::model::{Kind, Node, Outline, Row};

/// Whether the built-in Markdown reader should produce the outline for `path`.
pub(crate) fn is_markdown(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            ["md", "markdown", "mdown", "mkd"]
                .iter()
                .any(|ext| value.eq_ignore_ascii_case(ext))
        })
}
