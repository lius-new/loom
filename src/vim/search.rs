//! Vim search patterns, translated to the `regex` crate's syntax.
//!
//! Patterns use Vim's default "magic" syntax: `.`, `*`, `[]`, `^` and `$` are
//! special, while `\(`, `\|`, `\+`, `\?`, `\=`, `\{n,m}`, `\<` and `\>` must be
//! escaped to be special. `\c` and `\C` force case (in)sensitivity.

use regex::{Regex, RegexBuilder};

/// Compile a Vim pattern. `ignore_case` and `smart_case` follow the Vim
/// options of the same names.
pub fn compile(pattern: &str, ignore_case: bool, smart_case: bool) -> Result<Regex, String> {
    let mut out = String::new();
    let mut case: Option<bool> = None;
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let Some(next) = chars.next() else {
                    out.push_str("\\\\");
                    break;
                };
                match next {
                    '<' | '>' => out.push_str("\\b"),
                    '(' => out.push('('),
                    ')' => out.push(')'),
                    '|' => out.push('|'),
                    '+' => out.push('+'),
                    '?' | '=' => out.push('?'),
                    '{' => {
                        let lazy = chars.peek() == Some(&'-');
                        if lazy {
                            chars.next();
                        }
                        let mut bounds = String::new();
                        for c in chars.by_ref() {
                            if c == '}' {
                                break;
                            }
                            if c != '\\' {
                                bounds.push(c);
                            }
                        }
                        if bounds.is_empty() {
                            out.push('*');
                        } else {
                            if bounds.starts_with(',') {
                                bounds.insert(0, '0');
                            }
                            out.push('{');
                            out.push_str(&bounds);
                            out.push('}');
                        }
                        if lazy {
                            out.push('?');
                        }
                    }
                    'c' => case = Some(true),
                    'C' => case = Some(false),
                    'n' => out.push_str("\\n"),
                    't' => out.push_str("\\t"),
                    'r' => out.push_str("\\r"),
                    'e' => out.push_str("\\x1b"),
                    's' | 'S' | 'd' | 'D' | 'w' | 'W' => {
                        out.push('\\');
                        out.push(next);
                    }
                    'a' => out.push_str("[A-Za-z]"),
                    'A' => out.push_str("[^A-Za-z]"),
                    'l' => out.push_str("[a-z]"),
                    'L' => out.push_str("[^a-z]"),
                    'u' => out.push_str("[A-Z]"),
                    'U' => out.push_str("[^A-Z]"),
                    'x' => out.push_str("[0-9A-Fa-f]"),
                    'X' => out.push_str("[^0-9A-Fa-f]"),
                    'h' => out.push_str("[A-Za-z_]"),
                    'H' => out.push_str("[^A-Za-z_]"),
                    '1'..='9' => {
                        return Err(format!("E486: Back references are not supported: \\{next}"));
                    }
                    other => out.push_str(&regex::escape(&other.to_string())),
                }
            }
            '[' => {
                // Copy a collection through its closing bracket.
                let mut class = String::from("[");
                let mut closed = false;
                if chars.peek() == Some(&'^') {
                    class.push(chars.next().unwrap());
                }
                if chars.peek() == Some(&']') {
                    chars.next();
                    class.push_str("\\]");
                }
                while let Some(c) = chars.next() {
                    match c {
                        ']' => {
                            closed = true;
                            break;
                        }
                        '\\' => {
                            if let Some(escaped) = chars.next() {
                                match escaped {
                                    'n' => class.push_str("\\n"),
                                    't' => class.push_str("\\t"),
                                    other => class.push_str(&regex::escape(&other.to_string())),
                                }
                            }
                        }
                        '[' => class.push_str("\\["),
                        other => class.push(other),
                    }
                }
                if closed {
                    class.push(']');
                    out.push_str(&class);
                } else {
                    out.push_str("\\[");
                    out.push_str(&regex::escape(&class[1..]));
                }
            }
            '.' | '*' | '^' | '$' => out.push(c),
            '~' => out.push('~'),
            other => out.push_str(&regex::escape(&other.to_string())),
        }
    }
    let insensitive = case
        .unwrap_or_else(|| ignore_case && !(smart_case && pattern.chars().any(char::is_uppercase)));
    RegexBuilder::new(&out)
        .case_insensitive(insensitive)
        .multi_line(true)
        .build()
        .map_err(|error| format!("E383: Invalid search pattern: {pattern} ({error})"))
}

/// A match found by `find`, and whether the search wrapped around.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    pub start: usize,
    pub end: usize,
    pub wrapped: bool,
}

/// The `count`-th match after (or before) `from`, wrapping around the text.
pub fn find(text: &str, regex: &Regex, from: usize, forward: bool, count: usize) -> Option<Found> {
    let matches: Vec<(usize, usize)> = regex
        .find_iter(text)
        .map(|found| (found.start(), found.end()))
        .collect();
    if matches.is_empty() {
        return None;
    }
    let mut position = from;
    let mut wrapped = false;
    let mut current = None;
    for _ in 0..count.max(1) {
        let next = if forward {
            matches
                .iter()
                .find(|(start, _)| *start > position)
                .or_else(|| {
                    wrapped = true;
                    matches.first()
                })
        } else {
            matches
                .iter()
                .rev()
                .find(|(start, _)| *start < position)
                .or_else(|| {
                    wrapped = true;
                    matches.last()
                })
        };
        let &(start, end) = next?;
        position = start;
        current = Some(Found {
            start,
            end,
            wrapped,
        });
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pattern: &str, text: &str) -> Vec<String> {
        compile(pattern, false, false)
            .unwrap()
            .find_iter(text)
            .map(|found| found.as_str().to_owned())
            .collect()
    }

    #[test]
    fn magic_syntax_translates_to_regex() {
        assert_eq!(matches("a.c", "abc a.c"), ["abc", "a.c"]);
        assert_eq!(matches("a\\.c", "abc a.c"), ["a.c"]);
        assert_eq!(matches("f(x)", "f(x) fx"), ["f(x)"]);
        assert_eq!(matches("\\(ab\\)\\+", "ababx"), ["abab"]);
        assert_eq!(matches("\\<is\\>", "this is"), ["is"]);
        assert_eq!(matches("a\\{2}", "aaa"), ["aa"]);
        assert_eq!(matches("a\\{-1,}", "aaa"), ["a", "a", "a"]);
        assert_eq!(matches("x\\|y", "xyz"), ["x", "y"]);
        assert_eq!(matches("^b", "a\nb"), ["b"]);
        assert_eq!(matches("[]a]", "]a"), ["]", "a"]);
        assert_eq!(matches("a+b?", "a+b?"), ["a+b?"]);
        assert!(compile("\\1", false, false).is_err());
    }

    #[test]
    fn case_follows_options_and_overrides() {
        let found =
            |pattern, ignore, smart| compile(pattern, ignore, smart).unwrap().is_match("Hello");
        assert!(!found("hello", false, false));
        assert!(found("hello", true, false));
        assert!(found("hello", true, true));
        assert!(
            !found("hEllo", true, true),
            "smartcase: an uppercase letter"
        );
        assert!(found("hello\\c", false, false));
        assert!(!found("hello\\C", true, false));
    }

    #[test]
    fn finding_wraps_in_both_directions() {
        let regex = compile("x", false, false).unwrap();
        let text = "x-x-x";
        let start = |found: Option<Found>| found.map(|found| (found.start, found.wrapped));
        assert_eq!(start(find(text, &regex, 0, true, 1)), Some((2, false)));
        assert_eq!(start(find(text, &regex, 4, true, 1)), Some((0, true)));
        assert_eq!(start(find(text, &regex, 0, false, 1)), Some((4, true)));
        assert_eq!(start(find(text, &regex, 2, true, 2)), Some((0, true)));
        assert_eq!(
            find(text, &compile("y", false, false).unwrap(), 0, true, 1),
            None
        );
    }
}
