//! Outline items from the headings of a Markdown text. This reads lines, not a
//! parse tree: it is cheap enough to run on every pause in typing and needs no
//! language server.
use super::model::{Item, Kind};

struct Heading {
    level: u8,
    title: String,
    line_start: usize,
    title_start: usize,
}

pub fn items(text: &str) -> Vec<Item> {
    let headings = headings(text);
    let mut stack: Vec<(u8, Item)> = Vec::new();
    let mut roots: Vec<Item> = Vec::new();
    // `end_of` closes the section of a heading when the next one at its level
    // or above begins.
    let close = |stack: &mut Vec<(u8, Item)>, roots: &mut Vec<Item>, until: u8, end: usize| {
        while stack.last().is_some_and(|(level, _)| *level >= until) {
            let (_, mut done) = stack.pop().expect("checked non-empty");
            done.full.end = end;
            match stack.last_mut() {
                Some((_, parent)) => parent.children.push(done),
                None => roots.push(done),
            }
        }
    };
    for heading in headings {
        // An empty heading has nothing to show or navigate to.
        if heading.title.is_empty() {
            continue;
        }
        close(&mut stack, &mut roots, heading.level, heading.line_start);
        stack.push((
            heading.level,
            Item {
                name: heading.title,
                detail: None,
                kind: Kind::Heading(heading.level),
                deprecated: false,
                full: heading.line_start..text.len(),
                select: heading.title_start..heading.title_start,
                children: Vec::new(),
            },
        ));
    }
    close(&mut stack, &mut roots, 0, text.len());
    roots
}

fn headings(text: &str) -> Vec<Heading> {
    let lines = lines(text);
    let mut out = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut first = true;
    let mut index = 0;
    while index < lines.len() {
        let (start, line) = lines[index];
        let at_top = std::mem::take(&mut first);
        // Front matter at the very top is not content.
        if at_top
            && line.trim_end() == "---"
            && let Some(close) = lines[index + 1..]
                .iter()
                .position(|(_, line)| matches!(line.trim_end(), "---" | "..."))
        {
            index += close + 2;
            continue;
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        let body = &line[indent..];
        if let Some((mark, length)) = fence {
            let run = body.chars().take_while(|&c| c == mark).count();
            if indent < 4 && run >= length && body[run..].trim().is_empty() {
                fence = None;
            }
            index += 1;
            continue;
        }
        if indent < 4 {
            let mark = body.chars().next().filter(|c| matches!(c, '`' | '~'));
            if let Some(mark) = mark {
                let run = body.chars().take_while(|&c| c == mark).count();
                if run >= 3 && !(mark == '`' && body[run..].contains('`')) {
                    fence = Some((mark, run));
                    index += 1;
                    continue;
                }
            }
            if let Some(heading) = atx(start, indent, body) {
                out.push(heading);
                index += 1;
                continue;
            }
            if let Some(&(_, next)) = lines.get(index + 1)
                && let Some(level) = setext(next)
                && is_paragraph_line(body)
            {
                out.push(Heading {
                    level,
                    title: body.trim().to_owned(),
                    line_start: start,
                    title_start: start + indent,
                });
                index += 2;
                continue;
            }
        }
        index += 1;
    }
    out
}

fn lines(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    for piece in text.split_inclusive('\n') {
        out.push((start, piece.trim_end_matches(['\n', '\r'])));
        start += piece.len();
    }
    out
}

fn atx(start: usize, indent: usize, body: &str) -> Option<Heading> {
    let level = body.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &body[level..];
    if !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    let trimmed = rest.trim();
    // A closing run of # counts only when a space precedes it ("C#" keeps its #).
    let stripped = trimmed.trim_end_matches('#');
    let title = if stripped.is_empty() {
        ""
    } else if stripped.len() < trimmed.len() && stripped.ends_with([' ', '\t']) {
        stripped.trim_end()
    } else {
        trimmed
    };
    let leading = rest.len() - rest.trim_start().len();
    Some(Heading {
        level: level as u8,
        title: title.to_owned(),
        line_start: start,
        title_start: start + indent + level + leading,
    })
}

fn setext(line: &str) -> Option<u8> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let body = line.trim();
    if !body.is_empty() && body.chars().all(|c| c == '=') {
        Some(1)
    } else if !body.is_empty() && body.chars().all(|c| c == '-') {
        Some(2)
    } else {
        None
    }
}

fn is_paragraph_line(body: &str) -> bool {
    let body = body.trim();
    !body.is_empty()
        && !body.starts_with(['#', '>', '|', '-', '*', '+', '`', '~'])
        && !body.starts_with(|c: char| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .map(|item| format!("{}[{}]", item.name, names(&item.children).join(",")))
            .collect()
    }

    #[test]
    fn headings_nest_by_level_and_skip_code_fences_and_front_matter() {
        let text = "---\ntitle: x\n---\n# One\ntext\n## Two\n```\n# not a heading\n```\n### Deep\n# Three ##\n";
        let items = items(text);
        assert_eq!(names(&items), ["One[Two[Deep[]]]", "Three[]"]);
        assert_eq!(items[0].kind, Kind::Heading(1));
    }

    #[test]
    fn a_section_covers_text_until_the_next_heading_at_its_level() {
        let text = "# A\nbody\n## B\nmore\n# C\n";
        let items = items(text);
        assert_eq!(&text[items[0].full.clone()], "# A\nbody\n## B\nmore\n");
        assert_eq!(&text[items[0].children[0].full.clone()], "## B\nmore\n");
        assert_eq!(&text[items[1].full.clone()], "# C\n");
    }

    #[test]
    fn navigation_lands_on_the_title_text() {
        let text = "x\n##   Spaced  title\n";
        let items = items(text);
        assert_eq!(items[0].name, "Spaced  title");
        assert_eq!(&text[items[0].select.start..][..6], "Spaced");
    }

    #[test]
    fn setext_headings_and_crlf_lines_are_read() {
        let text = "Title\r\n=====\r\nSub\r\n---\r\n\r\n- item\r\n";
        let items = items(text);
        assert_eq!(names(&items), ["Title[Sub[]]"]);
    }

    #[test]
    fn multibyte_titles_and_closing_hashes_never_split_a_character() {
        let text = "# 概览（说明）\n## C# 语言 ##\n### 全角）#\n####\n";
        let items = items(text);
        assert_eq!(names(&items), ["概览（说明）[C# 语言[全角）#[]]]"]);
        assert!(items[0].children[0].children[0].children.is_empty());
    }

    #[test]
    fn hash_without_a_space_is_not_a_heading() {
        assert!(items("#tag\n#!shebang\n").is_empty());
    }
}
