//! Bounded snippet substitutions. No shell evaluation or runtime process is
//! involved; a malformed or over-budget transform preserves its input.
use fancy_regex::{Captures, Regex, RegexBuilder};
use std::iter::Peekable;
use std::str::Chars;
use std::sync::Arc;

const MAX_INPUT: usize = 64 * 1024;
const MAX_OUTPUT: usize = 256 * 1024;
const MAX_MATCHES: usize = 256;

#[derive(Clone, Debug)]
pub struct Transform {
    regex: Option<Arc<Regex>>,
    pattern: String,
    replacement: Vec<Part>,
    options: String,
    pub indent: String,
}

impl PartialEq for Transform {
    fn eq(&self, other: &Self) -> bool {
        self.pattern == other.pattern
            && self.replacement == other.replacement
            && self.options == other.options
            && self.indent == other.indent
    }
}
impl Eq for Transform {}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    Text(String),
    Capture(usize),
    Case(usize, String),
    Conditional {
        index: usize,
        yes: Option<Vec<Part>>,
        no: Option<Vec<Part>>,
    },
}

impl Transform {
    /// Reads the regex/replacement/options after the opening slash, including
    /// the closing brace. A slash inside a format expression is not a delimiter.
    pub(super) fn parse(chars: &mut Peekable<Chars<'_>>) -> Option<Self> {
        let mut pattern = String::new();
        loop {
            match chars.next()? {
                '/' => break,
                '\\' => {
                    let c = chars.next()?;
                    if c != '/' {
                        pattern.push('\\');
                    }
                    pattern.push(c);
                }
                c => pattern.push(c),
            }
        }
        let mut replacement = String::new();
        let mut depth = 0usize;
        loop {
            match chars.next()? {
                '/' if depth == 0 => break,
                '\\' => {
                    replacement.push('\\');
                    replacement.push(chars.next()?);
                }
                '$' if chars.peek() == Some(&'{') => {
                    chars.next();
                    replacement.push_str("${");
                    depth += 1;
                }
                '}' if depth > 0 => {
                    replacement.push('}');
                    depth -= 1;
                }
                '}' => return None,
                c => replacement.push(c),
            }
        }
        let mut options = String::new();
        loop {
            match chars.next()? {
                '}' => break,
                c if c.is_ascii_alphabetic() => options.push(c),
                _ => return None,
            }
        }
        Some(Self::new(&pattern, &replacement, &options))
    }

    fn new(pattern: &str, replacement: &str, options: &str) -> Self {
        let replacement = (replacement.len() <= 16 * 1024)
            .then(|| Format::parse(replacement))
            .flatten();
        let mut used = std::collections::BTreeSet::new();
        let valid = pattern.len() <= 4096
            && replacement.is_some()
            && options
                .chars()
                .all(|c| "gimsuyd".contains(c) && used.insert(c));
        let regex = valid
            .then(|| {
                RegexBuilder::new(pattern)
                    .case_insensitive(options.contains('i'))
                    .multi_line(options.contains('m'))
                    .dot_matches_new_line(options.contains('s'))
                    .backtrack_limit(10_000)
                    .delegate_size_limit(256 * 1024)
                    .delegate_dfa_size_limit(256 * 1024)
                    .build()
                    .ok()
                    .map(Arc::new)
            })
            .flatten();
        Self {
            regex,
            pattern: pattern.into(),
            replacement: replacement.unwrap_or_default(),
            options: options.into(),
            indent: String::new(),
        }
    }

    pub fn apply(&self, value: &str) -> String {
        self.apply_checked(value)
            .unwrap_or_else(|| value.to_owned())
    }

    fn apply_checked(&self, value: &str) -> Option<String> {
        if value.len() > MAX_INPUT {
            return None;
        }
        let regex = self.regex.as_ref()?;
        // The editor has already indented the original placeholder's lines.
        let unindented = if self.indent.is_empty() {
            value.to_owned()
        } else {
            value.replace(&format!("\n{}", self.indent), "\n")
        };
        let input = unindented.as_str();
        let mut out = String::new();
        let mut end = 0;
        let mut expected = 0;
        let mut count = 0;
        for found in regex.captures_iter(input) {
            let captures = found.ok()?;
            let matched = captures.get(0)?;
            if self.options.contains('y') && matched.start() != expected {
                break;
            }
            count += 1;
            if count > MAX_MATCHES {
                return None;
            }
            push(&mut out, &input[end..matched.start()])?;
            render(&self.replacement, Some(&captures), &mut out)?;
            end = matched.end();
            expected = if matched.start() == matched.end() {
                end + input[end..].chars().next().map_or(0, char::len_utf8)
            } else {
                end
            };
            if !self.options.contains('g') {
                break;
            }
        }
        if count == 0
            && self
                .replacement
                .iter()
                .any(|p| matches!(p, Part::Conditional { no: Some(_), .. }))
        {
            render(&self.replacement, None, &mut out)?;
        } else {
            push(&mut out, &input[end..])?;
        }
        let extra = out
            .bytes()
            .filter(|c| *c == b'\n')
            .count()
            .checked_mul(self.indent.len())?;
        if out.len().checked_add(extra)? > MAX_OUTPUT {
            return None;
        }
        Some(if self.indent.is_empty() {
            out
        } else {
            out.replace('\n', &format!("\n{}", self.indent))
        })
    }
}

fn push(out: &mut String, value: &str) -> Option<()> {
    if out.len().checked_add(value.len())? > MAX_OUTPUT {
        return None;
    }
    out.push_str(value);
    Some(())
}

fn render(parts: &[Part], captures: Option<&Captures<'_, str>>, out: &mut String) -> Option<()> {
    for part in parts {
        let capture = |index| {
            captures
                .and_then(|c| c.get(index))
                .map_or("", |m| m.as_str())
        };
        match part {
            Part::Text(text) => push(out, text)?,
            Part::Capture(index) => push(out, capture(*index))?,
            Part::Case(index, kind) => push(out, &case(capture(*index), kind))?,
            Part::Conditional { index, yes, no } => {
                if !capture(*index).is_empty() {
                    if let Some(parts) = yes {
                        render(parts, captures, out)?;
                    } else {
                        push(out, capture(*index))?;
                    }
                } else if let Some(parts) = no {
                    render(parts, captures, out)?;
                }
            }
        }
    }
    Some(())
}

struct Format<'a> {
    chars: Peekable<Chars<'a>>,
    depth: usize,
    remaining: usize,
    exceeded: bool,
}
impl Format<'_> {
    fn parse(source: &str) -> Option<Vec<Part>> {
        let mut parser = Format {
            chars: source.chars().peekable(),
            depth: 0,
            remaining: 1024,
            exceeded: false,
        };
        let parts = parser.parts(&[])?;
        (!parser.exceeded).then_some(parts)
    }
    fn parts(&mut self, ends: &[char]) -> Option<Vec<Part>> {
        if self.depth >= 32 {
            self.exceeded = true;
            return None;
        }
        self.depth += 1;
        let mut parts = Vec::new();
        let mut text = String::new();
        while let Some(&c) = self.chars.peek() {
            if ends.contains(&c) {
                break;
            }
            if self.remaining == 0 {
                self.exceeded = true;
                return None;
            }
            self.remaining -= 1;
            self.chars.next();
            if c == '\\' {
                if let Some(&next) = self.chars.peek()
                    && "$}\\/:".contains(next)
                {
                    self.chars.next();
                    text.push(next);
                } else {
                    text.push(c);
                }
            } else if c == '$' {
                let saved = self.chars.clone();
                let depth = self.depth;
                if let Some(part) = self.capture() {
                    if !text.is_empty() {
                        parts.push(Part::Text(std::mem::take(&mut text)));
                    }
                    parts.push(part);
                } else {
                    self.chars = saved;
                    self.depth = depth;
                    text.push('$');
                }
            } else {
                text.push(c);
            }
        }
        if !text.is_empty() {
            parts.push(Part::Text(text));
        }
        self.depth -= 1;
        Some(parts)
    }
    fn capture(&mut self) -> Option<Part> {
        let braced = self.chars.peek() == Some(&'{');
        if braced {
            self.chars.next();
        }
        let mut number = String::new();
        while let Some(&c) = self.chars.peek() {
            if !c.is_ascii_digit() {
                break;
            }
            number.push(c);
            self.chars.next();
        }
        let index = number.parse::<usize>().ok()?;
        if !braced {
            return Some(Part::Capture(index));
        }
        match self.chars.next()? {
            '}' => Some(Part::Capture(index)),
            ':' => {
                if self.chars.peek() == Some(&'/') {
                    self.chars.next();
                    let mut kind = String::new();
                    loop {
                        let c = self.chars.next()?;
                        if c == '}' {
                            break;
                        }
                        kind.push(c);
                    }
                    return Some(Part::Case(index, kind));
                }
                let (yes, no) = match self.chars.peek() {
                    Some('+') => {
                        self.chars.next();
                        (Some(self.parts(&['}'])?), None)
                    }
                    Some('?') => {
                        self.chars.next();
                        let yes = self.parts(&[':', '}'])?;
                        if self.chars.next()? != ':' {
                            return None;
                        }
                        (Some(yes), Some(self.parts(&['}'])?))
                    }
                    Some('-') => {
                        self.chars.next();
                        (None, Some(self.parts(&['}'])?))
                    }
                    _ => (None, Some(self.parts(&['}'])?)),
                };
                if self.chars.next()? != '}' {
                    return None;
                }
                Some(Part::Conditional { index, yes, no })
            }
            _ => None,
        }
    }
}

fn case(value: &str, kind: &str) -> String {
    fn capitalize(value: &str) -> String {
        let mut chars = value.chars();
        chars
            .next()
            .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
    }
    match kind {
        "upcase" => return value.to_uppercase(),
        "downcase" => return value.to_lowercase(),
        "capitalize" => return capitalize(value),
        "camelcase" | "pascalcase" | "snakecase" | "kebabcase" => {}
        _ => return value.into(),
    }
    let chars: Vec<char> = value.chars().collect();
    let mut words = Vec::new();
    let mut word = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let boundary = !c.is_alphanumeric()
            || c.is_uppercase()
                && i > 0
                && (chars[i - 1].is_lowercase()
                    || chars[i - 1].is_numeric()
                    || chars[i - 1].is_uppercase()
                        && chars.get(i + 1).is_some_and(|c| c.is_lowercase()));
        if boundary && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        if c.is_alphanumeric() {
            word.push(c);
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    match kind {
        "snakecase" => words
            .iter()
            .map(|w| w.to_lowercase())
            .collect::<Vec<_>>()
            .join("_"),
        "kebabcase" => words
            .iter()
            .map(|w| w.to_lowercase())
            .collect::<Vec<_>>()
            .join("-"),
        _ => words
            .iter()
            .enumerate()
            .map(|(i, word)| {
                let lower = word.to_lowercase();
                if kind == "camelcase" && i == 0 {
                    lower
                } else {
                    capitalize(&lower)
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_global_case_and_sticky_options_keep_unmatched_text() {
        assert_eq!(Transform::new("(x)", "[$0:$1]", "").apply("x_x"), "[x:x]_x");
        assert_eq!(Transform::new("(x)", "[$1]", "gi").apply("x_X"), "[x]_[X]");
        assert_eq!(Transform::new("a", "A", "gy").apply("aa baa"), "AA baa");
        assert_eq!(Transform::new("a", "A", "y").apply("ba"), "ba");
        assert_eq!(Transform::new("^a.$", "ok", "ms").apply("a\n"), "ok");
    }

    #[test]
    fn lookarounds_and_backreferences_are_supported() {
        assert_eq!(
            Transform::new("(?<=foo)(bar)(?=!)", "$1$1", "").apply("foobar!"),
            "foobarbar!"
        );
        assert_eq!(
            Transform::new(r"(\w+)-\1", "${1:/upcase}", "").apply("go-go"),
            "GO"
        );
    }

    #[test]
    fn all_case_formats_handle_words_acronyms_and_unicode() {
        for (kind, value, expected) in [
            ("upcase", "éß", "ÉSS"),
            ("downcase", "ÉSS", "éss"),
            ("capitalize", "éWord", "ÉWord"),
            ("camelcase", "HTTP_server-name", "httpServerName"),
            ("pascalcase", "my_var-name", "MyVarName"),
            ("snakecase", "HTTPServerName", "http_server_name"),
            ("kebabcase", "aName_名字", "a-name-名字"),
        ] {
            assert_eq!(
                Transform::new("(.*)", &format!("${{1:/{kind}}}"), "").apply(value),
                expected
            );
        }
    }

    #[test]
    fn conditional_formats_distinguish_empty_captures_and_handle_no_match() {
        let format = "${1:+yes}|${1:?then:else}|${1:-fallback}|${1:default}";
        let t = Transform::new("^(a)?$", format, "");
        assert_eq!(t.apply("a"), "yes|then|a|a");
        assert_eq!(t.apply(""), "|else|fallback|default");
        assert_eq!(
            Transform::new("never", "${1:-fallback}", "").apply("input"),
            "fallback"
        );
        assert_eq!(
            Transform::new("(a)?(b)", "${1:?${2:/upcase}:none}", "").apply("ab"),
            "B"
        );
        assert_eq!(
            Transform::new("(a)?(b)", "${1:?${2:/upcase}:none}", "").apply("b"),
            "none"
        );
    }

    #[test]
    fn escaped_delimiters_do_not_end_formats_or_drop_trailing_source() {
        let mut chars = r"a\/b/${0:/upcase}\/${1:?yes\:ok:no}/} tail"
            .chars()
            .peekable();
        let t = Transform::parse(&mut chars).unwrap();
        assert_eq!(t.apply("a/b"), "A/B/no");
        assert_eq!(chars.collect::<String>(), " tail");
    }

    #[test]
    fn unicode_zero_width_global_matches_advance_without_losing_characters() {
        assert_eq!(Transform::new("(?=.)", "_", "g").apply("é中"), "_é_中");
        assert_eq!(Transform::new("", "+", "g").apply("é"), "+é+");
    }

    #[test]
    fn bad_regex_flags_backtracking_and_expansion_limits_preserve_input() {
        for (pattern, replacement, flags) in [("[", "x", ""), (".", "x", "ii"), (".", "x", "v")] {
            assert_eq!(
                Transform::new(pattern, replacement, flags).apply("abc"),
                "abc"
            );
        }
        let value = "xxxxxxxxxxy".to_owned();
        let mut expensive = Transform::new("(x+x+)+(?>y)", "x", "");
        // A deliberately tiny limit exercises runtime budget errors even when
        // the engine optimizes other pathological patterns into a linear DFA.
        expensive.regex = Some(Arc::new(
            RegexBuilder::new(&expensive.pattern)
                .backtrack_limit(1)
                .build()
                .unwrap(),
        ));
        assert!(expensive.regex.as_ref().unwrap().captures(&value).is_err());
        assert_eq!(expensive.apply(&value), value);
        let value = "a".repeat(1024);
        assert_eq!(
            Transform::new("(.*)", &"$1".repeat(300), "").apply(&value),
            value
        );
        assert_eq!(
            Transform::new("a", "b", "g").apply(&"a".repeat(257)),
            "a".repeat(257)
        );
        let large = "a".repeat(MAX_INPUT + 1);
        assert_eq!(Transform::new("a", "b", "g").apply(&large), large);
        assert_eq!(Transform::new(".*", &"b".repeat(1025), "").apply("a"), "a");
        let deep = format!("{}x{}", "${1:+".repeat(33), "}".repeat(33));
        assert_eq!(Transform::new("(.*)", &deep, "").apply("a"), "a");
    }

    #[test]
    fn delayed_multiline_substitutions_apply_the_insertion_indent_once() {
        let mut t = Transform::new("(.*)", "$1\nnew", "");
        t.indent = "  ".into();
        assert_eq!(t.apply("a"), "a\n  new");
        let mut identity = Transform::new("(?s:.*)", "$0", "");
        identity.indent = "  ".into();
        assert_eq!(identity.apply("a\n  b"), "a\n  b");
    }
}
