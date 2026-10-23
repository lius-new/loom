//! Shared Markdown documents, projection, layout and controlled reading contracts.
pub mod content;
pub mod html;
pub mod layout;
pub mod model;
pub mod parser;
#[cfg(test)]
mod production_tests;
pub mod reader;
pub mod resources;

/// Whether `path` names a file the Markdown reader handles.
pub fn is_markdown_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            ["md", "markdown", "mdown", "mkd"]
                .iter()
                .any(|ext| value.eq_ignore_ascii_case(ext))
        })
}
