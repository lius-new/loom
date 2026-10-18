//! The class table every highlight source maps into. A class is a compact id;
//! its dotted name (`function.method`) is the stable form used by themes and
//! settings. Ids index `CLASS_NAMES` and are never persisted.

/// Canonical class names. The order fixes the ids below; a test keeps the two
/// in step.
const CLASS_NAMES: &[&str] = &[
    "keyword",
    "function",
    "function.method",
    "type",
    "type.builtin",
    "variable",
    "variable.parameter",
    "property",
    "constant",
    "string",
    "string.escape",
    "string.regex",
    "comment",
    "comment.documentation",
    "number",
    "operator",
    "punctuation",
    "tag",
    "attribute",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct HighlightId(u8);

#[allow(dead_code)] // The classes without a consumer yet are used from P3 on.
impl HighlightId {
    pub const KEYWORD: Self = Self(0);
    pub const FUNCTION: Self = Self(1);
    pub const FUNCTION_METHOD: Self = Self(2);
    pub const TYPE: Self = Self(3);
    pub const TYPE_BUILTIN: Self = Self(4);
    pub const VARIABLE: Self = Self(5);
    pub const VARIABLE_PARAMETER: Self = Self(6);
    pub const PROPERTY: Self = Self(7);
    pub const CONSTANT: Self = Self(8);
    pub const STRING: Self = Self(9);
    pub const STRING_ESCAPE: Self = Self(10);
    pub const STRING_REGEX: Self = Self(11);
    pub const COMMENT: Self = Self(12);
    pub const COMMENT_DOCUMENTATION: Self = Self(13);
    pub const NUMBER: Self = Self(14);
    pub const OPERATOR: Self = Self(15);
    pub const PUNCTUATION: Self = Self(16);
    pub const TAG: Self = Self(17);
    pub const ATTRIBUTE: Self = Self(18);

    pub const COUNT: usize = CLASS_NAMES.len();

    pub fn name(self) -> &'static str {
        CLASS_NAMES[self.0 as usize]
    }

    /// The class for `name`, or for its nearest known ancestor: `keyword.control`
    /// is a `keyword`. Names with no known ancestor have no class.
    pub fn from_name(name: &str) -> Option<Self> {
        let mut name = name;
        loop {
            if let Some(index) = CLASS_NAMES.iter().position(|known| *known == name) {
                return Some(Self(index as u8));
            }
            name = name.rsplit_once('.')?.0;
        }
    }

    /// The next more general class: `function.method` falls back to `function`.
    pub fn parent(self) -> Option<Self> {
        Self::from_name(self.name().rsplit_once('.')?.0)
    }
}

/// Flags refining a class, such as `declaration` or `readonly`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct HighlightModifiers(u8);

#[allow(dead_code)] // Modifier sources arrive with Tree-sitter and LSP.
impl HighlightModifiers {
    pub const NONE: Self = Self(0);
    pub const DECLARATION: Self = Self(1 << 0);
    pub const DEFINITION: Self = Self(1 << 1);
    pub const READONLY: Self = Self(1 << 2);
    pub const STATIC: Self = Self(1 << 3);
    pub const DEPRECATED: Self = Self(1 << 4);
    pub const ASYNC: Self = Self(1 << 5);
    pub const DOCUMENTATION: Self = Self(1 << 6);
    pub const DEFAULT_LIBRARY: Self = Self(1 << 7);

    /// Names as the language server protocol spells them.
    const NAMES: [(&'static str, Self); 8] = [
        ("declaration", Self::DECLARATION),
        ("definition", Self::DEFINITION),
        ("readonly", Self::READONLY),
        ("static", Self::STATIC),
        ("deprecated", Self::DEPRECATED),
        ("async", Self::ASYNC),
        ("documentation", Self::DOCUMENTATION),
        ("defaultLibrary", Self::DEFAULT_LIBRARY),
    ];

    pub fn from_name(name: &str) -> Option<Self> {
        Self::NAMES
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, flag)| *flag)
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether every flag of `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_the_name_table() {
        let expected = [
            (HighlightId::KEYWORD, "keyword"),
            (HighlightId::FUNCTION, "function"),
            (HighlightId::FUNCTION_METHOD, "function.method"),
            (HighlightId::TYPE, "type"),
            (HighlightId::TYPE_BUILTIN, "type.builtin"),
            (HighlightId::VARIABLE, "variable"),
            (HighlightId::VARIABLE_PARAMETER, "variable.parameter"),
            (HighlightId::PROPERTY, "property"),
            (HighlightId::CONSTANT, "constant"),
            (HighlightId::STRING, "string"),
            (HighlightId::STRING_ESCAPE, "string.escape"),
            (HighlightId::STRING_REGEX, "string.regex"),
            (HighlightId::COMMENT, "comment"),
            (HighlightId::COMMENT_DOCUMENTATION, "comment.documentation"),
            (HighlightId::NUMBER, "number"),
            (HighlightId::OPERATOR, "operator"),
            (HighlightId::PUNCTUATION, "punctuation"),
            (HighlightId::TAG, "tag"),
            (HighlightId::ATTRIBUTE, "attribute"),
        ];
        assert_eq!(expected.len(), HighlightId::COUNT);
        for (id, name) in expected {
            assert_eq!(id.name(), name);
            assert_eq!(HighlightId::from_name(name), Some(id));
        }
    }

    #[test]
    fn unknown_names_fall_back_to_the_nearest_known_ancestor() {
        let id = HighlightId::from_name;
        assert_eq!(id("keyword.control.import"), Some(HighlightId::KEYWORD));
        assert_eq!(
            id("function.method.call"),
            Some(HighlightId::FUNCTION_METHOD)
        );
        assert_eq!(id("function.macro"), Some(HighlightId::FUNCTION));
        assert_eq!(id("type.builtin"), Some(HighlightId::TYPE_BUILTIN));
        assert_eq!(id("nonsense"), None);
        assert_eq!(id("nonsense.keyword"), None, "only a prefix is an ancestor");
        assert_eq!(id(""), None);
    }

    #[test]
    fn parents_walk_toward_the_root_class() {
        assert_eq!(
            HighlightId::FUNCTION_METHOD.parent(),
            Some(HighlightId::FUNCTION)
        );
        assert_eq!(
            HighlightId::COMMENT_DOCUMENTATION.parent(),
            Some(HighlightId::COMMENT)
        );
        assert_eq!(HighlightId::KEYWORD.parent(), None);
    }

    #[test]
    fn modifiers_combine_and_parse_lsp_names() {
        let both = HighlightModifiers::READONLY.union(HighlightModifiers::STATIC);
        assert!(both.contains(HighlightModifiers::READONLY));
        assert!(both.contains(both));
        assert!(!both.contains(HighlightModifiers::ASYNC));
        assert!(HighlightModifiers::NONE.contains(HighlightModifiers::NONE));
        assert_eq!(
            HighlightModifiers::from_name("defaultLibrary"),
            Some(HighlightModifiers::DEFAULT_LIBRARY)
        );
        assert_eq!(HighlightModifiers::from_name("unknown"), None);
    }
}
