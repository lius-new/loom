//! The on-disk shape of a language description, and the rules a description must
//! satisfy before it can be registered. Parsing is strict: an unknown or
//! misspelled field anywhere is an error.
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Descriptor {
    pub name: String,
    pub display_name: String,
    pub badge: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub comments: Comments,
    pub variants: Vec<VariantDescriptor>,
    #[serde(default)]
    pub files: Vec<FileRuleDescriptor>,
}

/// Editing markers only; they never replace a grammar's own rules.
#[derive(Debug, Default, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Comments {
    pub line: Option<String>,
    pub block: Option<[String; 2]>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VariantDescriptor {
    pub id: String,
    pub grammar: Option<String>,
    /// The language server that serves this variant, if any.
    #[serde(default)]
    pub server: Option<ServerBindingDescriptor>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerBindingDescriptor {
    /// The name of a server definition (`assets/servers/<name>.json`).
    pub definition: String,
    /// The `languageId` sent to the server for documents of this variant. It is
    /// the server's vocabulary, not Loom's, so it may differ from the names here.
    pub language_id: String,
}

/// How a server finds the project root of a file.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RootStrategy {
    /// The nearest folder above the file that holds a marker.
    #[default]
    Nearest,
    /// The outermost such folder, but never above the workspace folder.
    Outermost,
}

/// What to use when no marker folder is found.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RootFallback {
    /// The workspace folder that holds the file.
    Workspace,
    /// The folder the file is in.
    FileDirectory,
    /// No server for such a file.
    None,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RootDescriptor {
    pub markers: Vec<String>,
    #[serde(default)]
    pub strategy: RootStrategy,
    pub fallback: RootFallback,
}

/// A language server Loom knows how to start: what to run and how to find the
/// project root. Per-user choices of program and arguments live in settings.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerDescriptor {
    pub name: String,
    pub display_name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub root: RootDescriptor,
    #[serde(default)]
    pub initialization_options: Option<serde_json::Value>,
}

/// A server name: lowercase words that may be joined by hyphens.
pub(crate) fn is_server_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

impl ServerDescriptor {
    /// The initialization options as compact JSON text, for handing to the server.
    pub fn options_json(&self) -> Option<String> {
        self.initialization_options
            .as_ref()
            .map(|options| options.to_string())
    }

    pub fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|error| error.to_string())
    }

    pub fn validate(&self, file_stem: &str) -> Result<(), String> {
        if !is_server_name(&self.name) {
            return Err(format!("`{}` is not a lowercase server name", self.name));
        }
        if self.name != file_stem {
            return Err(format!(
                "name `{}` must match the file name `{file_stem}.json`",
                self.name
            ));
        }
        if self.display_name.trim().is_empty() || self.display_name != self.display_name.trim() {
            return Err("display_name must be non-empty text without outer spaces".into());
        }
        if self.command.trim().is_empty() || self.command.chars().any(char::is_control) {
            return Err("command must be a program name or path".into());
        }
        if self
            .args
            .iter()
            .any(|arg| arg.chars().any(char::is_control))
        {
            return Err("arguments must not contain control characters".into());
        }
        if self.root.markers.is_empty() {
            return Err("root.markers needs at least one file name".into());
        }
        for marker in &self.root.markers {
            let bad = marker.trim().is_empty()
                || marker
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '/' | '\\'));
            if bad {
                return Err(format!("invalid root marker {marker:?}"));
            }
        }
        if let Some(options) = &self.initialization_options
            && !options.is_object()
        {
            return Err("initialization_options must be a JSON object".into());
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileRuleDescriptor {
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub names: Vec<String>,
    /// Tree-sitter file-types are filename suffixes, including compound ones.
    #[serde(default)]
    pub suffixes: Vec<String>,
    pub variant: String,
}

pub(crate) fn is_canonical_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A file extension as rules and lookups compare it: lowercase, no dot.
pub(crate) fn normalize_extension(extension: &str) -> Result<String, String> {
    let bad = extension.is_empty()
        || extension.starts_with('.')
        || extension
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '\\' | '*' | '?'));
    if bad {
        return Err(format!("invalid file extension {extension:?}"));
    }
    Ok(extension.to_ascii_lowercase())
}

/// An exact file name as rules and lookups compare it: lowercase, no separators.
pub(crate) fn normalize_file_name(name: &str) -> Result<String, String> {
    let bad = name.trim().is_empty()
        || name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\'));
    if bad {
        return Err(format!("invalid file name {name:?}"));
    }
    Ok(name.to_ascii_lowercase())
}

pub(crate) fn normalize_suffix(suffix: &str) -> Result<String, String> {
    if suffix.contains(['*', '?', ':']) {
        return Err(format!("invalid file suffix {suffix:?}"));
    }
    normalize_file_name(suffix)
}

impl Descriptor {
    pub fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|error| error.to_string())
    }

    /// Checks one description on its own; clashes between descriptions are the
    /// registry's job. `file_stem` is the description's file name without
    /// `.json`, which must equal its `name`.
    pub fn validate(&self, file_stem: &str) -> Result<(), String> {
        if !is_canonical_name(&self.name) {
            return Err(format!("`{}` is not a canonical lowercase name", self.name));
        }
        if self.name != file_stem {
            return Err(format!(
                "name `{}` must match the file name `{file_stem}.json`",
                self.name
            ));
        }
        if self.display_name.trim().is_empty() || self.display_name != self.display_name.trim() {
            return Err("display_name must be non-empty text without outer spaces".into());
        }
        let badge_len = self.badge.chars().count();
        if !(1..=4).contains(&badge_len)
            || self
                .badge
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(format!(
                "badge {:?} must be 1 to 4 visible characters",
                self.badge
            ));
        }
        for (index, alias) in self.aliases.iter().enumerate() {
            if !is_canonical_name(alias) {
                return Err(format!("alias `{alias}` is not a canonical lowercase name"));
            }
            if alias == &self.name || self.aliases[..index].contains(alias) {
                return Err(format!("alias `{alias}` is repeated"));
            }
        }
        for marker in [&self.comments.line].into_iter().flatten() {
            if marker.is_empty() || marker.contains(['\n', '\r']) {
                return Err("the line comment marker must be one line of text".into());
            }
        }
        if let Some([open, close]) = &self.comments.block
            && (open.is_empty() || close.is_empty())
        {
            return Err("block comment markers must both be non-empty".into());
        }
        if self.variants.is_empty() {
            return Err("at least one variant is required".into());
        }
        for (index, variant) in self.variants.iter().enumerate() {
            if !is_canonical_name(&variant.id) {
                return Err(format!("variant `{}` is not a canonical name", variant.id));
            }
            if self.variants[..index].iter().any(|v| v.id == variant.id) {
                return Err(format!("variant `{}` is repeated", variant.id));
            }
            if let Some(grammar) = &variant.grammar
                && !is_canonical_name(grammar)
            {
                return Err(format!("grammar `{grammar}` is not a canonical name"));
            }
            if let Some(server) = &variant.server {
                if !is_server_name(&server.definition) {
                    return Err(format!(
                        "server `{}` is not a server name",
                        server.definition
                    ));
                }
                let id = &server.language_id;
                if id.is_empty() || id.chars().any(|c| c.is_whitespace() || c.is_control()) {
                    return Err(format!("server language_id {id:?} must be a plain word"));
                }
            }
        }
        for rule in &self.files {
            if rule.extensions.is_empty() && rule.names.is_empty() && rule.suffixes.is_empty() {
                return Err("a file rule needs at least one extension or name".into());
            }
            if !self.variants.iter().any(|v| v.id == rule.variant) {
                return Err(format!(
                    "file rule names unknown variant `{}`",
                    rule.variant
                ));
            }
            for extension in &rule.extensions {
                normalize_extension(extension)?;
            }
            for name in &rule.names {
                normalize_file_name(name)?;
            }
            for suffix in &rule.suffixes {
                normalize_suffix(suffix)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::language::descriptor::*;

    const GOOD: &str = r##"{
        "name": "demo", "display_name": "Demo", "badge": "DM",
        "aliases": ["dm"],
        "comments": { "line": "#", "block": ["<!--", "-->"] },
        "variants": [{ "id": "demo", "grammar": "rust" }, { "id": "demo_x" }],
        "files": [{ "extensions": ["demo"], "names": ["Demofile"], "variant": "demo" }]
    }"##;

    fn check(json: &str) -> Result<(), String> {
        Descriptor::parse(json)?.validate("demo")
    }

    #[test]
    fn a_complete_description_is_accepted() {
        check(GOOD).unwrap();
    }

    #[test]
    fn unknown_fields_fail_at_every_level() {
        let cases = [
            GOOD.replace("\"badge\"", "\"badges\""),
            GOOD.replace("\"line\": \"#\"", "\"line\": \"#\", \"extra\": 1"),
            GOOD.replace(
                "{ \"id\": \"demo_x\" }",
                "{ \"id\": \"demo_x\", \"server\": \"x\" }",
            ),
            GOOD.replace(
                "\"variant\": \"demo\" }]",
                "\"variant\": \"demo\", \"glob\": \"*\" }]",
            ),
        ];
        for case in cases {
            assert!(Descriptor::parse(&case).is_err(), "{case}");
        }
        assert!(
            Descriptor::parse("{ \"name\": \"demo\" }").is_err(),
            "missing fields"
        );
        assert!(Descriptor::parse("not json").is_err());
    }

    #[test]
    fn invalid_values_are_rejected_with_a_reason() {
        let cases: [(&str, String); 14] = [
            (
                "canonical",
                GOOD.replace("\"name\": \"demo\"", "\"name\": \"Demo\""),
            ),
            (
                "match the file name",
                GOOD.replace("\"name\": \"demo\"", "\"name\": \"other\""),
            ),
            ("display_name", GOOD.replace("\"Demo\"", "\" Demo\"")),
            ("display_name", GOOD.replace("\"Demo\"", "\"\"")),
            ("badge", GOOD.replace("\"DM\"", "\"\"")),
            ("badge", GOOD.replace("\"DM\"", "\"TOOLONG\"")),
            ("badge", GOOD.replace("\"DM\"", "\"D M\"")),
            ("alias", GOOD.replace("[\"dm\"]", "[\"dm\", \"dm\"]")),
            ("alias", GOOD.replace("[\"dm\"]", "[\"Dm\"]")),
            ("line comment", GOOD.replace("\"#\"", "\"\"")),
            (
                "block comment",
                GOOD.replace("[\"<!--\", \"-->\"]", "[\"<!--\", \"\"]"),
            ),
            (
                "variant `demo` is repeated",
                GOOD.replace("\"demo_x\"", "\"demo\""),
            ),
            (
                "unknown variant",
                GOOD.replace("\"variant\": \"demo\" }]", "\"variant\": \"nope\" }]"),
            ),
            (
                "at least one",
                GOOD.replace(
                    "\"extensions\": [\"demo\"], \"names\": [\"Demofile\"], ",
                    "",
                ),
            ),
        ];
        for (expected, case) in cases {
            let error = check(&case).unwrap_err();
            assert!(
                error.contains(expected),
                "expected {expected:?}, got {error:?}\n{case}"
            );
        }
        assert!(
            check(&GOOD.replace("\"variants\": [{ \"id\": \"demo\", \"grammar\": \"rust\" }, { \"id\": \"demo_x\" }]", "\"variants\": []"))
                .unwrap_err()
                .contains("at least one variant")
        );
    }

    #[test]
    fn extensions_and_names_are_validated_and_normalized() {
        assert_eq!(normalize_extension("TSX").unwrap(), "tsx");
        assert_eq!(normalize_file_name("Makefile").unwrap(), "makefile");
        for bad in ["", ".rs", "a/b", "a\\b", "a b", "a*", "a?", "t\tab"] {
            assert!(normalize_extension(bad).is_err(), "{bad:?}");
        }
        for bad in ["", "  ", "a/b", "a\\b", "a\nb"] {
            assert!(normalize_file_name(bad).is_err(), "{bad:?}");
        }
        assert!(normalize_file_name("Dockerfile.dev").is_ok());
    }
}
