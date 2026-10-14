//! Application/project configuration integration regression.
use crate::features::settings::rules::Settings;
use crate::features::terminal::ShellKind;
use crate::infrastructure::project_settings::*;
use std::fs;
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

    let mut app = crate::app::state::AppState::new();
    app.explorer
        .model
        .restore(vec![with.clone(), without.clone()], Vec::new());
    app.refresh_project_settings();
    assert_eq!(app.preferences.values().project_settings.len(), 1);
    assert_eq!(
        app.preferences.values().project_settings[0].1.errors.len(),
        1
    );
    assert!(app.chrome.toast.is_some(), "the ignored theme is reported");

    let inside = app.settings_at(Some(&with.join("src")));
    assert_eq!(inside.default_shell, ShellKind::Cmd);
    assert_eq!(
        inside.theme,
        app.preferences.values().theme,
        "personal settings stay the user's"
    );
    assert_eq!(
        app.settings_at(Some(&without)).default_shell,
        app.preferences.values().default_shell
    );
    assert_eq!(
        app.settings_at(None).default_shell,
        app.preferences.values().default_shell
    );

    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn custom_default_keeps_its_id_across_settings_and_workspace_json() {
    let settings = Settings {
        default_shell: ShellKind::Custom(7),
        ..Settings::default()
    };
    let json = serde_json::to_string(&settings).unwrap();
    assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
    let session = crate::app::workspace_session::WorkspaceSession {
        terminal_tabs: vec![
            ShellKind::PowerShell,
            ShellKind::System,
            ShellKind::Custom(7),
        ],
        ..Default::default()
    };
    assert_eq!(
        serde_json::from_str::<crate::app::workspace_session::WorkspaceSession>(
            &serde_json::to_string(&session).unwrap()
        )
        .unwrap(),
        session
    );
}
