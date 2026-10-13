//! Per-project settings in `<workspace folder>/.loom/settings.json`.
//!
//! The file is JSONC, like the user's `settings.json`, but may only set the
//! keys in `settings_persistence::PROJECT_KEYS`: it is committed with the
//! project and read from repositories the user did not write, so it never
//! carries personal preferences or anything that names a program to run.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::settings_persistence::{self, Overrides, PROJECT_KEYS, Scope};

pub const DIR_NAME: &str = ".loom";
pub const FILE_NAME: &str = "settings.json";

/// The project settings file of the workspace folder `root`.
pub fn path(root: &Path) -> PathBuf {
    root.join(DIR_NAME).join(FILE_NAME)
}

/// Whether `file` is some workspace folder's project settings file.
pub fn is_settings_file(file: &Path) -> bool {
    file.file_name().is_some_and(|name| name == FILE_NAME)
        && file
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == DIR_NAME)
}

/// Overrides read from one project, plus messages for skipped values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectSettings {
    pub overrides: Overrides,
    pub errors: Vec<String>,
}

/// Read `root`'s project settings; `Ok(None)` when it has none.
pub fn read(root: &Path) -> Result<Option<ProjectSettings>, String> {
    match fs::read_to_string(path(root)) {
        Ok(text) => {
            let (overrides, errors) = settings_persistence::parse_overrides(&text, Scope::Project)?;
            Ok(Some(ProjectSettings { overrides, errors }))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// Contents of a new project settings file.
pub fn template() -> String {
    let mut text = String::from(
        "// Loom project settings, shared with everyone who opens this folder.\n\
         // Values here override your user settings.json for this folder.\n\
         //\n\
         // Settings that can be set per project:\n",
    );
    let defaults = serde_json::to_value(settings_persistence::Settings::default())
        .expect("settings serialize");
    for key in PROJECT_KEYS {
        text.push_str(&format!("//   \"{key}\": {}\n", defaults[key]));
    }
    text.push_str("{\n}\n");
    text
}

/// Create `root`'s project settings file from the template if needed.
pub fn ensure(root: &Path) -> io::Result<PathBuf> {
    let path = path(root);
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, template())?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_session::ShellKind;

    #[test]
    fn only_project_keys_apply() {
        let (overrides, errors) = settings_persistence::parse_overrides(
            r#"{ "default_shell": "bash", "theme": "ember", "terminal_profiles": [] }"#,
            Scope::Project,
        )
        .unwrap();
        assert_eq!(overrides.len(), 1);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[0].contains("\"theme\" can only be set in the user settings.json"));

        let user = settings_persistence::Settings {
            smooth_caret: false,
            ..Default::default()
        };
        let merged = settings_persistence::overlay(&user, &overrides);
        assert_eq!(merged.default_shell, ShellKind::Bash);
        assert!(!merged.smooth_caret);
    }

    #[test]
    fn template_lists_project_keys_and_parses_empty() {
        let template = template();
        for key in PROJECT_KEYS {
            assert!(template.contains(&format!("\"{key}\"")));
        }
        let (overrides, errors) =
            settings_persistence::parse_overrides(&template, Scope::Project).unwrap();
        assert!(overrides.is_empty() && errors.is_empty());
    }

    #[test]
    fn workspace_folders_apply_their_own_project_settings() {
        let base = std::env::temp_dir().join(format!(
            "loom-project-settings-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let (with, without) = (base.join("with"), base.join("without"));
        fs::create_dir_all(with.join(DIR_NAME)).unwrap();
        fs::create_dir_all(&without).unwrap();
        fs::write(
            path(&with),
            "// team shell\n{ \"default_shell\": \"cmd\", \"theme\": \"ember\" }",
        )
        .unwrap();

        let mut app = crate::state::AppState::new();
        app.explorer
            .model
            .restore(vec![with.clone(), without.clone()], Vec::new());
        app.refresh_project_settings();
        assert_eq!(app.project_settings.len(), 1);
        assert_eq!(app.project_settings[0].1.errors.len(), 1);
        assert!(app.toast.is_some(), "the ignored theme is reported");

        let inside = app.settings_at(Some(&with.join("src")));
        assert_eq!(inside.default_shell, ShellKind::Cmd);
        assert_eq!(inside.theme, app.theme, "personal settings stay the user's");
        assert_eq!(
            app.settings_at(Some(&without)).default_shell,
            app.default_shell
        );
        assert_eq!(app.settings_at(None).default_shell, app.default_shell);

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn recognizes_settings_files() {
        assert!(is_settings_file(Path::new("/repo/.loom/settings.json")));
        assert!(!is_settings_file(Path::new("/repo/settings.json")));
        assert!(!is_settings_file(Path::new("/repo/.loom/keymap.json")));
    }
}
