//! User preferences, stored in `settings.json` like VS Code's user settings.
//!
//! The file is JSONC holding only the values that differ from the defaults.
//! The Settings page edits it member by member, so the user's comments and
//! layout survive; hand edits are picked up by a file watcher.

use std::fs;
use std::io;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::features::terminal::ShellKind;
use crate::fonts::FontSettings;
use crate::jsonc;
use crate::theme::ThemeId;
use crate::workspace_persistence::config_path;

pub const FILE_NAME: &str = "settings.json";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Settings {
    #[serde(default)]
    pub theme: ThemeId,
    #[serde(default)]
    pub default_shell: ShellKind,
    #[serde(default)]
    pub terminal_profiles: Vec<crate::features::terminal::profiles::ShellProfile>,
    #[serde(default = "default_true")]
    pub terminal_cursor_blink: bool,
    #[serde(default = "default_true")]
    pub smooth_caret: bool,
    #[serde(default)]
    pub git_inline_blame: bool,
    #[serde(default)]
    pub git_split_diff: bool,
    #[serde(default)]
    pub git_tree_view: bool,
    #[serde(default)]
    pub vim: VimSettings,
    #[serde(default)]
    pub editor_font: FontSettings,
    #[serde(default = "default_terminal_font")]
    pub terminal_font: FontSettings,
}

/// Preferences for Vim editing (`"vim"` in `settings.json`).
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct VimSettings {
    pub enabled: bool,
    /// Yanks, deletes and puts without a register name use the system clipboard.
    pub system_clipboard: bool,
    pub ignore_case: bool,
    pub smart_case: bool,
}

impl From<VimSettings> for crate::vim::Options {
    fn from(settings: VimSettings) -> Self {
        Self {
            enabled: settings.enabled,
            system_clipboard: settings.system_clipboard,
            ignore_case: settings.ignore_case,
            smart_case: settings.smart_case,
        }
    }
}

impl From<crate::vim::Options> for VimSettings {
    fn from(options: crate::vim::Options) -> Self {
        Self {
            enabled: options.enabled,
            system_clipboard: options.system_clipboard,
            ignore_case: options.ignore_case,
            smart_case: options.smart_case,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeId::default(),
            default_shell: ShellKind::default(),
            terminal_profiles: Vec::new(),
            terminal_cursor_blink: true,
            smooth_caret: true,
            git_inline_blame: false,
            git_split_diff: false,
            git_tree_view: false,
            vim: VimSettings::default(),
            editor_font: FontSettings::EDITOR,
            terminal_font: FontSettings::TERMINAL,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_terminal_font() -> FontSettings {
    FontSettings::TERMINAL
}

/// Settings read from one file, plus messages for values that were ignored.
#[derive(Debug, Default)]
pub struct LoadOutcome {
    pub settings: Settings,
    pub errors: Vec<String>,
}

/// The result of reading `settings.json`.
pub enum UserSettings {
    Missing,
    Loaded(LoadOutcome),
    /// The file as a whole is unusable; nothing in it applies.
    Invalid(String),
}

/// Settings a project's `.loom/settings.json` may override. Everything else
/// is a personal preference and only read from the user's file.
pub const PROJECT_KEYS: &[&str] = &["default_shell"];

/// Which file a settings source is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    User,
    Project,
}

/// The values a settings file sets, validated, keyed like `Settings`; a
/// nested object holds only the fields the file sets.
pub type Overrides = Map<String, Value>;

/// Parse file contents over the defaults. An unknown key or a bad value is
/// reported and skipped, so one mistake never resets the other settings.
/// `Err` means the file is not a JSONC object at all.
pub fn parse(source: &str) -> Result<LoadOutcome, String> {
    let (overrides, errors) = parse_overrides(source, Scope::User)?;
    Ok(LoadOutcome {
        settings: overlay(&Settings::default(), &overrides),
        errors,
    })
}

/// The valid values of a settings file for `scope`, plus messages for the
/// ones that were skipped.
pub fn parse_overrides(source: &str, scope: Scope) -> Result<(Overrides, Vec<String>), String> {
    let stripped = jsonc::strip(source);
    if stripped.trim().is_empty() {
        return Ok(Default::default());
    }
    let value: Value = serde_json::from_str(&stripped).map_err(|error| error.to_string())?;
    let Value::Object(file) = value else {
        return Err("expected an object of settings".to_owned());
    };
    let defaults = to_map(&Settings::default());
    let mut accepted = Overrides::new();
    let mut errors = Vec::new();
    for (key, value) in file {
        let Some(default) = defaults.get(&key) else {
            errors.push(format!("unknown setting \"{key}\""));
            continue;
        };
        if scope == Scope::Project && !PROJECT_KEYS.contains(&key.as_str()) {
            errors.push(format!(
                "\"{key}\" can only be set in the user settings.json"
            ));
            continue;
        }
        if let (Value::Object(default_fields), Value::Object(fields)) = (default, &value) {
            for (field, value) in fields {
                let name = format!("{key}.{field}");
                if !default_fields.contains_key(field) {
                    errors.push(format!("unknown setting \"{name}\""));
                    continue;
                }
                let mut candidate = accepted.clone();
                let nested = candidate
                    .entry(key.clone())
                    .or_insert_with(|| Value::Object(Map::new()));
                nested[field] = value.clone();
                accept(
                    &defaults,
                    &mut accepted,
                    candidate,
                    &name,
                    value,
                    &mut errors,
                );
            }
            continue;
        }
        let mut candidate = accepted.clone();
        candidate.insert(key.clone(), value.clone());
        accept(
            &defaults,
            &mut accepted,
            candidate,
            &key,
            &value,
            &mut errors,
        );
    }
    Ok((accepted, errors))
}

/// Keep `candidate` when it still deserializes, otherwise report why.
fn accept(
    defaults: &Map<String, Value>,
    accepted: &mut Overrides,
    candidate: Overrides,
    name: &str,
    value: &Value,
    errors: &mut Vec<String>,
) {
    // Unknown theme ids deserialize as the default, so check them here.
    if name == "theme"
        && let Some(id) = value.as_str()
        && ThemeId::parse(id).is_none()
    {
        errors.push(format!("theme: unknown theme \"{id}\""));
        return;
    }
    let mut merged = defaults.clone();
    overlay_map(&mut merged, &candidate);
    match serde_json::from_value::<Settings>(Value::Object(merged)) {
        Ok(_) => *accepted = candidate,
        Err(error) => errors.push(format!("{name}: {error}")),
    }
}

/// `base` with `overrides` applied on top.
pub fn overlay(base: &Settings, overrides: &Overrides) -> Settings {
    if overrides.is_empty() {
        return base.clone();
    }
    let mut merged = to_map(base);
    overlay_map(&mut merged, overrides);
    serde_json::from_value(Value::Object(merged)).unwrap_or_else(|_| base.clone())
}

fn overlay_map(base: &mut Map<String, Value>, overrides: &Overrides) {
    for (key, value) in overrides {
        match (base.get_mut(key), value) {
            (Some(Value::Object(fields)), Value::Object(overrides)) => {
                for (field, value) in overrides {
                    fields.insert(field.clone(), value.clone());
                }
            }
            _ => {
                base.insert(key.clone(), value.clone());
            }
        }
    }
}

fn to_map(settings: &Settings) -> Map<String, Value> {
    match serde_json::to_value(settings) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

/// Edit `text` so it parses to `new` instead of `current`, touching only
/// the members that changed. A value set back to its default is removed.
fn patch(text: &str, current: &Settings, new: &Settings) -> Option<String> {
    let defaults = to_map(&Settings::default());
    let current = to_map(current);
    let mut text = text.to_owned();
    for (key, value) in to_map(new) {
        let (Some(old), Some(default)) = (current.get(&key), defaults.get(&key)) else {
            continue;
        };
        if *old == value {
            continue;
        }
        match (&value, default, old) {
            (Value::Object(fields), Value::Object(default_fields), Value::Object(old_fields)) => {
                if fields == default_fields {
                    text = jsonc::set(&text, &[&key], None)?;
                    continue;
                }
                for (field, value) in fields {
                    if old_fields.get(field) == Some(value) {
                        continue;
                    }
                    let stored = (default_fields.get(field) != Some(value)).then_some(value);
                    text = jsonc::set(&text, &[&key, field], stored)?;
                }
            }
            _ => {
                let stored = (value != *default).then_some(&value);
                text = jsonc::set(&text, &[&key], stored)?;
            }
        }
    }
    Some(text)
}

/// Contents of a new `settings.json`: a header listing every setting with
/// its default, then an empty object.
pub fn template() -> String {
    let defaults = serde_json::to_string_pretty(&Settings::default()).expect("settings serialize");
    let mut text = String::from(
        "// Loom settings. Only values that differ from the defaults belong here;\n\
         // the Settings page writes its changes to this file as well.\n\
         //\n\
         // Every setting with its default value:\n",
    );
    for line in defaults.lines() {
        text.push_str("//   ");
        text.push_str(line);
        text.push('\n');
    }
    text.push_str("{\n}\n");
    text
}

pub fn read() -> UserSettings {
    let _guard = settings_lock().lock().expect("settings lock poisoned");
    let Some(path) = config_path(FILE_NAME) else {
        return UserSettings::Missing;
    };
    match fs::read_to_string(&path) {
        Ok(contents) => match parse(&contents) {
            Ok(outcome) => UserSettings::Loaded(outcome),
            Err(error) => UserSettings::Invalid(error),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => UserSettings::Missing,
        Err(error) => UserSettings::Invalid(error.to_string()),
    }
}

/// The settings to start with; problems are reported by the first reload.
pub fn load() -> Settings {
    match read() {
        UserSettings::Loaded(outcome) => outcome.settings,
        UserSettings::Missing | UserSettings::Invalid(_) => Settings::default(),
    }
}

/// Write the members of `settings` that differ from what the file holds.
/// A file that does not parse is left alone so a half-finished hand edit
/// is never overwritten.
pub fn save(settings: &Settings) -> io::Result<()> {
    let _guard = settings_lock().lock().expect("settings lock poisoned");
    let path = config_path(FILE_NAME).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "could not determine the user configuration directory",
        )
    })?;
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if *settings == Settings::default() {
                return Ok(());
            }
            template()
        }
        Err(error) => return Err(error),
    };
    let invalid = |error: String| io::Error::new(io::ErrorKind::InvalidData, error);
    let current = parse(&text).map_err(invalid)?.settings;
    let patched = patch(&text, &current, settings)
        .ok_or_else(|| invalid("expected an object of settings".to_owned()))?;
    if patched == text && path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, &patched)?;
    *last_written().lock().expect("settings lock poisoned") = Some(patched);
    Ok(())
}

/// Whether `text` is exactly what `save` last wrote, so the watcher can
/// skip reloading the app's own writes.
pub fn is_own_write(text: &str) -> bool {
    last_written()
        .lock()
        .expect("settings lock poisoned")
        .as_deref()
        == Some(text)
}

/// Create `settings.json` from the template if it does not exist yet.
pub fn ensure_user_file() -> io::Result<std::path::PathBuf> {
    let _guard = settings_lock().lock().expect("settings lock poisoned");
    let path = config_path(FILE_NAME).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "could not determine the user configuration directory",
        )
    })?;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, template())?;
    }
    Ok(path)
}

fn last_written() -> &'static Mutex<Option<String>> {
    static LAST: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    LAST.get_or_init(Default::default)
}

fn settings_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_as_json() {
        let settings = Settings {
            theme: ThemeId::parse("ember").unwrap(),
            default_shell: ShellKind::Bash,
            terminal_profiles: vec![crate::features::terminal::profiles::ShellProfile {
                id: 7,
                name: "My shell".into(),
                program: "/custom/fish".into(),
                args: vec!["--login".into()],
            }],
            terminal_cursor_blink: false,
            smooth_caret: false,
            git_inline_blame: true,
            git_split_diff: true,
            git_tree_view: true,
            vim: VimSettings {
                enabled: true,
                smart_case: true,
                ..VimSettings::default()
            },
            editor_font: FontSettings {
                family: "Fira Code".into(),
                size: 14,
                line_height: 150,
            },
            terminal_font: FontSettings {
                size: 13,
                ..FontSettings::TERMINAL
            },
        };

        let json = serde_json::to_string(&settings).unwrap();
        let decoded: Settings = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, settings);
    }

    #[test]
    fn missing_fields_use_defaults() {
        let decoded: Settings = serde_json::from_str("{}").unwrap();

        assert_eq!(decoded, Settings::default());
    }

    #[test]
    fn custom_default_keeps_its_id_across_settings_and_workspace_json() {
        let settings = Settings {
            default_shell: ShellKind::Custom(7),
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
        let session = crate::workspace_persistence::WorkspaceSession {
            terminal_tabs: vec![
                ShellKind::PowerShell,
                ShellKind::System,
                ShellKind::Custom(7),
            ],
            ..Default::default()
        };
        assert_eq!(
            serde_json::from_str::<crate::workspace_persistence::WorkspaceSession>(
                &serde_json::to_string(&session).unwrap()
            )
            .unwrap(),
            session
        );
    }
}

#[cfg(test)]
mod file_tests {
    use super::*;

    #[test]
    fn a_partial_file_overrides_only_its_values() {
        let outcome = parse(
            r#"// mine
            {
                "theme": "ember",
                "terminal_font": { "size": 14 },
                "vim": { "enabled": true, },
            }"#,
        )
        .unwrap();
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        let settings = outcome.settings;
        assert_eq!(settings.theme, ThemeId::parse("ember").unwrap());
        assert_eq!(settings.terminal_font.size, 14);
        // Unlisted nested fields keep their own defaults, not the editor's.
        assert_eq!(
            settings.terminal_font.line_height,
            FontSettings::TERMINAL.line_height
        );
        assert!(settings.vim.enabled);
        assert!(settings.smooth_caret);
    }

    #[test]
    fn bad_values_are_reported_without_resetting_the_rest() {
        let outcome = parse(
            r#"{
                "smooth_caret": false,
                "theme": "no-such-theme",
                "git_tree_view": "yes",
                "thme": "ember",
                "editor_font": { "size": "big", "line_height": 150, "weight": 7 }
            }"#,
        )
        .unwrap();
        let settings = outcome.settings;
        assert!(!settings.smooth_caret);
        assert_eq!(settings.theme, ThemeId::default());
        assert!(!settings.git_tree_view);
        assert_eq!(settings.editor_font.size, FontSettings::EDITOR.size);
        assert_eq!(settings.editor_font.line_height, 150);
        assert_eq!(outcome.errors.len(), 5, "{:?}", outcome.errors);
        assert!(outcome.errors.iter().any(|e| e.contains("\"thme\"")));
        assert!(
            outcome
                .errors
                .iter()
                .any(|e| e.contains("editor_font.weight"))
        );
        assert!(
            outcome
                .errors
                .iter()
                .any(|e| e.starts_with("editor_font.size:"))
        );
    }

    #[test]
    fn unusable_files_are_errors() {
        assert!(parse("{ \"theme\": ").is_err());
        assert!(parse("[]").is_err());
        assert_eq!(
            parse("// nothing yet\n").unwrap().settings,
            Settings::default()
        );
    }

    #[test]
    fn patch_writes_only_changed_members_and_drops_defaults() {
        let text = "{\n  // keep me\n  \"theme\": \"ember\",\n  \"smooth_caret\": false\n}\n";
        let current = parse(text).unwrap().settings;

        let mut new = current.clone();
        new.git_tree_view = true;
        new.editor_font.size = 14;
        let patched = patch(text, &current, &new).unwrap();
        assert_eq!(
            patched,
            "{\n  // keep me\n  \"theme\": \"ember\",\n  \"smooth_caret\": false,\n  \"git_tree_view\": true,\n  \"editor_font\": {\n    \"size\": 14\n  }\n}\n"
        );
        assert_eq!(parse(&patched).unwrap().settings, new);

        let mut reset = new.clone();
        reset.smooth_caret = true;
        reset.editor_font = FontSettings::EDITOR;
        let patched = patch(&patched, &new, &reset).unwrap();
        assert_eq!(
            patched,
            "{\n  // keep me\n  \"theme\": \"ember\",\n  \"git_tree_view\": true\n}\n"
        );
        assert_eq!(parse(&patched).unwrap().settings, reset);
    }

    #[test]
    fn template_parses_to_the_defaults_and_accepts_patches() {
        let template = template();
        assert_eq!(parse(&template).unwrap().settings, Settings::default());
        let new = Settings {
            vim: VimSettings {
                enabled: true,
                ..VimSettings::default()
            },
            ..Settings::default()
        };
        let patched = patch(&template, &Settings::default(), &new).unwrap();
        assert!(patched.starts_with(&template[..template.len() - "{\n}\n".len()]));
        assert!(patched.ends_with("{\n  \"vim\": {\n    \"enabled\": true\n  }\n}\n"));
        assert_eq!(parse(&patched).unwrap().settings, new);
    }
}
