//! JSON with comments (JSONC), as used by `settings.json` and `keymap.json`.
//!
//! `strip` turns JSONC into plain JSON for parsing. `set` edits one member
//! in place and leaves every other byte of the file, comments included,
//! untouched, so a program write does not undo how the user laid it out.

use std::ops::Range;

use serde_json::Value;

/// A byte order mark. Windows tools such as PowerShell's
/// `Set-Content -Encoding utf8` start UTF-8 files with one.
const BOM: char = '\u{feff}';

/// Remove a leading byte order mark, `//` and `/* */` comments and trailing
/// commas, keeping line numbers intact so JSON errors still point at the
/// right line.
pub fn strip(source: &str) -> String {
    let source = source.strip_prefix(BOM).unwrap_or(source);
    let chars = source.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(source.len());
    let mut index = 0;
    let mut in_string = false;
    while index < chars.len() {
        let c = chars[index];
        if in_string {
            out.push(c);
            if c == '\\' && index + 1 < chars.len() {
                out.push(chars[index + 1]);
                index += 1;
            } else if c == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        match (c, chars.get(index + 1)) {
            ('"', _) => {
                in_string = true;
                out.push(c);
                index += 1;
            }
            ('/', Some('/')) => {
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
            }
            ('/', Some('*')) => {
                index += 2;
                while index < chars.len()
                    && !(chars[index] == '*' && chars.get(index + 1) == Some(&'/'))
                {
                    if chars[index] == '\n' {
                        out.push('\n');
                    }
                    index += 1;
                }
                index += 2;
            }
            (',', _) => {
                let next = chars[index + 1..]
                    .iter()
                    .enumerate()
                    .find(|(_, c)| !c.is_whitespace())
                    .map(|(offset, c)| (index + 1 + offset, *c));
                // Skip comments between a comma and the closing bracket.
                let closes = match next {
                    Some((_, '}' | ']')) => true,
                    Some((position, '/')) => {
                        let rest = strip(&chars[position..].iter().collect::<String>());
                        matches!(rest.trim_start().chars().next(), Some('}' | ']'))
                    }
                    _ => false,
                };
                out.push(if closes { ' ' } else { ',' });
                index += 1;
            }
            _ => {
                out.push(c);
                index += 1;
            }
        }
    }
    out
}

/// Set the member at `path` (object keys from the root) to `value`, or
/// remove it for `None`. Missing parent objects are created. Returns `None`
/// when the root is not an object; the text must otherwise be valid JSONC.
/// A leading byte order mark is kept.
pub fn set(text: &str, path: &[&str], value: Option<&Value>) -> Option<String> {
    if let Some(body) = text.strip_prefix(BOM) {
        return set(body, path, value).map(|body| format!("{BOM}{body}"));
    }
    let mut text = text.to_owned();
    if strip(&text).trim().is_empty() {
        if value.is_none() {
            return Some(text);
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str("{\n}\n");
    }
    let root = root_object(&text)?;
    Some(set_in(&text, root, path, value))
}

fn set_in(text: &str, object: Range<usize>, path: &[&str], value: Option<&Value>) -> String {
    let Some((key, rest)) = path.split_first() else {
        return text.to_owned();
    };
    let members = members(text, object.clone());
    let existing = members.iter().find(|member| member.key == *key);
    match (existing, rest.is_empty(), value) {
        (Some(member), true, Some(value)) => {
            let formatted = format_value(value, &line_indent(text, member.start));
            splice(text, member.value.clone(), &formatted)
        }
        (Some(member), true, None) => {
            let text = splice(text, removal_span(text, member), "");
            // Removing the last member would leave the one before it with a
            // trailing comma; drop that comma unless the member had its own.
            let index = members.iter().position(|m| m.start == member.start);
            match index.and_then(|i| i.checked_sub(1)).map(|i| &members[i]) {
                Some(previous)
                    if member.end == member.value.end && previous.end > previous.value.end =>
                {
                    splice(&text, previous.end - 1..previous.end, "")
                }
                _ => text,
            }
        }
        (Some(member), false, _) if text.as_bytes()[member.value.start] == b'{' => {
            set_in(text, member.value.clone(), rest, value)
        }
        // A parent of the wrong type is replaced by the object it should be.
        (Some(member), false, Some(value)) => {
            let nested = nest(rest, value);
            let formatted = format_value(&nested, &line_indent(text, member.start));
            splice(text, member.value.clone(), &formatted)
        }
        (_, _, None) => text.to_owned(),
        (None, _, Some(value)) => insert(text, object, &members, key, &nest(rest, value)),
    }
}

/// `value` wrapped in one object per key of `path`.
fn nest(path: &[&str], value: &Value) -> Value {
    path.iter().rev().fold(value.clone(), |inner, key| {
        Value::Object(serde_json::Map::from_iter([((*key).to_owned(), inner)]))
    })
}

fn insert(
    text: &str,
    object: Range<usize>,
    members: &[Member],
    key: &str,
    value: &Value,
) -> String {
    let parent_indent = line_indent(text, object.start);
    let indent = members
        .first()
        .map(|member| line_indent(text, member.start))
        .unwrap_or_else(|| format!("{parent_indent}  "));
    let entry = format!(
        "{}: {}",
        serde_json::to_string(key).expect("strings serialize"),
        format_value(value, &indent)
    );
    match members.last() {
        Some(last) => splice(
            text,
            last.value.end..last.value.end,
            &format!(",\n{indent}{entry}"),
        ),
        None => {
            let inside = object.start + 1..object.end - 1;
            if text[inside.clone()].trim().is_empty() {
                splice(text, inside, &format!("\n{indent}{entry}\n{parent_indent}"))
            } else {
                // Keep comments in an otherwise empty object below the entry.
                splice(
                    text,
                    inside.start..inside.start,
                    &format!("\n{indent}{entry}"),
                )
            }
        }
    }
}

/// The member and its comma, plus its whole line when it stands alone there.
fn removal_span(text: &str, member: &Member) -> Range<usize> {
    let line_start = text[..member.start].rfind('\n').map_or(0, |i| i + 1);
    let rest_of_line = text[member.end..].find('\n').map(|i| member.end + i);
    let alone = text[line_start..member.start].trim().is_empty()
        && rest_of_line.is_some_and(|eol| text[member.end..eol].trim().is_empty());
    match rest_of_line {
        Some(eol) if alone => line_start..eol + 1,
        _ => member.start..member.end,
    }
}

fn splice(text: &str, range: Range<usize>, replacement: &str) -> String {
    let mut out = String::with_capacity(text.len() + replacement.len());
    out.push_str(&text[..range.start]);
    out.push_str(replacement);
    out.push_str(&text[range.end..]);
    out
}

/// Pretty JSON whose continuation lines start at `indent`.
fn format_value(value: &Value, indent: &str) -> String {
    let pretty = serde_json::to_string_pretty(value).expect("values serialize");
    pretty.replace('\n', &format!("\n{indent}"))
}

fn line_indent(text: &str, offset: usize) -> String {
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    text[line_start..]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// One `"key": value` of an object, as byte offsets into the text.
#[derive(Debug)]
struct Member {
    key: String,
    /// Opening quote of the key.
    start: usize,
    value: Range<usize>,
    /// Past the value, or past its comma when one follows.
    end: usize,
}

fn root_object(text: &str) -> Option<Range<usize>> {
    let start = skip_trivia(text.as_bytes(), 0);
    (text.as_bytes().get(start) == Some(&b'{')).then(|| start..value_end(text.as_bytes(), start))
}

fn members(text: &str, object: Range<usize>) -> Vec<Member> {
    let bytes = text.as_bytes();
    let mut members = Vec::new();
    let mut index = object.start + 1;
    while index < object.end {
        index = skip_trivia(bytes, index);
        match bytes.get(index) {
            Some(b'"') => {}
            Some(b',') => {
                index += 1;
                continue;
            }
            _ => break,
        }
        let start = index;
        let key_end = string_end(bytes, start);
        let key = serde_json::from_str(&text[start..key_end]).unwrap_or_default();
        index = skip_trivia(bytes, key_end);
        if bytes.get(index) != Some(&b':') {
            break;
        }
        let value_start = skip_trivia(bytes, index + 1);
        let value_stop = value_end(bytes, value_start);
        let after = skip_trivia(bytes, value_stop);
        let end = if bytes.get(after) == Some(&b',') {
            after + 1
        } else {
            value_stop
        };
        members.push(Member {
            key,
            start,
            value: value_start..value_stop,
            end,
        });
        index = end;
    }
    members
}

/// Past whitespace and comments.
fn skip_trivia(bytes: &[u8], mut index: usize) -> usize {
    loop {
        match (bytes.get(index), bytes.get(index + 1)) {
            (Some(b' ' | b'\t' | b'\r' | b'\n'), _) => index += 1,
            (Some(b'/'), Some(b'/')) => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            (Some(b'/'), Some(b'*')) => {
                index += 2;
                while index < bytes.len() && !bytes[index..].starts_with(b"*/") {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
            }
            _ => return index,
        }
    }
}

/// Past the closing quote of the string opening at `index`.
fn string_end(bytes: &[u8], mut index: usize) -> usize {
    index += 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

/// Past the value starting at `index`.
fn value_end(bytes: &[u8], index: usize) -> usize {
    match bytes.get(index) {
        Some(b'"') => string_end(bytes, index),
        Some(b'{' | b'[') => {
            let mut depth = 0usize;
            let mut at = index;
            while at < bytes.len() {
                match bytes[at] {
                    b'"' => {
                        at = string_end(bytes, at);
                        continue;
                    }
                    b'/' if matches!(bytes.get(at + 1), Some(b'/' | b'*')) => {
                        at = skip_trivia(bytes, at);
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return at + 1;
                        }
                    }
                    _ => {}
                }
                at += 1;
            }
            bytes.len()
        }
        _ => {
            let mut at = index;
            while at < bytes.len()
                && !matches!(
                    bytes[at],
                    b',' | b'}' | b']' | b'/' | b' ' | b'\t' | b'\r' | b'\n'
                )
            {
                at += 1;
            }
            at
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(text: &str) -> Value {
        serde_json::from_str(&strip(text)).unwrap()
    }

    #[test]
    fn strips_comments_and_trailing_commas() {
        let source = "[\n  // note\n  { \"a\": \"x // y\", /* c */ \"b\": [1, 2,], },\n]";
        let value = parse(source);
        assert_eq!(value[0]["a"], "x // y");
        assert_eq!(value[0]["b"], json!([1, 2]));
        assert_eq!(strip("{\"a\": \"\\\"//\"}"), "{\"a\": \"\\\"//\"}");
        assert_eq!(strip("[1, // c\n]").trim(), "[1  \n]");
    }

    #[test]
    fn replacing_a_value_keeps_comments_and_other_members() {
        let text = "// header\n{\n  // the theme\n  \"theme\": \"ember\", // inline\n  \"vim\": { \"enabled\": true }\n}\n";
        let edited = set(text, &["theme"], Some(&json!("aura-dark"))).unwrap();
        assert_eq!(
            edited,
            "// header\n{\n  // the theme\n  \"theme\": \"aura-dark\", // inline\n  \"vim\": { \"enabled\": true }\n}\n"
        );
    }

    #[test]
    fn nested_values_are_edited_in_place() {
        let text = "{\n  \"vim\": {\n    \"enabled\": true\n  }\n}\n";
        let edited = set(text, &["vim", "smart_case"], Some(&json!(true))).unwrap();
        assert_eq!(
            edited,
            "{\n  \"vim\": {\n    \"enabled\": true,\n    \"smart_case\": true\n  }\n}\n"
        );
        let edited = set(&edited, &["vim", "enabled"], None).unwrap();
        assert_eq!(
            edited,
            "{\n  \"vim\": {\n    \"smart_case\": true\n  }\n}\n"
        );
    }

    #[test]
    fn inserting_creates_missing_parents_and_handles_empty_objects() {
        let edited = set("{\n}\n", &["editor_font", "size"], Some(&json!(14))).unwrap();
        assert_eq!(
            edited,
            "{\n  \"editor_font\": {\n    \"size\": 14\n  }\n}\n"
        );
        assert_eq!(parse(&edited), json!({"editor_font": {"size": 14}}));

        let commented = "{\n  // \"theme\": \"ember\"\n}\n";
        let edited = set(commented, &["smooth_caret"], Some(&json!(false))).unwrap();
        assert_eq!(parse(&edited), json!({"smooth_caret": false}));
        assert!(edited.contains("// \"theme\": \"ember\""));
    }

    #[test]
    fn inserting_after_a_trailing_comma_stays_valid() {
        let edited = set("{\n  \"a\": 1,\n}\n", &["b"], Some(&json!(2))).unwrap();
        assert_eq!(parse(&edited), json!({"a": 1, "b": 2}));
    }

    #[test]
    fn removing_drops_the_whole_line() {
        let text = "{\n  \"a\": 1,\n  \"b\": [1, 2],\n  \"c\": 3\n}\n";
        let edited = set(text, &["b"], None).unwrap();
        assert_eq!(edited, "{\n  \"a\": 1,\n  \"c\": 3\n}\n");
        let edited = set(&edited, &["c"], None).unwrap();
        assert_eq!(parse(&edited), json!({"a": 1}));
        assert_eq!(set(&edited, &["missing"], None).unwrap(), edited);
    }

    #[test]
    fn empty_or_comment_only_files_gain_a_root_object() {
        let edited = set("// notes\n", &["a"], Some(&json!(1))).unwrap();
        assert!(edited.starts_with("// notes\n{"));
        assert_eq!(parse(&edited), json!({"a": 1}));
        assert_eq!(set("[]", &["a"], Some(&json!(1))), None);
    }

    #[test]
    fn a_byte_order_mark_is_ignored_when_parsing_and_kept_when_editing() {
        let text = "\u{feff}{\n  // c\n  \"a\": 1\n}\n";
        assert_eq!(parse(text), json!({"a": 1}));
        let edited = set(text, &["b"], Some(&json!(2))).unwrap();
        assert_eq!(edited, "\u{feff}{\n  // c\n  \"a\": 1,\n  \"b\": 2\n}\n");
        assert_eq!(
            set("\u{feff}", &["a"], Some(&json!(1))).unwrap(),
            "\u{feff}{\n  \"a\": 1\n}\n"
        );
        // Only a leading mark is special; one inside a string is content.
        assert_eq!(parse("{\"a\": \"\u{feff}\"}"), json!({"a": "\u{feff}"}));
    }

    #[test]
    fn a_parent_of_the_wrong_type_is_replaced() {
        let edited = set("{\"vim\": true}", &["vim", "enabled"], Some(&json!(true))).unwrap();
        assert_eq!(parse(&edited), json!({"vim": {"enabled": true}}));
    }
}
