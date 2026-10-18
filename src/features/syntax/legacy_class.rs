//! Temporary bridge from the hand-written lexer's token kinds to shared highlight
//! classes. It exists only while the lexer does; P3b deletes both together.
use super::TokenKind;
use crate::features::highlighting::HighlightId;

impl TokenKind {
    /// `None` is plain text, drawn in the normal text color.
    pub(crate) fn highlight(self) -> Option<HighlightId> {
        match self {
            TokenKind::Kw => Some(HighlightId::KEYWORD),
            TokenKind::Fn => Some(HighlightId::FUNCTION),
            TokenKind::Type => Some(HighlightId::TYPE),
            TokenKind::Str => Some(HighlightId::STRING),
            TokenKind::Comment => Some(HighlightId::COMMENT),
            TokenKind::Num => Some(HighlightId::NUMBER),
            TokenKind::Prop => Some(HighlightId::PROPERTY),
            TokenKind::Plain => None,
        }
    }
}
