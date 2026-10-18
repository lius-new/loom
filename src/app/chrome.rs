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
            StatusCommand::ToggleSourceControl => writer.update(|app| {
                app.apply(crate::features::keymap::action::Action::ToggleSourceControl)
            }),
        },
    )
}
/// The language server's state and the active file's diagnostic counts.
fn language_notes(
    state: &AppContext,
    app: &crate::app::state::AppState,
) -> Vec<crate::ui::chrome::StatusNote> {
    use crate::features::language_service::diagnostics::Severity;
    use crate::infrastructure::lsp::manager::Status;
    use crate::ui::chrome::{NoteLevel, StatusNote};
    let mut notes = Vec::new();
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
    let Some((definition, _)) = registry.server_for(document.meta.lang) else {
        return notes;
    };
    let name = definition.display_name.to_string();
    let infos = services.infos();
    let info = infos
        .iter()
        .filter(|i| i.server == name && document.meta.path.starts_with(&i.root))
        .max_by_key(|i| i.root.components().count());
    let Some(info) = info else {
        return notes;
    };
    let (text, level) = match &info.status {
        Status::Starting => (format!("{name}: starting…"), NoteLevel::Quiet),
        Status::Ready => (name.clone(), NoteLevel::Quiet),
        Status::Restarting { .. } => (format!("{name}: restarting…"), NoteLevel::Warning),
        Status::Failed(_) => (format!("{name}: stopped"), NoteLevel::Error),
        Status::Unavailable(_) => (format!("{name}: not found"), NoteLevel::Quiet),
    };
    notes.push(StatusNote { text, level });
    if let Some(found) = services
        .results(&document.meta.path, &document.stamp(id))
        .diagnostics
    {
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
    }
    notes
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
