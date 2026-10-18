use crate::domain::language::ResolvedLanguage;
use std::ops::Range;
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokenKind {
    Kw,
    Fn,
    Type,
    Str,
    Comment,
    Num,
    Prop,
    Plain,
}

/// Which keyword table applies. The lexer keeps its own table because it is
/// replaced in P3b; nothing here is part of a language description.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dialect {
    CSharp,
    Rust,
    Script,
    Plain,
}

impl Dialect {
    fn of(language: ResolvedLanguage) -> Self {
        match language.name() {
            "csharp" => Self::CSharp,
            "rust" => Self::Rust,
            "typescript" | "javascript" => Self::Script,
            _ => Self::Plain,
        }
    }
}

pub(crate) struct Tokens<'a> {
    source: &'a str,
    dialect: Dialect,
    offset: usize,
    cancel: Option<&'a std::sync::atomic::AtomicBool>,
    checked: usize,
    stopped: bool,
}
impl<'a> Tokens<'a> {
    pub(crate) fn new(source: &'a str, lang: ResolvedLanguage, offset: usize) -> Self {
        Self {
            source,
            dialect: Dialect::of(lang),
            offset,
            cancel: None,
            checked: 0,
            stopped: false,
        }
    }
    pub(crate) fn cancellable(mut self, cancel: &'a std::sync::atomic::AtomicBool) -> Self {
        self.cancel = Some(cancel);
        self
    }
    fn peek(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }
    fn advance(&mut self) {
        if let Some(c) = self.peek() {
            self.offset += c.len_utf8();
            self.checked += c.len_utf8();
            if self.checked >= 4096 {
                self.checked = 0;
                if self
                    .cancel
                    .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
                {
                    self.stopped = true;
                    self.offset = self.source.len();
                }
            }
        }
    }
    fn consume(&mut self, predicate: impl Fn(char) -> bool) {
        while self.peek().is_some_and(&predicate) {
            self.advance();
        }
    }
}
impl Iterator for Tokens<'_> {
    type Item = (Range<usize>, TokenKind);
    fn next(&mut self) -> Option<Self::Item> {
        let start = self.offset;
        let c = self.peek()?;
        let rest = &self.source[start..];
        let kind = if rest.starts_with("//") {
            self.offset = self.source.len();
            TokenKind::Comment
        } else if let Some(comment) = rest.strip_prefix("/*") {
            self.offset = comment
                .find("*/")
                .map_or(self.source.len(), |end| start + end + 4);
            TokenKind::Comment
        } else if c == '"' || c == '\'' || c == '`' || rest.starts_with("$\"") {
            let quote = if rest.starts_with("$\"") {
                self.advance();
                '"'
            } else {
                c
            };
            self.advance();
            while let Some(next) = self.peek() {
                self.advance();
                if next == '\\' {
                    self.advance();
                } else if next == quote {
                    break;
                }
            }
            TokenKind::Str
        } else if c.is_ascii_digit() {
            self.consume(|c| c.is_ascii_digit() || c == '.');
            TokenKind::Num
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            self.consume(|c| c.is_alphanumeric() || c == '_' || c == '$');
            let text = &self.source[start..self.offset];
            let before = self.source[..start].trim_end();
            let prev = before
                .rsplit(|c: char| !c.is_alphanumeric() && c != '_' && c != '$')
                .next()
                .unwrap_or("");
            if is_keyword(text, self.dialect) {
                TokenKind::Kw
            } else if is_builtin_type(text, self.dialect) || prev == "new" {
                TokenKind::Type
            } else if self.peek() == Some('(') {
                TokenKind::Fn
            } else if c.is_uppercase() {
                TokenKind::Type
            } else if c == '_' || before.ends_with('.') {
                TokenKind::Prop
            } else {
                TokenKind::Plain
            }
        } else {
            // Consume at least one character, including non-ASCII numerals.
            self.advance();
            while let Some(next) = self.peek() {
                let rest = &self.source[self.offset..];
                if next.is_ascii_digit()
                    || next.is_alphabetic()
                    || matches!(next, '_' | '$' | '"' | '\'' | '`')
                    || rest.starts_with("//")
                    || rest.starts_with("/*")
                {
                    break;
                }
                self.advance();
            }
            TokenKind::Plain
        };
        (!self.stopped).then_some((start..self.offset, kind))
    }
}
fn is_keyword(text: &str, dialect: Dialect) -> bool {
    let list: &[&str] = match dialect {
        Dialect::CSharp => &[
            "namespace",
            "public",
            "sealed",
            "class",
            "private",
            "readonly",
            "async",
            "if",
            "throw",
            "new",
            "return",
            "await",
            "using",
            "static",
            "var",
            "true",
            "false",
            "nameof",
            "this",
            "else",
            "null",
            "for",
            "foreach",
            "while",
            "get",
            "set",
            "override",
            "virtual",
            "out",
            "ref",
            "in",
            "params",
            "is",
            "as",
            "internal",
            "protected",
            "interface",
            "struct",
            "enum",
        ],
        Dialect::Rust => &[
            "pub", "struct", "impl", "fn", "self", "let", "mut", "use", "mod", "if", "else",
            "match", "for", "while", "loop", "return", "true", "false", "where", "type", "trait",
            "enum", "const", "static", "ref", "async", "await", "move", "in", "as", "box", "dyn",
            "unsafe", "extern", "crate", "super",
        ],
        Dialect::Script => &[
            "import",
            "from",
            "export",
            "async",
            "function",
            "return",
            "await",
            "new",
            "const",
            "let",
            "var",
            "if",
            "else",
            "for",
            "while",
            "throw",
            "try",
            "catch",
            "typeof",
            "interface",
            "type",
            "class",
            "extends",
            "implements",
            "public",
            "private",
            "readonly",
            "static",
            "true",
            "false",
            "null",
            "undefined",
            "this",
            "in",
            "of",
            "as",
        ],
        Dialect::Plain => &[],
    };
    list.contains(&text)
}

fn is_builtin_type(text: &str, dialect: Dialect) -> bool {
    let list: &[&str] = match dialect {
        Dialect::CSharp => &[
            "string", "decimal", "bool", "void", "int", "long", "double", "float", "byte", "char",
            "object", "dynamic",
        ],
        Dialect::Rust => &[
            "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64", "isize", "f32", "f64",
            "bool", "str", "char",
        ],
        Dialect::Script => &[
            "string", "number", "boolean", "any", "void", "unknown", "never", "object", "symbol",
            "bigint",
        ],
        Dialect::Plain => &[],
    };
    list.contains(&text)
}
