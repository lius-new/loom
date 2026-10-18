//! Sparse styles keyed by class, resolved with class fallback and modifier
//! rules. Colors are plain `0xRRGGBB`; the theme converts them for drawing.
use super::{HighlightId, HighlightModifiers};

/// Unset attributes inherit; an explicit `false` clears an inherited `true`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HighlightStyle {
    pub fg: Option<u32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
}

#[allow(dead_code)] // Font attributes are drawn once layout measures them (design 7.4).
impl HighlightStyle {
    pub const NONE: Self = Self {
        fg: None,
        bold: None,
        italic: None,
        underline: None,
        strikethrough: None,
    };

    pub const fn fg(rgb: u32) -> Self {
        Self {
            fg: Some(rgb),
            ..Self::NONE
        }
    }

    /// `top` wins wherever it sets an attribute.
    pub const fn overlay(self, top: Self) -> Self {
        Self {
            fg: pick(self.fg, top.fg),
            bold: pick(self.bold, top.bold),
            italic: pick(self.italic, top.italic),
            underline: pick(self.underline, top.underline),
            strikethrough: pick(self.strikethrough, top.strikethrough),
        }
    }
}

const fn pick<T: Copy>(base: Option<T>, top: Option<T>) -> Option<T> {
    match top {
        Some(_) => top,
        None => base,
    }
}

/// A theme's syntax styles: per-class entries plus modifier rules.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StyleTable {
    pub classes: &'static [(HighlightId, HighlightStyle)],
    /// Applied in order after the class style, so a later rule wins a conflict.
    pub modifiers: &'static [(HighlightModifiers, HighlightStyle)],
}

impl StyleTable {
    /// The most specific class entry wins; attributes it leaves unset come from
    /// its ancestors. Modifier rules whose flags are all present apply last.
    pub fn resolve(&self, class: HighlightId, modifiers: HighlightModifiers) -> HighlightStyle {
        let mut chain = [None; HighlightId::COUNT];
        let mut length = 0;
        let mut next = Some(class);
        while let Some(id) = next {
            chain[length] = Some(id);
            length += 1;
            next = id.parent();
        }
        let mut style = HighlightStyle::NONE;
        for id in chain[..length].iter().rev().flatten() {
            if let Some((_, entry)) = self.classes.iter().find(|(known, _)| known == id) {
                style = style.overlay(*entry);
            }
        }
        for (flags, rule) in self.modifiers {
            if modifiers.contains(*flags) {
                style = style.overlay(*rule);
            }
        }
        style
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: StyleTable = StyleTable {
        classes: &[
            (
                HighlightId::FUNCTION,
                HighlightStyle {
                    fg: Some(0x111111),
                    bold: Some(true),
                    ..HighlightStyle::NONE
                },
            ),
            (HighlightId::FUNCTION_METHOD, HighlightStyle::fg(0x222222)),
            (
                HighlightId::COMMENT,
                HighlightStyle {
                    fg: Some(0x333333),
                    italic: Some(true),
                    ..HighlightStyle::NONE
                },
            ),
            (
                HighlightId::COMMENT_DOCUMENTATION,
                HighlightStyle {
                    italic: Some(false),
                    ..HighlightStyle::NONE
                },
            ),
        ],
        modifiers: &[
            (HighlightModifiers::READONLY, HighlightStyle::fg(0x444444)),
            (HighlightModifiers::STATIC, HighlightStyle::fg(0x555555)),
            (
                HighlightModifiers::DEPRECATED,
                HighlightStyle {
                    strikethrough: Some(true),
                    ..HighlightStyle::NONE
                },
            ),
        ],
    };

    #[test]
    fn a_missing_class_falls_back_to_its_ancestor() {
        let none = HighlightModifiers::NONE;
        assert_eq!(
            TABLE.resolve(HighlightId::FUNCTION, none).fg,
            Some(0x111111)
        );
        assert_eq!(
            TABLE.resolve(HighlightId::KEYWORD, none),
            HighlightStyle::NONE
        );
        assert_eq!(
            TABLE.resolve(HighlightId::STRING_REGEX, none),
            HighlightStyle::NONE,
            "no entry for string or string.regex"
        );
    }

    #[test]
    fn unset_attributes_inherit_and_an_explicit_false_clears() {
        let none = HighlightModifiers::NONE;
        let method = TABLE.resolve(HighlightId::FUNCTION_METHOD, none);
        assert_eq!(method.fg, Some(0x222222), "own color");
        assert_eq!(method.bold, Some(true), "inherited from function");
        let documentation = TABLE.resolve(HighlightId::COMMENT_DOCUMENTATION, none);
        assert_eq!(documentation.fg, Some(0x333333));
        assert_eq!(documentation.italic, Some(false), "false is not 'unset'");
    }

    #[test]
    fn modifier_rules_apply_in_table_order() {
        let both = HighlightModifiers::READONLY.union(HighlightModifiers::STATIC);
        assert_eq!(
            TABLE.resolve(HighlightId::FUNCTION, both).fg,
            Some(0x555555),
            "the later rule wins"
        );
        let all = both.union(HighlightModifiers::DEPRECATED);
        let style = TABLE.resolve(HighlightId::FUNCTION, all);
        assert_eq!(style.fg, Some(0x555555));
        assert_eq!(style.strikethrough, Some(true));
        assert_eq!(style.bold, Some(true), "rules only override what they set");
        assert_eq!(
            TABLE
                .resolve(HighlightId::FUNCTION, HighlightModifiers::ASYNC)
                .fg,
            Some(0x111111),
            "a flag without a rule changes nothing"
        );
    }
}
