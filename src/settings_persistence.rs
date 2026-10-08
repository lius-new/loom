//! User preferences edited on the Settings page, stored in `settings.json`.
//!
//! Earlier versions kept these values in `workspace.json`; the first load
//! without a `settings.json` migrates them from there.

use std::fs;
use std::io;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::terminal_session::ShellKind;
use crate::theme::ThemeId;
use crate::workspace_persistence::{config_path, session_path, write_json};

const FILE_NAME: &str = "settings.json";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Settings {
    #[serde(default)]
    pub theme: ThemeId,
    #[serde(default)]
    pub default_shell: ShellKind,
    #[serde(default = "default_true")]
    pub terminal_cursor_blink: bool,
    #[serde(default)]
    pub git_inline_blame: bool,
    #[serde(default)]
    pub git_split_diff: bool,
    #[serde(default)]
    pub git_tree_view: bool,
    #[serde(default)]
    pub vim: VimSettings,
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
            terminal_cursor_blink: true,
            git_inline_blame: false,
            git_split_diff: false,
            git_tree_view: false,
            vim: VimSettings::default(),
        }
    }
}

fn default_true() -> bool {
    true
}

pub fn load() -> Settings {
    let _guard = settings_lock().lock().expect("settings lock poisoned");
    let Some(path) = config_path(FILE_NAME) else {
        return Settings::default();
    };
    if let Ok(contents) = fs::read_to_string(&path) {
        return serde_json::from_str(&contents).unwrap_or_default();
    }
    // The legacy keys in `workspace.json` use the same names, so the old
    // session parses straight into `Settings`.
    let Some(contents) = session_path().and_then(|path| fs::read_to_string(path).ok()) else {
        return Settings::default();
    };
    let settings = serde_json::from_str(&contents).unwrap_or_default();
    let _ = write_json(&path, &settings);
    settings
}

pub fn save(settings: &Settings) -> io::Result<()> {
    let _guard = settings_lock().lock().expect("settings lock poisoned");
    let path = config_path(FILE_NAME).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "could not determine the user configuration directory",
        )
    })?;
    write_json(&path, settings)
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
            terminal_cursor_blink: false,
            git_inline_blame: true,
            git_split_diff: true,
            git_tree_view: true,
            vim: VimSettings {
                enabled: true,
                smart_case: true,
                ..VimSettings::default()
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
    fn legacy_workspace_session_migrates_its_preferences() {
        let legacy = r#"{
            "open_folders": ["one"],
            "source_control_open": true,
            "git_tree_view": true,
            "git_inline_blame": true,
            "default_shell": "bash",
            "terminal_cursor_blink": false
        }"#;

        let decoded: Settings = serde_json::from_str(legacy).unwrap();

        assert_eq!(
            decoded,
            Settings {
                theme: ThemeId::default(),
                default_shell: ShellKind::Bash,
                terminal_cursor_blink: false,
                git_inline_blame: true,
                git_split_diff: false,
                git_tree_view: true,
                vim: VimSettings::default(),
            }
        );
    }
}
