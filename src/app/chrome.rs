//! Chrome composition: views receive only their relevant frames and commands.
use crate::app::context::AppContext;
use crate::app::terminal_binding::TerminalStore;
use crate::ui::chrome::{HomeBinding, HomeCommand, HomeFrame, TitleBinding, TitleFrame};
use lgui::core::{UiFocusHandle, UiId};
use lgui::prelude::{Element, UiRect};
pub fn title_binding(state: &AppContext) -> TitleBinding {
    let reader = state.clone();
    let writer = state.clone();
    TitleBinding::new(
        move || TitleFrame {
            settings_open: reader.get().workspace.active_is_settings(),
            maximized: crate::app::window::is_maximized(),
        },
        move || {
            writer.update(|app| app.apply(crate::features::keymap::action::Action::OpenSettings))
        },
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
pub fn status(
    rect: UiRect,
    state: AppContext,
    editor: UiFocusHandle,
    terminal: UiFocusHandle,
    tabs: TerminalStore,
) -> Element {
    crate::ui::statusbar::render(rect, status_binding(&state, editor, terminal, tabs))
}
