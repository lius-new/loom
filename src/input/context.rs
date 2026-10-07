//! Key contexts and the predicate language used by keymap sections.
//!
//! Ported from Zed's `gpui::keymap::context` (Apache-2.0), keeping its
//! semantics: identifiers and `key == value` checks look at a single context
//! node, `!X` holds only when no node on the path satisfies `X`, and `A > B`
//! matches when some ancestor satisfies `A` and the path below it satisfies `B`.

use std::fmt;

/// One node of the context stack, e.g. `Editor extension=rs`.
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct KeyContext(Vec<ContextEntry>);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContextEntry {
    pub key: String,
    pub value: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextError(pub String);

impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ContextError {}

type Result<T> = std::result::Result<T, ContextError>;

fn error<T>(message: impl Into<String>) -> Result<T> {
    Err(ContextError(message.into()))
}

impl KeyContext {
    pub fn new(identifier: &str) -> Self {
        let mut context = Self::default();
        context.add(identifier);
        context
    }

    /// Parse `Editor mode=full extension = rs` style descriptions.
    pub fn parse(source: &str) -> Result<Self> {
        let mut context = Self::default();
        let mut source = skip_whitespace(source);
        while !source.is_empty() {
            let key = take_identifier(source);
            if key.is_empty() {
                return error(format!("unexpected character in context `{source}`"));
            }
            source = skip_whitespace(&source[key.len()..]);
            if let Some(suffix) = source.strip_prefix('=') {
                source = skip_whitespace(suffix);
                let value = take_identifier(source);
                source = skip_whitespace(&source[value.len()..]);
                context.set(key, value);
            } else {
                context.add(key);
            }
        }
        Ok(context)
    }

    /// Add an identifier, unless this context already has that key.
    pub fn add(&mut self, identifier: &str) {
        if !self.contains(identifier) {
            self.0.push(ContextEntry {
                key: identifier.to_owned(),
                value: None,
            });
        }
    }

    /// Set a `key=value` attribute, unless this context already has that key.
    pub fn set(&mut self, key: &str, value: &str) {
        if !self.contains(key) {
            self.0.push(ContextEntry {
                key: key.to_owned(),
                value: Some(value.to_owned()),
            });
        }
    }

    pub fn with(mut self, key: &str, value: &str) -> Self {
        self.set(key, value);
        self
    }

    pub fn contains(&self, key: &str) -> bool {
        self.0.iter().any(|entry| entry.key == key)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|entry| entry.key == key)?
            .value
            .as_deref()
    }

    /// The first identifier without a value, usually the component name.
    pub fn primary(&self) -> Option<&str> {
        self.0
            .iter()
            .find(|entry| entry.value.is_none())
            .map(|entry| entry.key.as_str())
    }
}

impl fmt::Debug for KeyContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for KeyContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, entry) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(" ")?;
            }
            match &entry.value {
                Some(value) => write!(f, "{}={}", entry.key, value)?,
                None => f.write_str(&entry.key)?,
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ContextPredicate {
    Identifier(String),
    Equal(String, String),
    NotEqual(String, String),
    /// `parent > child`: some ancestor matches `parent`, the path below it `child`.
    Descendant(Box<ContextPredicate>, Box<ContextPredicate>),
    Not(Box<ContextPredicate>),
    And(Box<ContextPredicate>, Box<ContextPredicate>),
    Or(Box<ContextPredicate>, Box<ContextPredicate>),
}

const PRECEDENCE_CHILD: u32 = 1;
const PRECEDENCE_OR: u32 = 2;
const PRECEDENCE_AND: u32 = 3;
const PRECEDENCE_EQ: u32 = 4;
const PRECEDENCE_NOT: u32 = 5;

type Constructor = fn(ContextPredicate, ContextPredicate) -> Result<ContextPredicate>;

impl ContextPredicate {
    pub fn parse(source: &str) -> Result<Self> {
        let source = skip_whitespace(source);
        let (predicate, rest) = Self::parse_expr(source, 0)?;
        match rest.chars().next() {
            Some(next) => error(format!("unexpected character `{next}` in context")),
            None => Ok(predicate),
        }
    }

    /// The deepest depth (1-based length of the matching stack prefix) at
    /// which the predicate holds, or `None` when it never does.
    pub fn depth_of(&self, contexts: &[KeyContext]) -> Option<usize> {
        (0..=contexts.len())
            .rev()
            .find(|&depth| self.eval_inner(&contexts[..depth], contexts))
    }

    /// Evaluate against a stack ordered from root to the focused node.
    pub fn eval(&self, contexts: &[KeyContext]) -> bool {
        self.eval_inner(contexts, contexts)
    }

    fn eval_inner(&self, contexts: &[KeyContext], all_contexts: &[KeyContext]) -> bool {
        let Some(context) = contexts.last() else {
            return false;
        };
        match self {
            Self::Identifier(name) => context.contains(name),
            Self::Equal(left, right) => context.get(left) == Some(right.as_str()),
            Self::NotEqual(left, right) => context.get(left).is_none_or(|value| value != right),
            Self::Not(predicate) => (0..all_contexts.len())
                .all(|index| !predicate.eval_inner(&all_contexts[..=index], all_contexts)),
            Self::Descendant(parent, child) => {
                for index in 0..contexts.len() - 1 {
                    if parent.eval_inner(&contexts[..=index], all_contexts) {
                        return child.eval_inner(&contexts[index + 1..], &contexts[index + 1..]);
                    }
                }
                false
            }
            Self::And(left, right) => {
                left.eval_inner(contexts, all_contexts) && right.eval_inner(contexts, all_contexts)
            }
            Self::Or(left, right) => {
                left.eval_inner(contexts, all_contexts) || right.eval_inner(contexts, all_contexts)
            }
        }
    }

    /// Whether this predicate matches every context the other one matches.
    pub fn is_superset(&self, other: &Self) -> bool {
        if self == other {
            return true;
        }
        if let Self::Or(left, right) = self {
            return left.is_superset(other) || right.is_superset(other);
        }
        match other {
            Self::Descendant(_, child) => self.is_superset(child),
            Self::And(left, right) => self.is_superset(left) || self.is_superset(right),
            _ => false,
        }
    }

    fn parse_expr(mut source: &str, min_precedence: u32) -> Result<(Self, &str)> {
        let (mut predicate, rest) = Self::parse_primary(source)?;
        source = rest;
        'parse: loop {
            for (operator, precedence, constructor) in [
                (">", PRECEDENCE_CHILD, Self::new_child as Constructor),
                ("&&", PRECEDENCE_AND, Self::new_and as Constructor),
                ("||", PRECEDENCE_OR, Self::new_or as Constructor),
                ("==", PRECEDENCE_EQ, Self::new_eq as Constructor),
                ("!=", PRECEDENCE_EQ, Self::new_neq as Constructor),
            ] {
                if source.starts_with(operator) && precedence >= min_precedence {
                    source = skip_whitespace(&source[operator.len()..]);
                    let (right, rest) = Self::parse_expr(source, precedence + 1)?;
                    predicate = constructor(predicate, right)?;
                    source = rest;
                    continue 'parse;
                }
            }
            break;
        }
        Ok((predicate, source))
    }

    fn parse_primary(source: &str) -> Result<(Self, &str)> {
        let Some(next) = source.chars().next() else {
            return error("unexpected end of context");
        };
        match next {
            '(' => {
                let (predicate, rest) = Self::parse_expr(skip_whitespace(&source[1..]), 0)?;
                let Some(rest) = rest.strip_prefix(')') else {
                    return error("expected a `)` in context");
                };
                Ok((predicate, skip_whitespace(rest)))
            }
            '!' => {
                let (predicate, rest) =
                    Self::parse_expr(skip_whitespace(&source[1..]), PRECEDENCE_NOT)?;
                Ok((Self::Not(Box::new(predicate)), rest))
            }
            _ if is_identifier_char(next) => {
                let identifier = take_identifier(source);
                Ok((
                    Self::Identifier(identifier.to_owned()),
                    skip_whitespace(&source[identifier.len()..]),
                ))
            }
            _ => error(format!("unexpected character `{next}` in context")),
        }
    }

    fn new_or(self, other: Self) -> Result<Self> {
        Ok(Self::Or(Box::new(self), Box::new(other)))
    }

    fn new_and(self, other: Self) -> Result<Self> {
        Ok(Self::And(Box::new(self), Box::new(other)))
    }

    fn new_child(self, other: Self) -> Result<Self> {
        Ok(Self::Descendant(Box::new(self), Box::new(other)))
    }

    fn new_eq(self, other: Self) -> Result<Self> {
        match (self, other) {
            (Self::Identifier(left), Self::Identifier(right)) => Ok(Self::Equal(left, right)),
            _ => error("operands of == must be identifiers"),
        }
    }

    fn new_neq(self, other: Self) -> Result<Self> {
        match (self, other) {
            (Self::Identifier(left), Self::Identifier(right)) => Ok(Self::NotEqual(left, right)),
            _ => error("operands of != must be identifiers"),
        }
    }
}

impl fmt::Display for ContextPredicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Identifier(name) => f.write_str(name),
            Self::Equal(left, right) => write!(f, "{left} == {right}"),
            Self::NotEqual(left, right) => write!(f, "{left} != {right}"),
            Self::Descendant(parent, child) => write!(f, "{parent} > {child}"),
            Self::Not(predicate) => match predicate.as_ref() {
                Self::Identifier(name) => write!(f, "!{name}"),
                _ => write!(f, "!({predicate})"),
            },
            Self::And(left, right) => {
                write_operand(f, left, |node| matches!(node, Self::Or(..)))?;
                f.write_str(" && ")?;
                write_operand(f, right, |node| matches!(node, Self::Or(..)))
            }
            Self::Or(left, right) => {
                write_operand(f, left, |node| matches!(node, Self::And(..)))?;
                f.write_str(" || ")?;
                write_operand(f, right, |node| matches!(node, Self::And(..)))
            }
        }
    }
}

fn write_operand(
    f: &mut fmt::Formatter<'_>,
    node: &ContextPredicate,
    needs_parens: impl Fn(&ContextPredicate) -> bool,
) -> fmt::Result {
    if needs_parens(node) {
        write!(f, "({node})")
    } else {
        write!(f, "{node}")
    }
}

fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

fn take_identifier(source: &str) -> &str {
    let len = source
        .find(|c: char| !is_identifier_char(c))
        .unwrap_or(source.len());
    &source[..len]
}

fn skip_whitespace(source: &str) -> &str {
    source.trim_start()
}

#[cfg(test)]
mod tests {
    use super::ContextPredicate::*;
    use super::*;

    fn ctx(source: &str) -> KeyContext {
        KeyContext::parse(source).unwrap()
    }

    fn pred(source: &str) -> ContextPredicate {
        ContextPredicate::parse(source).unwrap()
    }

    fn id(name: &str) -> Box<ContextPredicate> {
        Box::new(Identifier(name.into()))
    }

    #[test]
    fn parses_contexts() {
        let mut expected = KeyContext::default();
        expected.add("baz");
        expected.set("foo", "bar");
        assert_eq!(ctx("baz foo=bar"), expected);
        assert_eq!(ctx("baz foo = bar"), expected);
        assert_eq!(ctx("  baz foo   =   bar baz"), expected);
        assert_eq!(ctx("baz foo=bar").primary(), Some("baz"));
        assert_eq!(ctx("Editor extension=rs").get("extension"), Some("rs"));
    }

    #[test]
    fn parses_predicates_with_precedence() {
        assert_eq!(pred("abc12"), Identifier("abc12".into()));
        assert_eq!(pred(" ! ! abc"), Not(Box::new(Not(id("abc")))));
        assert_eq!(pred("a == b"), Equal("a".into(), "b".into()));
        assert_eq!(pred("c!=d"), NotEqual("c".into(), "d".into()));
        assert_eq!(
            ContextPredicate::parse("c == !d").unwrap_err().0,
            "operands of == must be identifiers"
        );
        assert_eq!(
            pred("a || !b && c"),
            Or(id("a"), Box::new(And(Box::new(Not(id("b"))), id("c"))))
        );
        assert_eq!(
            pred("a && (b == c || d != e)"),
            And(
                id("a"),
                Box::new(Or(
                    Box::new(Equal("b".into(), "c".into())),
                    Box::new(NotEqual("d".into(), "e".into())),
                )),
            )
        );
        assert_eq!(pred(" ( a || b ) "), Or(id("a"), id("b")));
        assert!(ContextPredicate::parse("a &&").is_err());
        assert!(ContextPredicate::parse("(a").is_err());
        assert!(ContextPredicate::parse("a $ b").is_err());
    }

    #[test]
    fn display_round_trips() {
        for source in [
            "a",
            "a == b",
            "a != b",
            "a > b",
            "!a",
            "!(a && b)",
            "a && b && c",
            "a || (b && c)",
            "a && (b || c) && d",
            "a && b == c && !(d > e) && f == g",
        ] {
            let parsed = pred(source);
            assert_eq!(parsed.to_string(), source);
            assert_eq!(pred(&parsed.to_string()), parsed);
        }
    }

    #[test]
    fn depth_prefers_the_deepest_match() {
        let stack = [ctx("Workspace"), ctx("Editor mode=full")];
        assert_eq!(pred("Workspace").depth_of(&stack), Some(1));
        assert_eq!(pred("Editor").depth_of(&stack), Some(2));
        assert_eq!(pred("Editor && mode == full").depth_of(&stack), Some(2));
        assert_eq!(pred("Editor && mode == partial").depth_of(&stack), None);
        assert_eq!(pred("Terminal").depth_of(&stack), None);
    }

    #[test]
    fn child_operator_matches_ancestors() {
        let predicate = pred("parent > child");
        assert!(predicate.eval(&[ctx("parent"), ctx("child")]));
        assert!(predicate.eval(&[ctx("grandparent"), ctx("parent"), ctx("child")]));
        assert!(!predicate.eval(&[ctx("other"), ctx("child")]));
        assert!(predicate.eval(&[ctx("parent"), ctx("other"), ctx("child")]));
        assert!(!predicate.eval(&[]));
        assert!(!predicate.eval(&[ctx("child")]));
        assert!(!predicate.eval(&[ctx("parent")]));
        assert!(!pred("child > child").eval(&[ctx("child")]));
        assert!(pred("child > child").eval(&[ctx("child"), ctx("child")]));
    }

    #[test]
    fn not_operator_looks_at_the_whole_path() {
        let not_editor = pred("!editor");
        assert!(not_editor.eval(&[ctx("workspace")]));
        assert!(!not_editor.eval(&[ctx("editor")]));
        assert!(!not_editor.eval(&[ctx("editor"), ctx("workspace")]));
        assert!(!not_editor.eval(&[ctx("workspace"), ctx("editor")]));

        assert!(!pred("!(mode == full)").eval(&[KeyContext::default().with("mode", "full")]));
        assert!(pred("!(mode == full)").eval(&[KeyContext::default().with("mode", "partial")]));

        assert!(pred("!(parent > child)").eval(&[ctx("parent")]));
        assert!(!pred("!(parent > child)").eval(&[ctx("parent"), ctx("child")]));
        assert!(!pred("parent > !child").eval(&[ctx("parent"), ctx("child")]));

        let path = [ctx("Workspace"), ctx("Pane"), ctx("Editor")];
        assert!(!pred("Pane > (Pane > Editor)").eval(&path));
        assert!(pred("Workspace > Pane > Editor").eval(&path));
        assert!(!pred("(Pane > Pane) > Editor").eval(&path));
        assert!(pred("Pane > !Workspace").eval(&[ctx("Pane"), ctx("Editor")]));
        assert!(!pred("Pane > !Workspace").eval(&[ctx("Pane"), ctx("Workspace")]));
        assert!(!pred("!Workspace").eval(&path));
    }

    #[test]
    fn superset_checks() {
        let cases = [
            ("editor", "editor", true),
            ("editor", "workspace", false),
            ("editor", "editor && vim_mode", true),
            ("editor", "mode == full && editor", true),
            ("editor && mode == full", "editor", false),
            ("editor", "something > editor", true),
            ("editor", "editor > menu", false),
            ("foo || bar || baz", "bar", true),
            ("foo || bar || baz", "quux", false),
        ];
        for (a, b, expected) in cases {
            assert_eq!(pred(a).is_superset(&pred(b)), expected, "{a} ⊇ {b}");
        }
    }
}
