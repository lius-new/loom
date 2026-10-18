//! Syntax spans from Tree-sitter trees. No theme, GUI state or worker handles live here.
#[cfg(test)]
pub(crate) mod baseline;
#[cfg(test)]
pub(crate) mod bench_support;
mod grammars;
pub(crate) mod presentation;
#[cfg(test)]
mod presentation_tests;
mod queries;
mod treesitter;
#[cfg(test)]
mod treesitter_bench;
#[cfg(test)]
mod treesitter_tests;
mod window;
pub(crate) use self::treesitter::{TreeIndex, TreeStats};
pub(crate) use self::window::{Freshness, Source, Window, WindowTokens};
use crate::features::highlighting::HighlightId;
use std::ops::Range;

/// A run of text and the highlight class it is drawn with; `None` is plain text.
/// Every source of highlighting produces these, so the display does not care
/// which one made them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TokenSpan {
    pub range: Range<usize>,
    pub class: Option<HighlightId>,
}
