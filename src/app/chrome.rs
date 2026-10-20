//! Chrome composition: views receive only their relevant frames and commands.
use crate::app::context::AppContext;
use crate::app::terminal_binding::TerminalStore;
use crate::ui::chrome::{HomeBinding, HomeCommand, HomeFrame, TitleBinding, TitleFrame};
use lgui::core::{UiFocusHandle, UiId};
use lgui::prelude::{Element, UiRect};
pub fn title_binding(state: &AppContext) -> TitleBinding {
    let reader = state.clone();
    let writer = state.clone();
    let updater = state.clone();
    TitleBinding::new(
        move || {
            let app = reader.get();
            TitleFrame {
                settings_open: app.workspace.active_is_settings(),
                maximized: crate::app::window::is_maximized(),
                update: app.update.indicator(),
            }
        },
        move || {
            writer.update(|app| app.apply(crate::features::keymap::action::Action::OpenSettings))
        },
        move || crate::app::update::activate(&updater),
    )
}
pub fn title(rect: UiRect, state: AppContext, git: Element) -> Element {
    crate::ui::titlebar::render(rect, title_binding(&state), git)
}
pub fn home_binding(
    state: &AppContext,
    workspace: bool,
    editor: UiFocusHandle,
    terminal: Option<UiFocusHandle>,
    tabs: Option<TerminalStore>,
) -> HomeBinding {
    let reader = state.clone();
    let writer = state.clone();
    HomeBinding::new(
        move || {
            let app = reader.get();
            HomeFrame {
                paths: if workspace {
                    app.explorer.model.roots().to_vec()
                } else {
                    app.explorer.model.recents().to_vec()
                },
                hover: if workspace {
                    app.chrome.workspace_home_hover
                } else {
                    app.chrome.welcome_hover
                },
            }
        },
        move |command| match command {
            HomeCommand::Hover(value) => writer.try_update(|app| {
                let hover = if workspace {
                    &mut app.chrome.workspace_home_hover
                } else {
                    &mut app.chrome.welcome_hover
                };
                let changed = *hover != value;
                *hover = value;
                changed
            }),
            HomeCommand::Focus(focused) => {
                writer.update(|app| {
                    app.editor.focused = focused;
                    if !focused {
                        app.editor.drag = None;
                        app.editor.ime = None;
                        if let Some(mut buffer) = app.workspace.active_editor_mut() {
                            buffer.break_undo_group();
                        }
                    }
                });
                true
            }
            HomeCommand::OpenFile => {
                let opened = crate::app::workspace_commands::choose_file(&writer);
                if opened && workspace {
                    editor.focus();
                }
                opened
            }
            HomeCommand::OpenFolder => {
                crate::app::explorer::choose_folder(&writer);
                true
            }
            HomeCommand::OpenRecent(path) => {
                crate::app::explorer::open_recent_folder(&writer, path);
                true
            }
            HomeCommand::ToggleTerminal => {
                if let (Some(tabs), Some(terminal)) = (&tabs, &terminal) {
                    crate::app::terminal::toggle_panel(&writer, tabs, &editor, terminal);
                    true
                } else {
                    false
                }
            }
            HomeCommand::OpenSourceControl => {
                writer.update(|app| {
                    app.source_control.visible = true;
                    app.chrome.workspace_home_hover = None;
                });
                true
            }
        },
    )
}
pub fn welcome(rect: UiRect, state: AppContext, id: UiId, editor: UiFocusHandle) -> Element {
    crate::ui::welcome::render(
        rect,
        home_binding(&state, false, editor.clone(), None, None),
        id,
        editor,
    )
}
pub fn workspace_home(
    rect: UiRect,
    state: AppContext,
    id: UiId,
    editor: UiFocusHandle,
    terminal: UiFocusHandle,
    tabs: TerminalStore,
) -> Element {
    crate::ui::workspace_home::render(
        rect,
        home_binding(&state, true, editor, Some(terminal), Some(tabs)),
        id,
    )
}

pub fn status_binding(
    state: &AppContext,
    editor: UiFocusHandle,
    terminal: UiFocusHandle,
    tabs: TerminalStore,
) -> crate::ui::chrome::StatusBinding {
    use crate::ui::chrome::{FileStatus, StatusBinding, StatusCommand, StatusFrame};
    let reader = state.clone();
    let writer = state.clone();
    StatusBinding::new(
        move || {
            let app = reader.get();
            StatusFrame {
                terminal_open: app.default_terminal_open(),
                pending: app.keymap.pending().to_vec(),
                source_control: app.source_control.visible,
                diff_loading: app.git.model.diff_loading().is_some(),
                language: language_notes(&reader, &app),
                vim: crate::app::editor_vim::is_active(&app)
                    .then(|| (app.editor.vim.mode(), app.editor.vim.status())),
                file: app.workspace.active().map(|id| FileStatus {
                    lang: app
                        .workspace
                        .meta(id)
                        .expect("active document metadata exists")
                        .lang,
                    missing: app.workspace.is_missing_on_disk(id),
                    conflict: app.workspace.has_disk_conflict(id),
                    cursor: app
                        .workspace
                        .active_editor()
                        .map(|editor| editor.line_col()),
                    is_diff: app.workspace.is_diff(id),
                    is_file: app.workspace.is_file(id),
                }),
            }
        },
        move |command| match command {
            StatusCommand::ToggleTerminal => {
                crate::app::terminal::toggle_panel(&writer, &tabs, &editor, &terminal)
            }
            StatusCommand::LanguagePanel => writer.update(|app| {
                app.chrome.language_panel = !app.chrome.language_panel;
            }),
            StatusCommand::ToggleSourceControl => writer.update(|app| {
                app.apply(crate::features::keymap::action::Action::ToggleSourceControl)
            }),
        },
    )
}
/// Pending syntax resources, server state and the active file's diagnostics.
fn language_notes(
    state: &AppContext,
    app: &crate::app::state::AppState,
) -> Vec<crate::ui::chrome::StatusNote> {
    use crate::ui::chrome::{NoteLevel, StatusNote};
    let mut notes = Vec::new();
    if let Some(info) = crate::infrastructure::language_packs::resources()
        .download_infos()
        .into_iter()
        .find(|i| i.status != "ready")
    {
        notes.push(StatusNote {
            text: format!("{} syntax: {}", info.name, info.status),
            level: if info.error.is_some() {
                NoteLevel::Warning
            } else {
                NoteLevel::Quiet
            },
        });
    }
    let Some(id) = app.workspace.active() else {
        return notes;
    };
    let Some(services) = state.services().and_then(|r| r.language_services()) else {
        return notes;
    };
    let Some(document) = app.workspace.documents.get(&id).filter(|d| d.is_text()) else {
        return notes;
    };
    let registry = crate::domain::language::Registry::builtin();
    let servers = crate::features::language_service::server_set::ServerSet::current();
    let infos = services.infos();
    // One note for each server the language is bound to, in the servers' order.
    for (definition, _) in servers.for_language(registry, document.meta.lang) {
        let name = definition.display_name.to_string();
        let info = infos
            .iter()
            .filter(|i| i.server == name && document.meta.path.starts_with(&i.root))
            .max_by_key(|i| i.root.components().count());
        let Some(info) = info else {
            continue;
        };
        notes.push(server_note(&name, info));
    }
    if let Some(found) = services
        .results(&document.meta.path, &document.stamp(id))
        .diagnostics
    {
        notes.extend(diagnostic_notes(&found));
    }
    notes
}

/// The status-bar note for one server instance.
fn server_note(
    name: &str,
    info: &crate::infrastructure::lsp::manager::InstanceInfo,
) -> crate::ui::chrome::StatusNote {
    use crate::infrastructure::lsp::manager::Status;
    use crate::ui::chrome::{NoteLevel, StatusNote};
    let (text, level) = match &info.status {
        Status::Starting => (format!("{name}: starting…"), NoteLevel::Quiet),
        Status::Ready => match info.tasks.first() {
            // The server is connected but still working: say what on.
            Some(task) => (
                match task.percentage {
                    Some(percent) => format!("{name}: {} {percent}%", task.title),
                    None => format!("{name}: {}", task.title),
                },
                NoteLevel::Quiet,
            ),
            None => (name.to_owned(), NoteLevel::Quiet),
        },
        Status::Restarting { .. } => (format!("{name}: restarting…"), NoteLevel::Warning),
        Status::Failed(_) => (format!("{name}: stopped"), NoteLevel::Error),
        Status::Unavailable(_) => (format!("{name}: not found"), NoteLevel::Quiet),
        Status::NotInstalled => (format!("{name}: not installed"), NoteLevel::Warning),
        Status::Installing(progress) => {
            (format!("{name}: installing, {progress}"), NoteLevel::Quiet)
        }
        Status::InstallFailed(_) => (format!("{name}: install failed"), NoteLevel::Error),
    };
    StatusNote { text, level }
}

/// Error and warning counts of the active file's diagnostics.
fn diagnostic_notes(
    found: &[crate::features::language_service::diagnostics::Diagnostic],
) -> Vec<crate::ui::chrome::StatusNote> {
    use crate::features::language_service::diagnostics::Severity;
    use crate::ui::chrome::{NoteLevel, StatusNote};
    let mut notes = Vec::new();
    let count = |severity| found.iter().filter(|d| d.severity == severity).count();
    let (errors, warnings) = (count(Severity::Error), count(Severity::Warning));
    if errors > 0 {
        notes.push(StatusNote {
            text: format!("{errors} error{}", if errors == 1 { "" } else { "s" }),
            level: NoteLevel::Error,
        });
    }
    if warnings > 0 {
        notes.push(StatusNote {
            text: format!("{warnings} warning{}", if warnings == 1 { "" } else { "s" }),
            level: NoteLevel::Warning,
        });
    }
    notes
}

/// Syntax installation status and running language servers.
pub fn language_panel_frame(state: &AppContext) -> crate::ui::language_panel::Frame {
    use crate::infrastructure::lsp::manager::Status;
    use crate::ui::language_panel::{Frame, Level, Line, Section};
    let services = state.services().and_then(|r| r.language_services());
    let line = |text: String, level| Line { text, level };
    let mut sections: Vec<Section> = services
        .map(|s| s.infos())
        .unwrap_or_default()
        .into_iter()
        .map(|info| {
            let (state_text, level) = match &info.status {
                Status::Starting => ("starting".to_owned(), Level::Quiet),
                Status::Ready if info.tasks.is_empty() => ("ready".to_owned(), Level::Quiet),
                Status::Ready => ("working".to_owned(), Level::Quiet),
                Status::Restarting { attempt, .. } => {
                    (format!("restarting (attempt {attempt})"), Level::Warning)
                }
                Status::Failed(_) => ("stopped".to_owned(), Level::Error),
                Status::Unavailable(_) => ("not available".to_owned(), Level::Quiet),
                Status::NotInstalled => ("not installed".to_owned(), Level::Warning),
                Status::Installing(progress) => (format!("installing, {progress}"), Level::Quiet),
                Status::InstallFailed(_) => ("install failed".to_owned(), Level::Error),
            };
            let mut lines = vec![line(info.root.display().to_string(), Level::Quiet)];
            match &info.status {
                Status::Restarting { reason, .. }
                | Status::Failed(reason)
                | Status::InstallFailed(reason) => {
                    lines.push(line(reason.clone(), Level::Error));
                }
                Status::Unavailable(reason) => {
                    lines.push(line(reason.clone(), Level::Quiet));
                    lines.push(line(
                        "Set another program with language_servers in settings.json.".to_owned(),
                        Level::Quiet,
                    ));
                }
                _ => {}
            }
            for task in &info.tasks {
                let mut text = task.title.clone();
                if let Some(percent) = task.percentage {
                    text.push_str(&format!(" {percent}%"));
                }
                if let Some(message) = &task.message {
                    text.push_str(&format!(" — {message}"));
                }
                lines.push(line(text, Level::Quiet));
            }
            if let Some(providers) = &info.providers {
                let offered = providers.names();
                lines.push(line(
                    if offered.is_empty() {
                        "offers: nothing beyond diagnostics".to_owned()
                    } else {
                        format!("offers: {}", offered.join(", "))
                    },
                    Level::Quiet,
                ));
            }
            let counts = info.diagnostics;
            lines.push(line(
                format!(
                    "diagnostics: {} published, {} shown, {} moved, {} outdated, {} other file",
                    counts.published, counts.shown, counts.carried, counts.outdated, counts.unknown
                ),
                if counts.published > 0 && counts.shown == 0 {
                    Level::Warning
                } else {
                    Level::Quiet
                },
            ));
            lines.push(line(
                format!(
                    "{} documents{}",
                    info.documents,
                    info.process_id
                        .map(|p| format!(", pid {p}"))
                        .unwrap_or_default()
                ),
                Level::Quiet,
            ));
            let broken = matches!(info.status, Status::Restarting { .. } | Status::Failed(_));
            let recent = info.log.iter().rev().take(6).rev();
            lines.extend(recent.map(|l| line(l.clone(), Level::Quiet)));
            if broken {
                let tail = info.stderr.iter().rev().take(3).rev();
                lines.extend(tail.map(|l| line(l.clone(), Level::Warning)));
            }
            let install = match info.status {
                Status::NotInstalled => Some("Install"),
                Status::InstallFailed(_) => Some("Retry"),
                _ => None,
            };
            Section {
                title: format!("{} — {state_text}", info.server),
                level,
                lines,
                install: install.map(|label| (info.server_id.clone(), label)),
            }
        })
        .collect();
    let downloads: Vec<_> = crate::infrastructure::language_packs::resources()
        .download_infos()
        .into_iter()
        .filter(|i| i.status != "ready")
        .collect();
    let retry_downloads = downloads.iter().any(|i| i.error.is_some());
    sections.extend(downloads.into_iter().map(|info| {
        Section {
            title: format!("{} syntax — {}", info.name, info.status),
            level: if info.error.is_some() {
                Level::Warning
            } else {
                Level::Quiet
            },
            lines: info
                .error
                .into_iter()
                .map(|error| line(error, Level::Warning))
                .collect(),
            install: None,
        }
    }));
    Frame {
        sections,
        retry_downloads,
    }
}

pub fn status(
    rect: UiRect,
    state: AppContext,
    editor: UiFocusHandle,
    terminal: UiFocusHandle,
    tabs: TerminalStore,
) -> Element {
    crate::ui::statusbar::render(rect, status_binding(&state, editor, terminal, tabs))
}
