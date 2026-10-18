//! Incremental semantic spans. No theme, GUI state or worker handles live here.
#[cfg(test)]
mod baseline;
mod index;
mod lexer;
#[cfg(test)]
mod tests;
pub(crate) use self::index::{Index, IndexStats, Source, Window, WindowTokens};
pub(crate) use self::lexer::{TokenKind, Tokens};
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TokenSpan {
    pub range: Range<usize>,
    pub kind: TokenKind,
}
