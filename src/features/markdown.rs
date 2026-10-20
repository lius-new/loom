//! A small Markdown reader for documentation shown in hover cards, completion
//! details and signature help: the subset language servers actually send.
//!
//! Blocks: paragraphs, headings, fenced and indented code, block quotes,
//! bulleted and numbered lists, rules. Inline: code, strong, emphasis, links,
//! escapes and line breaks. Raw HTML is dropped, not shown. Nothing is ever
//! rejected: text that is not Markdown reads as paragraphs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Code(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Link {
        label: Vec<Inline>,
        url: String,
    },
    /// A forced line break inside a paragraph.
    Break,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Heading {
        level: u8,
        content: Vec<Inline>,
    },
    Code {
        language: Option<String>,
        text: String,
    },
    Quote(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    Rule,
}

/// More than this many lines are not read: a card shows a screenful at most.
const MAX_LINES: usize = 400;
const MAX_DEPTH: usize = 6;

pub fn parse(source: &str) -> Vec<Block> {
    let lines: Vec<&str> = source.lines().take(MAX_LINES).collect();
    blocks(&lines, 0)
}

fn indent(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

fn fence(line: &str) -> Option<(char, usize, &str)> {
    let trimmed = line.trim_start();
    if indent(line) > 3 {
        return None;
    }
    let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let count = trimmed.chars().take_while(|c| *c == marker).count();
    if count < 3 {
        return None;
    }
    let info = trimmed[count..].trim();
    if marker == '`' && info.contains('`') {
        return None;
    }
    Some((marker, count, info))
}

fn rule(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    indent(line) <= 3
        && compact.len() >= 3
        && ["-", "*", "_"]
            .iter()
            .any(|m| compact.chars().all(|c| c.to_string() == *m))
}

fn heading(line: &str) -> Option<(u8, &str)> {
    if indent(line) > 3 {
        return None;
    }
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !(rest.is_empty() || rest.starts_with(' ')) {
        return None;
    }
    let text = rest.trim().trim_end_matches('#').trim_end();
    Some((hashes as u8, text))
}

/// `(marker width, start number for ordered)` of a list item start.
fn list_marker(line: &str) -> Option<(usize, Option<u64>)> {
    let lead = indent(line);
    if lead > 3 {
        return None;
    }
    let rest = &line[lead..];
    let mut chars = rest.chars();
    let first = chars.next()?;
    if matches!(first, '-' | '*' | '+') {
        let after = chars.next();
        if after.is_none() || after == Some(' ') {
            return Some((lead + 2, None));
        }
        return None;
    }
    if first.is_ascii_digit() {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.len() > 9 {
            return None;
        }
        let after = &rest[digits.len()..];
        if (after.starts_with(". ") || after.starts_with(") ") || after == "." || after == ")")
            && let Ok(number) = digits.parse::<u64>()
        {
            return Some((lead + digits.len() + 2, Some(number)));
        }
    }
    None
}

fn blocks(lines: &[&str], depth: usize) -> Vec<Block> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if is_blank(line) {
            i += 1;
            continue;
        }
        if let Some((marker, count, info)) = fence(line) {
            let language = info
                .split_whitespace()
                .next()
                .map(|l| l.trim_matches(|c| c == '{' || c == '}').to_owned())
                .filter(|l| !l.is_empty());
            let lead = indent(line);
            let mut body = Vec::new();
            i += 1;
            while i < lines.len() {
                let candidate = lines[i];
                if let Some((m, n, rest)) = fence(candidate)
                    && m == marker
                    && n >= count
                    && rest.is_empty()
                {
                    i += 1;
                    break;
                }
                let strip = indent(candidate).min(lead);
                body.push(&candidate[strip..]);
                i += 1;
            }
            out.push(Block::Code {
                language,
                text: body.join("\n"),
            });
            continue;
        }
        if let Some((level, text)) = heading(line) {
            out.push(Block::Heading {
                level,
                content: inlines(text),
            });
            i += 1;
            continue;
        }
        if rule(line)
            && list_marker(line)
                .is_none_or(|(_, n)| n.is_none() && !line.trim_start().starts_with("- "))
        {
            out.push(Block::Rule);
            i += 1;
            continue;
        }
        if line.trim_start().starts_with('>') && depth < MAX_DEPTH {
            let mut inner = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let trimmed = lines[i].trim_start()[1..].to_owned();
                inner.push(
                    trimmed
                        .strip_prefix(' ')
                        .map_or(trimmed.clone(), str::to_owned),
                );
                i += 1;
            }
            let refs: Vec<&str> = inner.iter().map(String::as_str).collect();
            out.push(Block::Quote(blocks(&refs, depth + 1)));
            continue;
        }
        if let Some((_, start)) = list_marker(line)
            && depth < MAX_DEPTH
        {
            let ordered = start.is_some();
            let mut items: Vec<Vec<String>> = Vec::new();
            while i < lines.len() {
                let current = lines[i];
                if indent(current) < 2
                    && let Some((width, number)) = list_marker(current)
                {
                    if number.is_some() != ordered {
                        break;
                    }
                    items.push(vec![current[width.min(current.len())..].to_owned()]);
                    i += 1;
                } else if is_blank(current) {
                    // A blank line continues the list only if an indented line follows.
                    let next = lines.get(i + 1).copied().unwrap_or("");
                    if !is_blank(next) && (indent(next) >= 2 || list_marker(next).is_some()) {
                        if let Some(item) = items.last_mut() {
                            item.push(String::new());
                        }
                        i += 1;
                    } else {
                        break;
                    }
                } else if indent(current) >= 2 {
                    if let Some(item) = items.last_mut() {
                        item.push(current[indent(current).min(4)..].to_owned());
                    }
                    i += 1;
                } else {
                    break;
                }
            }
            let items = items
                .iter()
                .map(|item| {
                    let refs: Vec<&str> = item.iter().map(String::as_str).collect();
                    blocks(&refs, depth + 1)
                })
                .collect();
            out.push(Block::List { start, items });
            continue;
        }
        if indent(line) >= 4 {
            let mut body = Vec::new();
            while i < lines.len() && (indent(lines[i]) >= 4 || is_blank(lines[i])) {
                body.push(lines[i].get(4.min(indent(lines[i]))..).unwrap_or(""));
                i += 1;
            }
            while body.last().is_some_and(|l| l.trim().is_empty()) {
                body.pop();
            }
            out.push(Block::Code {
                language: None,
                text: body.join("\n"),
            });
            continue;
        }
        // A paragraph runs to a blank line or the start of another block.
        let mut text = String::new();
        while i < lines.len() {
            let current = lines[i];
            if is_blank(current)
                || fence(current).is_some()
                || heading(current).is_some()
                || (rule(current) && !text.is_empty() && false)
            {
                break;
            }
            if !text.is_empty()
                && (current.trim_start().starts_with('>') || list_marker(current).is_some())
            {
                break;
            }
            if !text.is_empty() {
                // Two trailing spaces or a backslash make the break hard.
                if text.ends_with("  ") || text.ends_with('\\') {
                    text = text.trim_end_matches('\\').trim_end().to_owned();
                    text.push('\n');
                } else {
                    text = text.trim_end().to_owned();
                    text.push(' ');
                }
            }
            text.push_str(current.trim_start());
            i += 1;
        }
        if !text.trim().is_empty() {
            out.push(Block::Paragraph(inlines(text.trim_end())));
        }
    }
    out
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// Inline content: code spans, strong, emphasis, links and escapes.
pub fn inlines(text: &str) -> Vec<Inline> {
    parse_inlines(&text.chars().collect::<Vec<_>>(), 0)
}

fn flush(text: &mut String, out: &mut Vec<Inline>) {
    if !text.is_empty() {
        out.push(Inline::Text(std::mem::take(text)));
    }
}

fn find_closing(chars: &[char], from: usize, marker: &[char]) -> Option<usize> {
    let mut i = from;
    while i + marker.len() <= chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '`' => {
                // Skip a code span so its contents cannot close emphasis.
                let run = chars[i..].iter().take_while(|c| **c == '`').count();
                let close = (i + run..chars.len()).find(|&j| {
                    chars[j] == '`' && chars[j..].iter().take_while(|c| **c == '`').count() == run
                });
                match close {
                    Some(j) => i = j + run,
                    None => i += run,
                }
            }
            _ if chars[i..i + marker.len()] == *marker => return Some(i),
            _ => i += 1,
        }
    }
    None
}

fn parse_inlines(chars: &[char], depth: usize) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if chars.get(i + 1).is_some_and(|n| n.is_ascii_punctuation()) => {
                text.push(chars[i + 1]);
                i += 2;
            }
            '\n' => {
                flush(&mut text, &mut out);
                out.push(Inline::Break);
                i += 1;
            }
            '`' => {
                let run = chars[i..].iter().take_while(|c| **c == '`').count();
                let close = (i + run..chars.len()).find(|&j| {
                    chars[j] == '`' && chars[j..].iter().take_while(|c| **c == '`').count() == run
                });
                match close {
                    Some(j) => {
                        flush(&mut text, &mut out);
                        let code: String = chars[i + run..j].iter().collect();
                        let code = code.replace('\n', " ");
                        let code = if code.starts_with(' ')
                            && code.ends_with(' ')
                            && code.trim().len() < code.len().saturating_sub(1)
                            && !code.trim().is_empty()
                        {
                            code[1..code.len() - 1].to_owned()
                        } else {
                            code
                        };
                        out.push(Inline::Code(code));
                        i = j + run;
                    }
                    None => {
                        text.extend(std::iter::repeat_n('`', run));
                        i += run;
                    }
                }
            }
            '*' | '_' if depth < MAX_DEPTH => {
                let run = chars[i..].iter().take_while(|x| **x == c).count();
                let before = i.checked_sub(1).map(|k| chars[k]);
                let after = chars.get(i + run).copied();
                // Underscores inside words are literal (snake_case).
                let intraword =
                    c == '_' && before.is_some_and(is_word_char) && after.is_some_and(is_word_char);
                let can_open = after.is_some_and(|a| !a.is_whitespace()) && !intraword;
                if !can_open {
                    text.extend(std::iter::repeat_n(c, run));
                    i += run;
                    continue;
                }
                let width = run.min(2);
                let marker: Vec<char> = std::iter::repeat_n(c, width).collect();
                let closing = find_closing(chars, i + run.min(width), &marker).map(|mut j| {
                    // In a longer run the last characters close, so `**a *b***` nests.
                    while width == 2 && chars.get(j + 2) == Some(&c) {
                        j += 1;
                    }
                    j
                });
                match closing {
                    Some(j) if j > i + width && !chars[j - 1].is_whitespace() => {
                        flush(&mut text, &mut out);
                        let inner = parse_inlines(&chars[i + width..j], depth + 1);
                        out.push(if width == 2 {
                            Inline::Strong(inner)
                        } else {
                            Inline::Emphasis(inner)
                        });
                        i = j + width;
                    }
                    _ => {
                        text.extend(std::iter::repeat_n(c, run));
                        i += run;
                    }
                }
            }
            '[' if depth < MAX_DEPTH => {
                // [label](url) with balanced brackets in the label.
                let mut level = 0;
                let mut close = None;
                for (j, ch) in chars.iter().enumerate().skip(i) {
                    match ch {
                        '[' => level += 1,
                        ']' => {
                            level -= 1;
                            if level == 0 {
                                close = Some(j);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let link = close
                    .filter(|j| chars.get(j + 1) == Some(&'('))
                    .and_then(|j| {
                        let end = (j + 2..chars.len()).find(|&k| chars[k] == ')')?;
                        Some((j, end))
                    });
                match link {
                    Some((j, end)) => {
                        flush(&mut text, &mut out);
                        let url: String = chars[j + 2..end].iter().collect();
                        let url = url.split_whitespace().next().unwrap_or("").to_owned();
                        out.push(Inline::Link {
                            label: parse_inlines(&chars[i + 1..j], depth + 1),
                            url,
                        });
                        i = end + 1;
                    }
                    None => {
                        text.push('[');
                        i += 1;
                    }
                }
            }
            '<' => {
                // <https://…> is a link; any other tag is dropped.
                match chars[i..].iter().position(|ch| *ch == '>') {
                    Some(rel) if rel > 1 => {
                        let inner: String = chars[i + 1..i + rel].iter().collect();
                        if inner.starts_with("http://") || inner.starts_with("https://") {
                            flush(&mut text, &mut out);
                            out.push(Inline::Link {
                                label: vec![Inline::Text(inner.clone())],
                                url: inner,
                            });
                            i += rel + 1;
                        } else if inner
                            .chars()
                            .next()
                            .is_some_and(|f| f.is_alphabetic() || f == '/' || f == '!')
                            && !inner.contains(' ')
                            || inner.starts_with("br")
                        {
                            if inner.starts_with("br") {
                                flush(&mut text, &mut out);
                                out.push(Inline::Break);
                            }
                            i += rel + 1;
                        } else {
                            text.push('<');
                            i += 1;
                        }
                    }
                    _ => {
                        text.push('<');
                        i += 1;
                    }
                }
            }
            other => {
                text.push(other);
                i += 1;
            }
        }
    }
    flush(&mut text, &mut out);
    out
}

/// The text of inline content without any styling, for measuring and tests.
pub fn plain(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(text) | Inline::Code(text) => out.push_str(text),
            Inline::Strong(inner) | Inline::Emphasis(inner) => out.push_str(&plain(inner)),
            Inline::Link { label, .. } => out.push_str(&plain(label)),
            Inline::Break => out.push('\n'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Inline {
        Inline::Text(s.to_owned())
    }

    #[test]
    fn paragraphs_join_soft_breaks_and_split_on_blank_lines() {
        let blocks = parse("one\ntwo\n\nthree");
        assert_eq!(
            blocks,
            [
                Block::Paragraph(vec![text("one two")]),
                Block::Paragraph(vec![text("three")]),
            ]
        );
    }

    #[test]
    fn hard_breaks_stay_breaks() {
        let blocks = parse("one  \ntwo\\\nthree");
        assert_eq!(
            blocks,
            [Block::Paragraph(vec![
                text("one"),
                Inline::Break,
                text("two"),
                Inline::Break,
                text("three")
            ])]
        );
    }

    #[test]
    fn headings_rules_and_quotes() {
        let blocks = parse("# Title\n\n---\n\n> quoted\n> more");
        assert_eq!(
            blocks[0],
            Block::Heading {
                level: 1,
                content: vec![text("Title")]
            }
        );
        assert_eq!(blocks[1], Block::Rule);
        assert_eq!(
            blocks[2],
            Block::Quote(vec![Block::Paragraph(vec![text("quoted more")])])
        );
        assert_eq!(
            parse("#nospace"),
            [Block::Paragraph(vec![text("#nospace")])]
        );
    }

    #[test]
    fn fenced_code_keeps_its_text_and_language() {
        let blocks = parse("```rust\nfn main() {\n    let x = 1;\n}\n```\nafter");
        assert_eq!(
            blocks[0],
            Block::Code {
                language: Some("rust".into()),
                text: "fn main() {\n    let x = 1;\n}".into()
            }
        );
        assert_eq!(blocks[1], Block::Paragraph(vec![text("after")]));
        // An unclosed fence runs to the end; a longer fence is not closed by a shorter one.
        let blocks = parse("````\n```\ninside\n````");
        assert_eq!(
            blocks[0],
            Block::Code {
                language: None,
                text: "```\ninside".into()
            }
        );
        let open = parse("```\nnever closed");
        assert_eq!(
            open[0],
            Block::Code {
                language: None,
                text: "never closed".into()
            }
        );
    }

    #[test]
    fn indented_code_is_code() {
        let blocks = parse("text\n\n    let a = 1;\n    let b = 2;\n");
        assert_eq!(
            blocks[1],
            Block::Code {
                language: None,
                text: "let a = 1;\nlet b = 2;".into()
            }
        );
    }

    #[test]
    fn lists_bulleted_numbered_and_nested() {
        let blocks = parse("- one\n- two\n  - inner\n- three");
        let Block::List { start: None, items } = &blocks[0] else {
            panic!("{blocks:?}")
        };
        assert_eq!(items.len(), 3);
        assert!(matches!(&items[1][1], Block::List { .. }));
        let blocks = parse("3. a\n4. b");
        assert!(matches!(&blocks[0], Block::List { start: Some(3), items } if items.len() == 2));
    }

    #[test]
    fn inline_styles_nest() {
        let content = inlines("a **bold *and em*** `code` [link](http://x.y) _e_ snake_case_name");
        assert_eq!(content[0], text("a "));
        assert!(
            matches!(&content[1], Inline::Strong(inner) if matches!(inner[1], Inline::Emphasis(_)))
        );
        assert!(content.contains(&Inline::Code("code".into())));
        assert!(content.contains(&Inline::Link {
            label: vec![text("link")],
            url: "http://x.y".into()
        }));
        assert!(content.contains(&Inline::Emphasis(vec![text("e")])));
        assert_eq!(plain(&content), "a bold and em code link e snake_case_name");
    }

    #[test]
    fn escapes_html_and_unmatched_markers_are_plain() {
        assert_eq!(plain(&inlines(r"\*not\* \[x\]")), "*not* [x]");
        assert_eq!(plain(&inlines("a <b>bold</b> c")), "a bold c");
        assert_eq!(plain(&inlines("2 * 3 * 4")), "2 * 3 * 4");
        assert_eq!(plain(&inlines("a < b")), "a < b");
        assert_eq!(plain(&inlines("[unclosed")), "[unclosed");
        assert_eq!(inlines("a<br>b"), [text("a"), Inline::Break, text("b")]);
        assert!(matches!(&inlines("<https://a.b>")[0], Inline::Link { .. }));
    }

    #[test]
    fn code_spans_protect_their_contents() {
        assert_eq!(inlines("`a*b*c`"), [Inline::Code("a*b*c".into())]);
        assert_eq!(inlines("`` a`b ``"), [Inline::Code("a`b".into())]);
        assert_eq!(plain(&inlines("`unclosed")), "`unclosed");
    }

    #[test]
    fn hostile_input_is_bounded() {
        let deep = ">".repeat(500) + " x";
        let _ = parse(&deep);
        let nested = "*".repeat(200) + "x" + &"*".repeat(200);
        let _ = inlines(&nested);
        let many = "line\n".repeat(10_000);
        assert!(!parse(&many).is_empty());
        let _ = parse("- ".repeat(300).as_str());
    }

    #[test]
    fn plain_text_that_is_not_markdown_is_paragraphs() {
        let blocks = parse("fn add(a: i32, b: i32) -> i32\n\nAdds two numbers.");
        assert_eq!(blocks.len(), 2);
        assert!(parse("").is_empty());
        assert!(parse("   \n\n  ").is_empty());
    }
}
