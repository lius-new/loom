//! LSP snippets (`insertTextFormat: 2`): text with tab stops, placeholders,
//! choices and variables.
//!
//! [`parse`] turns the snippet source into the text to insert and, for each tab
//! stop, where it sits in that text. Tab stops with the same index are mirrors:
//! typing into the first is copied into the others. Stop `0` is where the caret
//! ends; a snippet without one ends at its end.
use std::ops::Range;

/// Where one tab stop (and its mirrors) are in the inserted text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabStop {
    pub index: u32,
    /// The first range is the stop; the others mirror it. A placeholder's
    /// default text is the content of its range.
    pub ranges: Vec<Range<usize>>,
    /// The values offered by a choice (`${1|a,b|}`); empty for other stops.
    pub choices: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snippet {
    /// The text to insert, with placeholders expanded to their defaults.
    pub text: String,
    /// In visiting order: 1, 2, 3 … and last 0 (added at the end if missing).
    pub stops: Vec<TabStop>,
}

#[derive(Debug)]
enum Node {
    Text(String),
    Stop { index: u32, children: Vec<Node> },
    Choice { index: u32, options: Vec<String> },
    Variable { name: String, children: Vec<Node> },
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl Parser<'_> {
    /// Nodes until `end` (a closing brace of the enclosing construct) or the end
    /// of the input. `end` is not consumed.
    fn nodes(&mut self, end: Option<char>) -> Vec<Node> {
        let mut nodes = Vec::new();
        let mut text = String::new();
        let flush = |text: &mut String, nodes: &mut Vec<Node>| {
            if !text.is_empty() {
                nodes.push(Node::Text(std::mem::take(text)));
            }
        };
        while let Some(&c) = self.chars.peek() {
            if Some(c) == end {
                break;
            }
            self.chars.next();
            match c {
                '\\' => match self.chars.peek() {
                    Some(&next @ ('$' | '}' | '\\')) => {
                        self.chars.next();
                        text.push(next);
                    }
                    _ => text.push('\\'),
                },
                '$' => match self.dollar() {
                    Some(node) => {
                        flush(&mut text, &mut nodes);
                        nodes.push(node);
                    }
                    None => text.push('$'),
                },
                other => text.push(other),
            }
        }
        flush(&mut text, &mut nodes);
        nodes
    }

    /// What follows a `$`, or `None` when it is a plain dollar sign.
    fn dollar(&mut self) -> Option<Node> {
        match *self.chars.peek()? {
            c if c.is_ascii_digit() => Some(Node::Stop {
                index: self.number()?,
                children: Vec::new(),
            }),
            c if c.is_alphabetic() || c == '_' => Some(Node::Variable {
                name: self.name(),
                children: Vec::new(),
            }),
            '{' => {
                self.chars.next();
                self.braced()
            }
            _ => None,
        }
    }

    fn number(&mut self) -> Option<u32> {
        let mut digits = String::new();
        while let Some(&c) = self.chars.peek() {
            if !c.is_ascii_digit() {
                break;
            }
            digits.push(c);
            self.chars.next();
        }
        digits.parse().ok()
    }

    fn name(&mut self) -> String {
        let mut name = String::new();
        while let Some(&c) = self.chars.peek() {
            if !(c.is_alphanumeric() || c == '_') {
                break;
            }
            name.push(c);
            self.chars.next();
        }
        name
    }

    /// After `${`.
    fn braced(&mut self) -> Option<Node> {
        let first = *self.chars.peek()?;
        if first.is_ascii_digit() {
            let index = self.number()?;
            return match self.chars.next()? {
                '}' => Some(Node::Stop {
                    index,
                    children: Vec::new(),
                }),
                ':' => {
                    let children = self.nodes(Some('}'));
                    self.chars.next(); // the closing brace, if the input had one
                    Some(Node::Stop { index, children })
                }
                '|' => {
                    let mut options = vec![String::new()];
                    loop {
                        match self.chars.next()? {
                            '\\' => {
                                let escaped = self.chars.next()?;
                                options.last_mut()?.push(escaped);
                            }
                            ',' => options.push(String::new()),
                            '|' => break,
                            other => options.last_mut()?.push(other),
                        }
                    }
                    if self.chars.peek() == Some(&'}') {
                        self.chars.next();
                    }
                    Some(Node::Choice { index, options })
                }
                _ => None,
            };
        }
        if first.is_alphabetic() || first == '_' {
            let name = self.name();
            return match self.chars.peek() {
                Some('}') => {
                    self.chars.next();
                    Some(Node::Variable {
                        name,
                        children: Vec::new(),
                    })
                }
                Some(':') => {
                    self.chars.next();
                    let children = self.nodes(Some('}'));
                    self.chars.next();
                    Some(Node::Variable { name, children })
                }
                Some('/') => {
                    // A transform: skip it, the variable's own value is used.
                    let mut slashes = 0;
                    while let Some(c) = self.chars.next() {
                        match c {
                            '\\' => {
                                self.chars.next();
                            }
                            '/' => slashes += 1,
                            '}' if slashes >= 2 => break,
                            _ => {}
                        }
                    }
                    Some(Node::Variable {
                        name,
                        children: Vec::new(),
                    })
                }
                _ => None,
            };
        }
        None
    }
}

struct Builder<'a> {
    text: String,
    stops: Vec<TabStop>,
    defaults: std::collections::HashMap<u32, String>,
    resolve: &'a dyn Fn(&str) -> Option<String>,
}

impl Builder<'_> {
    fn emit(&mut self, nodes: &[Node]) {
        for node in nodes {
            match node {
                Node::Text(text) => self.text.push_str(text),
                Node::Stop { index, children } => {
                    let start = self.text.len();
                    if children.is_empty() {
                        // A mirror, or a bare stop: show the first placeholder's text.
                        if let Some(default) = self.defaults.get(index) {
                            self.text.push_str(&default.clone());
                        }
                    } else {
                        self.emit(children);
                    }
                    self.stop(*index, start..self.text.len(), Vec::new());
                }
                Node::Choice { index, options } => {
                    let start = self.text.len();
                    let first = options.first().cloned().unwrap_or_default();
                    self.text.push_str(&first);
                    self.stop(*index, start..self.text.len(), options.clone());
                }
                Node::Variable { name, children } => match (self.resolve)(name) {
                    Some(value) => self.text.push_str(&value),
                    None if children.is_empty() => self.text.push_str(name),
                    None => self.emit(children),
                },
            }
        }
    }

    fn stop(&mut self, index: u32, range: Range<usize>, choices: Vec<String>) {
        match self.stops.iter_mut().find(|stop| stop.index == index) {
            Some(stop) => stop.ranges.push(range),
            None => self.stops.push(TabStop {
                index,
                ranges: vec![range],
                choices,
            }),
        }
    }
}

/// The first placeholder text of each tab stop, so a bare `$1` elsewhere can
/// mirror it, wherever it appears.
fn collect_defaults(
    nodes: &[Node],
    defaults: &mut std::collections::HashMap<u32, String>,
    resolve: &dyn Fn(&str) -> Option<String>,
) {
    for node in nodes {
        match node {
            Node::Stop { index, children } if !children.is_empty() => {
                if !defaults.contains_key(index) {
                    let mut flat = String::new();
                    flatten(children, &mut flat, resolve);
                    defaults.insert(*index, flat);
                }
                collect_defaults(children, defaults, resolve);
            }
            Node::Choice { index, options } => {
                defaults
                    .entry(*index)
                    .or_insert_with(|| options.first().cloned().unwrap_or_default());
            }
            Node::Variable { children, .. } => collect_defaults(children, defaults, resolve),
            _ => {}
        }
    }
}

fn flatten(nodes: &[Node], out: &mut String, resolve: &dyn Fn(&str) -> Option<String>) {
    for node in nodes {
        match node {
            Node::Text(text) => out.push_str(text),
            Node::Stop { children, .. } => flatten(children, out, resolve),
            Node::Choice { options, .. } => out.push_str(options.first().map_or("", |s| s)),
            Node::Variable { name, children } => match resolve(name) {
                Some(value) => out.push_str(&value),
                None if children.is_empty() => out.push_str(name),
                None => flatten(children, out, resolve),
            },
        }
    }
}

/// Parses `source`. `resolve` supplies variables such as `TM_FILENAME`; one it
/// does not know is replaced by its default text, or by its own name.
pub fn parse(source: &str, resolve: &dyn Fn(&str) -> Option<String>) -> Snippet {
    let nodes = Parser {
        chars: source.chars().peekable(),
    }
    .nodes(None);
    let mut defaults = std::collections::HashMap::new();
    collect_defaults(&nodes, &mut defaults, resolve);
    let mut builder = Builder {
        text: String::new(),
        stops: Vec::new(),
        defaults,
        resolve,
    };
    builder.emit(&nodes);
    let Builder {
        text, mut stops, ..
    } = builder;
    // Mirrors that came before their stop's text was known are filled in by the
    // defaults; ranges are otherwise exact. Visiting order puts 0 last.
    stops.sort_by_key(|stop| (stop.index == 0, stop.index));
    if stops.last().is_none_or(|stop| stop.index != 0) {
        stops.push(TabStop {
            index: 0,
            ranges: vec![text.len()..text.len()],
            choices: Vec::new(),
        });
    }
    Snippet { text, stops }
}

/// The variables every editor offers that Loom can answer for a document.
pub fn document_variables(
    path: &std::path::Path,
    selected: &str,
) -> impl Fn(&str) -> Option<String> {
    let file = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    let stem = path
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned());
    let directory = path.parent().map(|dir| dir.to_string_lossy().into_owned());
    let full = path.to_string_lossy().into_owned();
    let selected = selected.to_owned();
    move |name| match name {
        "TM_FILENAME" => file.clone(),
        "TM_FILENAME_BASE" => stem.clone(),
        "TM_DIRECTORY" => directory.clone(),
        "TM_FILEPATH" => Some(full.clone()),
        "TM_SELECTED_TEXT" => Some(selected.clone()),
        "TM_CURRENT_LINE" | "TM_LINE_INDEX" | "TM_LINE_NUMBER" => None,
        "CLIPBOARD" => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<String> {
        None
    }

    fn snippet(source: &str) -> Snippet {
        parse(source, &none)
    }

    fn ranges(snippet: &Snippet) -> Vec<(u32, Vec<Range<usize>>)> {
        snippet
            .stops
            .iter()
            .map(|s| (s.index, s.ranges.clone()))
            .collect()
    }

    #[test]
    fn plain_text_has_only_the_final_stop() {
        let s = snippet("hello");
        assert_eq!(s.text, "hello");
        assert_eq!(ranges(&s), [(0, vec![5..5])]);
    }

    #[test]
    fn tab_stops_are_visited_in_order_and_zero_comes_last() {
        let s = snippet("fn $1($2) {\n\t$0\n}");
        assert_eq!(s.text, "fn () {\n\t\n}");
        assert_eq!(
            ranges(&s),
            [(1, vec![3..3]), (2, vec![4..4]), (0, vec![9..9])]
        );
    }

    #[test]
    fn a_placeholder_inserts_its_default_and_selects_it() {
        let s = snippet("let ${1:name} = ${2:value};$0");
        assert_eq!(s.text, "let name = value;");
        assert_eq!(&s.text[s.stops[0].ranges[0].clone()], "name");
        assert_eq!(&s.text[s.stops[1].ranges[0].clone()], "value");
    }

    #[test]
    fn stops_with_the_same_index_mirror_each_other() {
        let s = snippet("${1:x} + $1 = ${1}");
        assert_eq!(s.text, "x + x = x");
        assert_eq!(s.stops[0].ranges, [0..1, 4..5, 8..9]);
        // The mirror may come first.
        let s = snippet("$1 and ${1:y}");
        assert_eq!(s.text, "y and y");
        assert_eq!(s.stops[0].ranges, [0..1, 6..7]);
    }

    #[test]
    fn placeholders_nest() {
        let s = snippet("${1:a ${2:b} c}$0");
        assert_eq!(s.text, "a b c");
        assert_eq!(s.stops[0].ranges, [0..5]);
        assert_eq!(s.stops[1].ranges, [2..3]);
    }

    #[test]
    fn a_choice_offers_its_options_and_starts_on_the_first() {
        let s = snippet("${1|one,two,three|} end");
        assert_eq!(s.text, "one end");
        assert_eq!(s.stops[0].choices, ["one", "two", "three"]);
        let escaped = snippet(r"${1|a\,b,c|}");
        assert_eq!(escaped.stops[0].choices, ["a,b", "c"]);
    }

    #[test]
    fn variables_resolve_or_fall_back_to_their_default_or_name() {
        let known = |name: &str| (name == "TM_FILENAME").then(|| "main.rs".to_owned());
        let s = parse(
            "// $TM_FILENAME ${TM_FILENAME} ${UNKNOWN:fallback} $OTHER ${X}",
            &known,
        );
        assert_eq!(s.text, "// main.rs main.rs fallback OTHER X");
        // A transform is skipped; the value is inserted as it is.
        let s = parse("${TM_FILENAME/(.*)\\.rs/$1/}", &known);
        assert_eq!(s.text, "main.rs");
    }

    #[test]
    fn escapes_and_stray_dollars_are_literal() {
        assert_eq!(snippet(r"cost: \$5 \\ \}").text, r"cost: $5 \ }");
        assert_eq!(snippet("a $ b $").text, "a $ b $");
        assert_eq!(snippet(r"\n stays").text, r"\n stays");
    }

    #[test]
    fn unterminated_constructs_do_not_panic() {
        for source in [
            "${", "${1", "${1:abc", "${1|a,b", "${A:", "$", "${1:${2:", "}",
        ] {
            let _ = snippet(source);
        }
        assert_eq!(snippet("${1:abc").text, "abc");
    }

    #[test]
    fn unicode_text_keeps_byte_ranges_correct() {
        let s = snippet("变量 ${1:名字} é");
        assert_eq!(&s.text[s.stops[0].ranges[0].clone()], "名字");
    }

    #[test]
    fn document_variables_come_from_the_path() {
        let resolve = document_variables(std::path::Path::new("/w/src/lib.rs"), "sel");
        assert_eq!(resolve("TM_FILENAME").as_deref(), Some("lib.rs"));
        assert_eq!(resolve("TM_FILENAME_BASE").as_deref(), Some("lib"));
        assert_eq!(resolve("TM_SELECTED_TEXT").as_deref(), Some("sel"));
        assert_eq!(resolve("NOPE"), None);
    }
}
