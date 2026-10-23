//! Document structure shown in the Outline drawer. The tree is independent of
//! where it came from: language-server symbols or Markdown headings.
pub(crate) mod lsp;
pub(crate) mod markdown;
mod model;

#[cfg(test)]
pub(crate) use self::model::Item;
pub(crate) use self::model::{Kind, Node, Outline, Row};
