//! Loading keymap files: the built-in defaults and the user's `keymap.json`.
//!
//! Files are JSONC (comments and trailing commas allowed) holding an array of
//! sections: `{ "context": "Editor", "bindings": { "ctrl-s": "editor::Save" } }`.
//! A bad binding is reported and skipped; the rest of the file still loads.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use super::action::Action;
use super::context::ContextPredicate;
use super::keymap::{Binding, BindingSource, Keymap};
use super::keystroke::Keystroke;
use crate::workspace_persistence::config_path;

const DEFAULT_KEYMAP: &str = include_str!("../../assets/keymaps/default.json");
pub const FILE_NAME: &str = "keymap.json";

pub const USER_TEMPLATE: &str = r#"// Loom key bindings. Entries here override the built-in defaults.
//
// Each section maps keystrokes to actions while its context is active:
//   - keystrokes: modifiers joined with "-" (ctrl, alt, shift, cmd, secondary),
//     sequences separated by spaces: "secondary-k secondary-s"
//   - contexts: Workspace, Editor, Terminal, Input, Menu, CloseConfirmation,
//     CloneRepository; combine with &&, ||, ! and >
//   - actions: "namespace::Name", ["pane::ActivateItem", 0], or null to disable
//
// The Keymap page (secondary-k secondary-s) lists every action and its binding.
[
  // {
  //   "context": "Editor",
  //   "bindings": {
  //     "alt-up": "editor::MoveToBeginning"
  //   }
  // }
]
"#;

/// Bindings parsed from one file, plus messages for entries that were skipped.
#[derive(Debug, Default)]
pub struct LoadOutcome {
    pub bindings: Vec<Binding>,
    pub errors: Vec<String>,
}

/// Parse keymap file contents. `Err` means the file as a whole is unusable.
pub fn parse(content: &str, source: BindingSource) -> Result<LoadOutcome, String> {
    let stripped = strip_jsonc(content);
    if stripped.trim().is_empty() {
        return Ok(LoadOutcome::default());
    }
    let value: Value = serde_json::from_str(&stripped).map_err(|error| error.to_string())?;
    let Value::Array(sections) = value else {
        return Err("a keymap file must contain an array of sections".to_owned());
    };
    let mut outcome = LoadOutcome::default();
    for (index, section) in sections.iter().enumerate() {
        parse_section(index, section, source, &mut outcome);
    }
    Ok(outcome)
}

fn parse_section(index: usize, section: &Value, source: BindingSource, outcome: &mut LoadOutcome) {
    let label = |context: &str| {
        if context.is_empty() {
            format!("section {}", index + 1)
        } else {
            format!("section {} ({context})", index + 1)
        }
    };
    let Value::Object(fields) = section else {
        outcome
            .errors
            .push(format!("{}: expected an object", label("")));
        return;
    };
    let context = match fields.get("context") {
        None => "",
        Some(Value::String(context)) => context.as_str(),
        Some(_) => {
            outcome
                .errors
                .push(format!("{}: `context` must be a string", label("")));
            return;
        }
    };
    let label = label(context);
    for key in fields.keys() {
        if key != "context" && key != "bindings" {
            outcome
                .errors
                .push(format!("{label}: unknown field `{key}`"));
        }
    }
    let predicate = if context.trim().is_empty() {
        None
    } else {
        match ContextPredicate::parse(context) {
            Ok(predicate) => Some(Arc::new(predicate)),
            Err(error) => {
                outcome.errors.push(format!("{label}: {error}"));
                return;
            }
        }
    };
    let bindings = match fields.get("bindings") {
        None => return,
        Some(Value::Object(bindings)) => bindings,
        Some(_) => {
            outcome
                .errors
                .push(format!("{label}: `bindings` must be an object"));
            return;
        }
    };
    for (keys, action) in bindings {
        let keystrokes = match Keystroke::parse_sequence(keys) {
            Ok(keystrokes) => keystrokes,
            Err(error) => {
                outcome.errors.push(format!("{label}: \"{keys}\": {error}"));
                continue;
            }
        };
        let action = match Action::from_json(action) {
            Ok(action) => action,
            Err(error) => {
                outcome
                    .errors
                    .push(format!("{label}: \"{keys}\": {}", error.0));
                continue;
            }
        };
        outcome.bindings.push(Binding {
            keystrokes,
            action,
            predicate: predicate.clone(),
            source,
        });
    }
}

/// The built-in bindings. The default file is covered by tests, so a parse
/// failure here is a build defect.
pub fn default_bindings() -> Vec<Binding> {
    let outcome = parse(DEFAULT_KEYMAP, BindingSource::Default).expect("default keymap parses");
    debug_assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    outcome.bindings
}

pub fn default_keymap() -> Keymap {
    Keymap::new(default_bindings())
}

/// Defaults followed by the user's bindings, which therefore take precedence.
pub fn build(user: Vec<Binding>) -> Keymap {
    let mut keymap = default_keymap();
    keymap.add_bindings(user);
    keymap
}

pub fn user_path() -> Option<PathBuf> {
    config_path(FILE_NAME)
}

/// The result of reading the user's keymap file.
pub enum UserKeymap {
    Missing,
    Loaded(LoadOutcome),
    Invalid(String),
}

pub fn load_user() -> UserKeymap {
    let Some(path) = user_path() else {
        return UserKeymap::Missing;
    };
    match fs::read_to_string(&path) {
        Ok(content) => match parse(&content, BindingSource::User) {
            Ok(outcome) => UserKeymap::Loaded(outcome),
            Err(error) => UserKeymap::Invalid(error),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => UserKeymap::Missing,
        Err(error) => UserKeymap::Invalid(error.to_string()),
    }
}

/// Create the user keymap from the template if it does not exist yet.
pub fn ensure_user_file() -> io::Result<PathBuf> {
    let path = user_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "could not determine the user configuration directory",
        )
    })?;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, USER_TEMPLATE)?;
    }
    Ok(path)
}

/// Remove `//` and `/* */` comments and trailing commas, keeping line
/// numbers intact so JSON errors still point at the right line.
pub fn strip_jsonc(source: &str) -> String {
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
                        let rest = strip_jsonc(&chars[position..].iter().collect::<String>());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::context::KeyContext;

    #[test]
    fn default_keymap_loads_without_errors() {
        let outcome = parse(DEFAULT_KEYMAP, BindingSource::Default).unwrap();
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert!(outcome.bindings.len() > 50);
    }

    #[test]
    fn strips_comments_and_trailing_commas() {
        let source = "[\n  // note\n  { \"a\": \"x // y\", /* c */ \"b\": [1, 2,], },\n]";
        let value: Value = serde_json::from_str(&strip_jsonc(source)).unwrap();
        assert_eq!(value[0]["a"], "x // y");
        assert_eq!(value[0]["b"], serde_json::json!([1, 2]));
        assert_eq!(strip_jsonc("{\"a\": \"\\\"//\"}"), "{\"a\": \"\\\"//\"}");
        assert_eq!(strip_jsonc("[1, // c\n]").trim(), "[1  \n]");
    }

    #[test]
    fn bad_entries_are_reported_and_skipped() {
        let source = r#"[
            { "context": "Editor", "bindings": {
                "ctrl-q": "editor::Undo",
                "hyper-q": "editor::Undo",
                "ctrl-r": "editor::Nope"
            } },
            { "context": "Editor &&", "bindings": { "ctrl-t": "editor::Undo" } },
            { "bindings": { "f5": null }, "extra": true }
        ]"#;
        let outcome = parse(source, BindingSource::User).unwrap();
        assert_eq!(outcome.bindings.len(), 2);
        assert_eq!(outcome.errors.len(), 4, "{:?}", outcome.errors);
        assert!(
            outcome
                .bindings
                .iter()
                .all(|binding| binding.source == BindingSource::User)
        );
    }

    #[test]
    fn whole_file_failures_are_errors() {
        assert!(parse("{", BindingSource::User).is_err());
        assert!(parse("{}", BindingSource::User).is_err());
        assert!(
            parse("// only a comment\n", BindingSource::User)
                .unwrap()
                .bindings
                .is_empty()
        );
    }

    #[test]
    fn user_template_is_valid_and_empty() {
        let outcome = parse(USER_TEMPLATE, BindingSource::User).unwrap();
        assert!(outcome.bindings.is_empty() && outcome.errors.is_empty());
    }

    #[test]
    fn user_bindings_override_defaults() {
        let outcome = parse(
            r#"[{ "context": "Editor", "bindings": { "secondary-s": "editor::SelectAll" } }]"#,
            BindingSource::User,
        )
        .unwrap();
        let keymap = build(outcome.bindings);
        let stack = [KeyContext::new("Workspace"), KeyContext::new("Editor")];
        let typed = Keystroke::parse_sequence("secondary-s").unwrap();
        let (bindings, _) = keymap.bindings_for_input(&typed, &stack);
        assert_eq!(bindings[0].action, Action::SelectAll);
    }
}
