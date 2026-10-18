//! Incremental semantic spans. No theme, GUI state or worker handles live here.
#[cfg(test)]
pub(crate) mod baseline;
#[cfg(test)]
pub(crate) mod bench_support;
mod grammars;
mod index;
mod legacy_class;
mod lexer;
mod presentation;
#[cfg(test)]
mod presentation_tests;
mod queries;
#[cfg(test)]
mod tests;
mod treesitter;
#[cfg(test)]
mod treesitter_bench;
#[cfg(test)]
mod treesitter_tests;
pub(crate) use self::index::{Index, IndexStats, Source, Window, WindowTokens};
pub(crate) use self::lexer::{TokenKind, Tokens};
pub(crate) use self::treesitter::{TreeIndex, TreeStats};
use crate::features::highlighting::HighlightId;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
/// A run of text and the highlight class it is drawn with; `None` is plain text.
/// Every source of highlighting produces these, so the display does not care
/// which one made them.
pub(crate) struct TokenSpan {
    pub range: Range<usize>,
    pub class: Option<HighlightId>,
}
