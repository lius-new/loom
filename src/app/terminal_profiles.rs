//! Profile validation, discovery and configuration commands.
use crate::app::context::AppContext;
use crate::app::state::AppState;
use crate::features::terminal::ShellKind;
use crate::features::terminal::profiles::{ShellProfile, format_arguments, parse_arguments};
use crate::infrastructure::terminal::profiles::resolve_program;
use crate::ui::components::input::InputState;
use crate::ui::terminal::state::ProfileEditor;
use std::path::PathBuf;
pub fn edit(app: &mut AppState, id: Option<u64>) {
    let profile = id.and_then(|id| {
        app.preferences
            .values()
            .terminal_profiles
            .iter()
            .find(|p| p.id == id)
    });
    app.terminal.ui.shell_editor = Some(ProfileEditor {
        open: true,
        id,
        name: InputState::new(profile.map(|p| p.name.clone()).unwrap_or_default()),
        program: InputState::new(
            profile
                .map(|p| crate::ui::display_path(&p.program))
                .unwrap_or_default(),
        ),
        args: InputState::new(
            profile
                .map(|p| format_arguments(&p.args))
                .unwrap_or_default(),
        ),
        error: None,
    });
}

pub fn save_editor(app: &mut AppState) -> bool {
    let Some(editor) = app.terminal.ui.shell_editor.as_ref() else {
        return false;
    };
    let validate = || -> Result<ShellProfile, String> {
        let name = editor.name.text().trim();
        if name.is_empty() {
            return Err("Enter a name for this terminal.".into());
        }
        let raw = editor.program.text().trim();
        let raw = raw
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(raw);
        let program = resolve_program(&PathBuf::from(raw))
            .ok_or("Choose an available executable or enter its command name.")?;
        let program = program
            .canonicalize()
            .map(|p| PathBuf::from(crate::ui::display_path(&p)))
            .unwrap_or(program);
        let args = parse_arguments(editor.args.text())?;
        let id = editor.id.unwrap_or_else(|| {
            let clock = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;
            clock.max(
                app.preferences
                    .values()
                    .terminal_profiles
                    .iter()
                    .map(|p| p.id)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1),
            )
        });
        Ok(ShellProfile {
            id,
            name: name.into(),
            program,
            args,
        })
    };
    match validate() {
        Ok(profile) => {
            if let Some(old) = app
                .preferences
                .values_mut()
                .terminal_profiles
                .iter_mut()
                .find(|p| p.id == profile.id)
            {
                *old = profile;
            } else {
                app.preferences.values_mut().terminal_profiles.push(profile);
            }
            close_editor(app);
            app.refresh_shells();
            true
        }
        Err(error) => {
            app.terminal.ui.shell_editor.as_mut().unwrap().error = Some(error);
            false
        }
    }
}

pub fn remove(app: &mut AppState, id: u64) {
    app.preferences
        .values_mut()
        .terminal_profiles
        .retain(|p| p.id != id);
    // A deleted profile cannot be repaired, unlike an unavailable one.
    if app.preferences.values().default_shell == ShellKind::Custom(id) {
        app.preferences.values_mut().default_shell = ShellKind::default();
    }
    if app
        .terminal
        .ui
        .shell_editor
        .as_ref()
        .is_some_and(|editor| editor.id == Some(id))
    {
        close_editor(app);
    }
    app.refresh_shells();
}

pub(crate) fn close_editor(app: &mut AppState) {
    // Blur handlers of the unmounted inputs still reference this draft.
    if let Some(editor) = &mut app.terminal.ui.shell_editor {
        editor.open = false;
    }
}
/// The current project's folder name and the shell its project settings
/// choose, when they choose one.
pub(crate) fn project_shell(app: &AppState) -> Option<(String, String)> {
    let root = crate::app::key_actions::project_settings_root(app)?;
    let (_, project) = app
        .preferences
        .values()
        .project_settings
        .iter()
        .find(|(r, _)| *r == root)?;
    project.overrides.get("default_shell")?;
    let shell = app.settings_at(Some(&root)).default_shell;
    let folder = root.file_name().map_or_else(
        || root.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    Some((folder, app.shell_name(shell)))
}
/// Re-discovers shells. The saved default is never rewritten here, so a
/// repaired profile becomes the default again once it is available.
pub(crate) fn refresh_shells(app: &mut AppState) {
    app.preferences.values_mut().shell_options = crate::infrastructure::terminal::profiles::options(
        &app.preferences.values().terminal_profiles,
    );
    let fallback =
        Some(app.effective_shell()).filter(|kind| *kind != app.preferences.values().default_shell);
    if fallback != app.terminal.availability.fallback()
        && let Some(fallback) = fallback
    {
        let name = app.shell_name(fallback);
        app.show_toast(format!(
            "The default terminal is unavailable; using {name}."
        ));
    }
    app.terminal.availability.update(fallback);
}

pub(crate) fn settings_frame(app: &AppState) -> crate::ui::terminal::settings_binding::Frame {
    crate::ui::terminal::settings_binding::Frame {
        default_shell: app.preferences.values().default_shell,
        effective_shell: app.effective_shell(),
        shell_options: app.preferences.values().shell_options.clone(),
        project_shell: project_shell(app),
    }
}
pub(crate) fn binding(state: &AppContext) -> crate::ui::terminal::settings_binding::Binding {
    use crate::ui::terminal::settings_binding::{Binding, Command};
    let reader = state.clone();
    let writer = state.clone();
    Binding::new(
        crate::ui::components::state_access::StateAccess::project(
            state.model(),
            |app| &app.terminal.ui,
            |app| &mut app.terminal.ui,
        ),
        move || settings_frame(&reader.get()),
        move |command| {
            writer.update(|app| match command {
                Command::Edit(id) => edit(app, id),
                Command::Save => {
                    save_editor(app);
                }
                Command::Remove(id) => remove(app, id),
                Command::Close => close_editor(app),
                Command::Select(kind) => {
                    if app
                        .preferences
                        .values()
                        .shell_options
                        .iter()
                        .any(|o| o.kind == kind && o.available())
                    {
                        app.preferences.values_mut().default_shell = kind;
                    }
                }
                Command::Focus => app.terminal.ui.terminal_focused = false,
            })
        },
    )
}
