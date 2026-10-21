//! Validated LSP globs, independent of the filesystem and notification backend.
use super::uri;
use regex::{Regex, RegexBuilder};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Pattern {
    pub root: PathBuf,
    pub allow_parent: bool,
    base: Option<String>,
    matcher: Regex,
    scan_prefix: String,
    max_depth: Option<usize>,
}

/// The URI spelling also removes Windows extended-path prefixes.
pub fn path_text(path: &Path) -> Option<String> {
    let uri = uri::file_uri(path)?;
    let path = uri::file_path(&uri)?.components().collect::<PathBuf>();
    let text = if cfg!(windows) {
        path.to_str()?.replace('\\', "/")
    } else {
        path.to_str()?.to_owned()
    };
    Some(if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    })
}

fn glob(pattern: &str) -> Result<Regex, String> {
    if pattern.is_empty() || pattern.len() > 4096 {
        return Err("a glob must contain 1..4096 bytes".into());
    }
    // LSP character classes cannot cross a path separator, and '?' consumes
    // one Unicode character. Byte-oriented filesystem glob engines differ here.
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut regex = String::from("\\A");
    let mut at = 0;
    let mut braces = 0;
    while at < chars.len() {
        match chars[at] {
            '*' => {
                let start = at;
                while chars.get(at) == Some(&'*') {
                    at += 1;
                }
                if at - start == 1 {
                    regex.push_str("[^/]*");
                } else if (start == 0 || chars[start - 1] == '/') && chars.get(at) == Some(&'/') {
                    regex.push_str("(?:[^/]+/)*");
                    at += 1;
                } else if start > 0 && chars[start - 1] == '/' && at == chars.len() {
                    regex.pop();
                    regex.push_str("(?:/.*)?");
                } else {
                    regex.push_str(".*");
                }
                continue;
            }
            '?' => regex.push_str("[^/]"),
            '{' => {
                braces += 1;
                if braces > 16 {
                    return Err("glob alternatives nest too deeply".into());
                }
                regex.push_str("(?:");
            }
            '}' => {
                if braces == 0 {
                    return Err("unmatched glob brace".into());
                }
                braces -= 1;
                regex.push(')');
            }
            ',' if braces > 0 => regex.push('|'),
            '[' => {
                at += 1;
                let mut class = String::from("[");
                if chars.get(at) == Some(&'!') {
                    class.push('^');
                    at += 1;
                }
                let start = at;
                while at < chars.len() && chars[at] != ']' {
                    if chars[at] == '\\' {
                        at += 1;
                        class.push_str(&regex::escape(
                            &chars.get(at).ok_or("unfinished glob escape")?.to_string(),
                        ));
                    } else if chars[at] == '-' {
                        class.push('-');
                    } else {
                        class.push_str(&regex::escape(&chars[at].to_string()));
                    }
                    at += 1;
                }
                if at == start || chars.get(at) != Some(&']') {
                    return Err("unfinished glob character class".into());
                }
                class.push(']');
                regex.push_str(&format!("[{class}&&[^/]]"));
            }
            '\\' => {
                at += 1;
                regex.push_str(&regex::escape(
                    &chars.get(at).ok_or("unfinished glob escape")?.to_string(),
                ));
            }
            literal => regex.push_str(&regex::escape(&literal.to_string())),
        }
        at += 1;
    }
    if braces != 0 {
        return Err("unfinished glob alternative".into());
    }
    regex.push_str("\\z");
    RegexBuilder::new(&regex)
        .case_insensitive(cfg!(windows))
        .dot_matches_new_line(true)
        .size_limit(1024 * 1024)
        .build()
        .map_err(|error| format!("invalid LSP glob: {error}"))
}

impl Pattern {
    pub fn parse(value: &Value, workspace: &Path) -> Result<Self, String> {
        let (pattern, base) = if let Some(pattern) = value.as_str() {
            (pattern, None)
        } else {
            let base = value.get("baseUri").ok_or("missing baseUri")?;
            let base = base
                .as_str()
                .or_else(|| base.get("uri")?.as_str())
                .and_then(uri::file_path)
                .ok_or("baseUri must be an absolute file URI")?;
            (
                value
                    .get("pattern")
                    .and_then(Value::as_str)
                    .ok_or("missing pattern")?,
                Some(base),
            )
        };
        // Windows servers also send native absolute paths as string globs.
        // In that spelling backslashes are separators; relative patterns retain
        // the protocol's backslash escaping convention.
        let mut normalized;
        let pattern = if cfg!(windows) && Path::new(pattern).is_absolute() {
            normalized = pattern.replace('\\', "/");
            if let Some(unc) = normalized.strip_prefix("//?/UNC/") {
                normalized = format!("//{unc}");
            } else if let Some(path) = normalized.strip_prefix("//?/") {
                normalized = path.to_owned();
            }
            normalized.as_str()
        } else {
            pattern
        };
        let absolute = Path::new(pattern).is_absolute();
        if base.is_some() && (absolute || pattern.split('/').any(|part| part == "..")) {
            return Err("relative patterns must remain below baseUri".into());
        }
        let root = if let Some(base) = &base {
            base.clone()
        } else if absolute {
            let prefix = pattern.find(['*', '?', '{', '[']).unwrap_or(pattern.len());
            let prefix = &pattern[..prefix];
            let end = prefix
                .rfind('/')
                .ok_or("absolute globs use '/' separators")?;
            PathBuf::from(if end == 0 {
                "/"
            } else if end == 2 && prefix.as_bytes().get(1) == Some(&b':') {
                &prefix[..=end]
            } else {
                &prefix[..end]
            })
        } else {
            workspace.to_owned()
        };
        let base = if absolute {
            None
        } else {
            Some(path_text(base.as_deref().unwrap_or(workspace)).ok_or("invalid glob base")?)
        };
        let escaped = pattern.contains('\\');
        let first_glob = pattern.find(['*', '?', '{', '[']);
        let prefix = if escaped {
            ""
        } else if let Some(at) = first_glob {
            pattern[..at]
                .rfind('/')
                .map(|end| &pattern[..=end])
                .unwrap_or("")
        } else {
            pattern
        };
        let scan_prefix = if absolute && !escaped {
            path_text(Path::new(prefix))
        } else {
            let base_path = base
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| workspace.to_owned());
            path_text(&base_path.join(prefix))
        }
        .ok_or("invalid glob prefix")?;
        let max_depth =
            (!pattern.contains("**") && !pattern.contains('{') && !escaped).then(|| {
                pattern
                    .split('/')
                    .filter(|segment| !segment.is_empty())
                    .count()
            });
        Ok(Self {
            root,
            allow_parent: absolute && base.is_none(),
            base,
            matcher: glob(pattern)?,
            scan_prefix,
            max_depth,
        })
    }

    /// A conservative traversal filter: never scan unrelated home directories
    /// just because a missing literal file requires leasing its ancestor.
    pub fn may_descend(&self, path: &Path) -> bool {
        let Some(path) = path_text(path) else {
            return false;
        };
        let path = path.trim_end_matches('/');
        let prefix = self.scan_prefix.trim_end_matches('/');
        if path != prefix
            && !path.starts_with(&(prefix.to_owned() + "/"))
            && !prefix.starts_with(&(path.to_owned() + "/"))
        {
            return false;
        }
        if let Some(limit) = self.max_depth {
            let relative = match &self.base {
                Some(base) if base.starts_with(&(path.to_owned() + "/")) => return true,
                Some(base) => match path.strip_prefix(base.trim_end_matches('/')) {
                    Some(relative) => relative,
                    None => return false,
                },
                None => path,
            };
            if relative
                .split('/')
                .filter(|segment| !segment.is_empty())
                .count()
                >= limit
            {
                return false;
            }
        }
        true
    }

    pub fn matches(&self, path: &Path) -> bool {
        let Some(path) = path_text(path) else {
            return false;
        };
        let relative = match &self.base {
            Some(base) => {
                let base = base.trim_end_matches('/');
                let Some(relative) = path.strip_prefix(base).and_then(|p| p.strip_prefix('/'))
                else {
                    return false;
                };
                relative
            }
            None => &path,
        };
        self.matcher.is_match(relative)
    }
}

#[derive(Clone, Debug)]
pub struct Watch {
    pub pattern: Pattern,
    pub kind: u8,
}

impl Watch {
    pub fn parse(options: &Value, workspace: &Path) -> Result<Vec<Self>, String> {
        let watchers = options
            .get("watchers")
            .and_then(Value::as_array)
            .ok_or("missing watchers array")?;
        if watchers.len() > 128 {
            return Err("at most 128 watchers per registration".into());
        }
        watchers
            .iter()
            .map(|watch| {
                let kind = match watch.get("kind") {
                    None => 7,
                    Some(value) => value
                        .as_u64()
                        .filter(|kind| *kind <= 7)
                        .ok_or("invalid WatchKind")? as u8,
                };
                Ok(Self {
                    pattern: Pattern::parse(
                        watch.get("globPattern").ok_or("missing globPattern")?,
                        workspace,
                    )?,
                    kind,
                })
            })
            .collect()
    }

    pub fn matches(&self, path: &Path, change: u8) -> bool {
        let mask = match change {
            1 => 1,
            2 => 2,
            3 => 4,
            _ => return false,
        };
        self.kind & mask != 0 && self.pattern.matches(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lsp_glob_grammar_and_separators() {
        let cases = [
            ("*.rs", "a.rs", true),
            ("*.rs", "dir/a.rs", false),
            ("**/*.rs", "a.rs", true),
            ("**/*.rs", "x/y/a.rs", true),
            ("a/**/b", "a/b", true),
            ("a/**/b", "a/x/y/b", true),
            ("?.rs", "x.rs", true),
            ("?.rs", "xx.rs", false),
            ("?.rs", "中.rs", true),
            ("[!a-z].rs", "中.rs", true),
            ("**/中文?.{rs,ts}", "src/中文件.rs", true),
            ("a/**", "a", true),
            ("**/*.{ts,js}", "x/a.ts", true),
            ("**/*.{ts,js}", "x/a.rs", false),
            ("a.[0-9]", "a.8", true),
            ("a.[0-9]", "a.x", false),
            ("a.[!0-9]", "a.x", true),
            ("a.[!0-9]", "a.3", false),
            ("a.[!0-9]", "a./", false),
            (r"a\[x\].rs", "a[x].rs", true),
            ("**/.git/config", ".git/config", true),
        ];
        for (pattern, path, expected) in cases {
            assert_eq!(
                glob(pattern).unwrap().is_match(path),
                expected,
                "{pattern} => {path}"
            );
        }
        assert!(glob("[").is_err());
    }

    #[test]
    fn relative_patterns_are_uri_based_and_confined_and_masks_are_checked() {
        let root = std::env::temp_dir().join("中文 space");
        let base = uri::file_uri(&root).unwrap();
        for base in [json!(base), json!({"uri": base, "name": "test"})] {
            let watchers = Watch::parse(&json!({"watchers": [{"globPattern": {"baseUri": base, "pattern": "**/*.rs"}, "kind": 5}]}), &root).unwrap();
            assert!(watchers[0].matches(&root.join("src/a.rs"), 1));
            assert!(watchers[0].matches(&root.join("a.rs"), 3));
            assert!(!watchers[0].matches(&root.join("a.rs"), 2));
            assert!(!watchers[0].matches(&root.with_extension("else").join("a.rs"), 1));
        }
        for pattern in [
            json!({"baseUri":"https://example.com", "pattern":"**"}),
            json!({"baseUri":base,"pattern":"../**"}),
        ] {
            assert!(Pattern::parse(&pattern, &root).is_err());
        }
        assert!(
            Watch::parse(&json!({"watchers":[{"globPattern":"**", "kind":8}]}), &root).is_err()
        );
    }

    #[test]
    fn absolute_globs_lease_their_literal_prefix_and_root_drives() {
        let root = std::env::temp_dir();
        let pattern = format!("{}/*.wat", path_text(&root).unwrap());
        let pattern = Pattern::parse(&serde_json::json!(pattern), &root).unwrap();
        assert!(pattern.matches(&root.join("x.wat")));
        assert!(!pattern.matches(&root.join("sub/x.wat")));
        assert_eq!(path_text(&pattern.root), path_text(&root));
        assert!(!pattern.may_descend(&root.join("unrelated")));
        let exact = Pattern::parse(
            &serde_json::json!(format!("{}/missing/config", path_text(&root).unwrap())),
            &root,
        )
        .unwrap();
        assert!(exact.may_descend(&root));
        assert!(!exact.may_descend(&root.join("unrelated")));
        if cfg!(windows) {
            let pattern = Pattern::parse(&serde_json::json!("C:/*.wat"), &root).unwrap();
            assert!(pattern.root.is_absolute());
            let native = format!("{}\\*.wat", root.to_string_lossy().trim_end_matches('\\'));
            assert!(
                Pattern::parse(&serde_json::json!(native), &root)
                    .unwrap()
                    .matches(&root.join("x.wat"))
            );
        }
    }
}
