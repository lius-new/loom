//! Bounded lexical bracket index. Incomplete code never produces crossed pairs.
pub(super) fn index(text: &str, language: &str) -> Vec<(usize, usize)> {
    if text.len() > 128 * 1024 {
        return Vec::new();
    }
    let bytes = text.as_bytes();
    let mut stack = Vec::new();
    let mut pairs = Vec::new();
    let (mut i, mut quote, mut line, mut block, mut escaped) = (0, 0, false, 0usize, false);
    let hash = matches!(
        language,
        "python" | "ruby" | "shell" | "bash" | "toml" | "yaml"
    );
    let dash = matches!(language, "sql" | "lua" | "haskell");
    let slash = !hash && !matches!(language, "text" | "plain" | "plaintext" | "markdown" | "");
    while i < bytes.len() {
        let c = bytes[i];
        let n = bytes.get(i + 1).copied().unwrap_or(0);
        if line {
            if c == b'\n' {
                line = false;
            }
            i += 1;
            continue;
        }
        if block > 0 {
            if c == b'/' && n == b'*' {
                block += 1;
                i += 2;
            } else if c == b'*' && n == b'/' {
                block -= 1;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if quote != 0 {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == quote {
                quote = 0;
            }
            i += 1;
            continue;
        }
        if hash && c == b'#' || dash && c == b'-' && n == b'-' || slash && c == b'/' && n == b'/' {
            line = true;
            i += 1;
            continue;
        }
        if slash && c == b'/' && n == b'*' {
            block = 1;
            i += 2;
            continue;
        }
        if c == b'\'' && language == "rust" {
            let start = i + 1;
            let end = if n == b'\\' {
                bytes[start + 1..]
                    .iter()
                    .take(12)
                    .position(|&b| b == b'\'')
                    .map(|p| start + 1 + p)
            } else {
                text.get(start..)
                    .and_then(|s| s.chars().next())
                    .map(|c| start + c.len_utf8())
                    .filter(|&end| bytes.get(end) == Some(&b'\''))
            };
            if let Some(end) = end {
                i = end + 1;
                continue;
            }
        }
        if slash || hash || dash {
            if c == b'"' || c == b'`' || c == b'\'' && language != "rust" {
                quote = c;
                i += 1;
                continue;
            }
        }
        if matches!(c, b'(' | b'[' | b'{') {
            stack.push((i, c));
        } else if matches!(c, b')' | b']' | b'}')
            && let Some((at, open)) = stack.pop()
        {
            if matches!((open, c), (b'(', b')') | (b'[', b']') | (b'{', b'}')) {
                pairs.push((at, i));
                pairs.push((i, at));
            } else {
                stack.clear();
            }
        }
        if pairs.len() > 64 * 1024 {
            return Vec::new();
        }
        i += 1;
    }
    pairs.sort_unstable();
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nesting_incomplete_code_comments_strings_and_rust_characters_are_safe() {
        let source = "fn f<'a>() { let c = '{'; let s=\"[x]\"; /* { */ foo([x]); // }\n}";
        let pairs = index(source, "rust");
        let open = source.find("{ let").unwrap();
        let close = source.rfind('}').unwrap();
        assert!(pairs.contains(&(open, close)));
        assert!(
            !pairs
                .iter()
                .any(|&(at, _)| at == source.find("'{").unwrap() + 1)
        );
        assert!(index("{(}", "rust").is_empty());
        assert_eq!(index("{([])", "rust").len(), 4);
        assert_eq!(index("# (\n[x]", "python"), vec![(4, 6), (6, 4)]);
        assert!(index(&"()".repeat(100_000), "rust").is_empty());
    }
}
