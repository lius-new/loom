//! Terminal commands: identity, model/layout ordering and runtime dispatch.
use crate::app::context::AppContext;
use crate::app::state::AppState;
use crate::app::terminal_binding::TerminalStore;
use crate::features::terminal::actions::{MenuAction, close_targets};
use crate::features::terminal::{SessionId, ShellKind};
use crate::infrastructure::terminal::runtime::TerminalHandle;
use lgui::core::UiFocusHandle;
use lgui::services::{Clipboard, ClipboardError};
use std::path::PathBuf;

pub(crate) fn visibility(app: &AppState) -> crate::domain::application_layout::Visibility {
    crate::domain::application_layout::Visibility {
        git: app.source_control.visible,
        files: app.drawer.visible,
        terminal: app.terminal_chrome.show_terminal,
    }
}
pub(super) fn target(
    tabs: &TerminalStore,
    id: SessionId,
) -> Option<crate::app::terminal_binding::TerminalTab> {
    tabs.get().tabs().into_iter().find(|tab| tab.id == id)
}
/// Check the completed UI round, after both stores have been updated.
fn assert_ownership(app: &AppState, tabs: &crate::app::terminal_binding::TerminalTabs) {
    debug_assert!(
        tabs.active_id()
            .is_none_or(|id| tabs.tabs().iter().any(|tab| tab.id == id))
    );
    if let Some(layout) = &app.layout.application_layout {
        let owned = layout
            .leaves()
            .into_iter()
            .flat_map(|(_, content)| match content {
                crate::domain::application_layout::Content::Terminal { tabs, active } => {
                    debug_assert!(active.is_none_or(|id| tabs.contains(&id)));
                    tabs
                }
                _ => Vec::new(),
            })
            .collect::<Vec<_>>();
        let ids = tabs
            .tabs()
            .into_iter()
            .map(|tab| tab.id)
            .collect::<std::collections::HashSet<_>>();
        debug_assert_eq!(owned.len(), ids.len());
        debug_assert_eq!(
            owned.into_iter().collect::<std::collections::HashSet<_>>(),
            ids
        );
    }
}
fn sync_layout(state: &AppContext, tabs: &TerminalStore) {
    let current = tabs.get();
    state.update(|app| {
        if let Some(layout) = &mut app.layout.application_layout {
            current.sync_layout(layout);
        }
        assert_ownership(app, &current);
    });
}
pub(crate) fn new(
    state: &AppContext,
    tabs: &TerminalStore,
    shell: ShellKind,
    cwd: Option<PathBuf>,
) -> SessionId {
    let mut id = 0;
    tabs.update(|tabs| id = tabs.add_at(shell, cwd));
    state.update(|app| {
        if let Some(layout) = &mut app.layout.application_layout {
            layout.attach(id);
        }
    });
    sync_layout(state, tabs);
    id
}
pub(crate) fn new_from_target(
    state: &AppContext,
    tabs: &TerminalStore,
    id: SessionId,
    shell: Option<ShellKind>,
    workspace_fallback: bool,
) -> Option<SessionId> {
    let tab = target(tabs, id)?;
    let cwd = tab.working_directory().or_else(|| {
        workspace_fallback
            .then(|| state.get().explorer.model.roots().first().cloned())
            .flatten()
    });
    Some(new(
        state,
        tabs,
        shell.unwrap_or_else(|| tab.controller.shell()),
        cwd,
    ))
}
pub(crate) fn select(state: &AppContext, tabs: &TerminalStore, id: SessionId) -> bool {
    if target(tabs, id).is_none() {
        return false;
    }
    tabs.update(|tabs| tabs.select(id));
    sync_layout(state, tabs);
    true
}
pub(crate) fn split(state: &AppContext, tabs: &TerminalStore, id: SessionId) -> Option<SessionId> {
    target(tabs, id)?;
    let mut created = None;
    tabs.update(|tabs| created = tabs.split(id));
    if let Some(created) = created {
        state.update(|app| {
            if let Some(layout) = &mut app.layout.application_layout {
                layout.attach(created);
            }
        });
    }
    sync_layout(state, tabs);
    created
}
/// Model edit, then layout prune, then process disposal. State locks never nest.
pub(crate) fn close(
    state: &AppContext,
    tabs: &TerminalStore,
    id: SessionId,
    action: MenuAction,
    application: &std::sync::Arc<lgui::ApplicationHandle>,
) -> bool {
    if target(tabs, id).is_none() {
        return false;
    }
    let ids = close_targets(tabs.get().model(), id, action);
    let mut handles = Vec::new();
    tabs.update(|tabs| {
        for id in ids {
            if let Some(handle) = tabs.close(id) {
                handles.push(handle);
            }
        }
    });
    sync_layout(state, tabs);
    for handle in handles {
        handle.terminate(application);
    }
    if tabs.get().is_empty() {
        state.update(|app| {
            app.terminal_chrome.show_terminal = false;
            app.terminal.ui.terminal_shell_menu = false;
            app.terminal.ui.terminal_focused = false;
        });
    }
    true
}
pub(crate) fn menu_action(
    state: &AppContext,
    tabs: &TerminalStore,
    id: SessionId,
    action: MenuAction,
    application: &std::sync::Arc<lgui::ApplicationHandle>,
) -> bool {
    state.update(|app| app.terminal.ui.terminal_tab_context_menu = None);
    if target(tabs, id).is_none() {
        return false;
    }
    match action {
        MenuAction::New => new_from_target(state, tabs, id, None, true).is_some(),
        MenuAction::Split => split(state, tabs, id).is_some(),
        _ => close(state, tabs, id, action, application),
    }
}
pub(crate) fn purpose(
    state: &AppContext,
    tabs: &TerminalStore,
    id: SessionId,
    purpose: Option<&str>,
) {
    if target(tabs, id).is_some() {
        tabs.update(|tabs| tabs.set_purpose(id, purpose));
    }
    state.update(|app| app.terminal.ui.terminal_tab_context_menu = None);
}
pub(crate) fn resize_pair(tabs: &TerminalStore, left: SessionId, right: SessionId, fraction: f32) {
    if target(tabs, left).is_some() && target(tabs, right).is_some() {
        tabs.update(|tabs| tabs.resize_pair(left, right, fraction));
    }
}
/// Toggle only the default terminal region. Detached sessions retain their
/// visibility and focus when that region is hidden.
pub fn toggle_panel(
    state: &AppContext,
    terminal_tabs: &crate::app::terminal_binding::TerminalStore,
    editor_focus: &UiFocusHandle,
    terminal_focus: &UiFocusHandle,
) {
    let app = state.get();
    let opening = !app.default_terminal_open();
    let default_session = app
        .layout
        .application_layout
        .as_ref()
        .and_then(|layout| layout.default_terminal_active())
        .or_else(|| {
            app.layout
                .application_layout
                .is_none()
                .then(|| terminal_tabs.get().active_id())
                .flatten()
        });
    let mut selected = default_session;
    if opening && selected.is_none() {
        let cwd = app.explorer.model.roots().first().cloned();
        let shell = app.effective_shell_at(cwd.as_deref());
        terminal_tabs.update(|tabs| {
            selected = Some(tabs.add_at(shell, cwd));
        });
    }
    if opening && let Some(id) = selected {
        terminal_tabs.update(|tabs| tabs.select(id));
    }
    let active = terminal_tabs.get().active_id();
    let mut active_visible = false;
    state.update(|app| {
        if opening
            && let Some(id) = selected
            && let Some(layout) = &mut app.layout.application_layout
        {
            layout.attach(id);
        }
        app.terminal_chrome.show_terminal = opening;
        let visible = visibility(app);
        active_visible = active.is_some_and(|id| {
            app.layout
                .application_layout
                .as_ref()
                .map_or(opening, |layout| layout.terminal_visible(id, visible))
        });
        let hidden_menu = app
            .terminal
            .ui
            .terminal_tab_context_menu
            .as_ref()
            .is_some_and(|menu| {
                app.layout
                    .application_layout
                    .as_ref()
                    .is_some_and(|layout| !layout.terminal_visible(menu.target, visible))
            });
        if opening || !active_visible || hidden_menu {
            app.terminal.ui.terminal_tab_context_menu = None;
            app.terminal.ui.terminal_shell_menu = false;
        }
    });
    sync_layout(state, terminal_tabs);
    if opening {
        terminal_focus.focus();
    } else if !active_visible {
        editor_focus.focus();
    }
}
/// The terminal's clipboard actions (`terminal::Copy` and friends).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardShortcut {
    Copy,
    /// Copy when text is selected and otherwise interrupt, matching Windows Terminal.
    CopyOrInterrupt,
    Paste,
    CopyOrPaste,
}

fn run_clipboard_shortcut(
    controller: &TerminalHandle,
    shortcut: ClipboardShortcut,
    clipboard: &dyn Clipboard,
) -> Result<(), ClipboardError> {
    match shortcut {
        ClipboardShortcut::Copy => copy_selection(controller, clipboard).map(|_| ()),
        ClipboardShortcut::CopyOrInterrupt => copy_selection(controller, clipboard).map(|copied| {
            if !copied {
                controller.write(b"\x03");
            }
        }),
        ClipboardShortcut::CopyOrPaste => match copy_selection(controller, clipboard)? {
            true => Ok(()),
            false => paste_clipboard(controller, clipboard),
        },
        ClipboardShortcut::Paste => paste_clipboard(controller, clipboard),
    }
}

fn copy_selection(
    controller: &TerminalHandle,
    clipboard: &dyn Clipboard,
) -> Result<bool, ClipboardError> {
    let Some(value) = controller.selected_text() else {
        return Ok(false);
    };
    // A failed copy keeps the selection so the user can retry.
    clipboard.write_text(&value)?;
    controller.clear_selection();
    Ok(true)
}

fn paste_clipboard(
    controller: &TerminalHandle,
    clipboard: &dyn Clipboard,
) -> Result<(), ClipboardError> {
    if let Some(value) = clipboard.read_text()? {
        controller.paste(&value);
    }
    Ok(())
}

pub fn report_clipboard_error(state: &AppContext, result: Result<(), ClipboardError>) {
    if let Err(error) = result {
        state.update(|app| app.show_error(format!("Clipboard: {error}")));
    }
}

pub(crate) fn write(tabs: &TerminalStore, id: SessionId, bytes: &[u8]) -> bool {
    let Some(tab) = target(tabs, id) else {
        return false;
    };
    tab.controller.write(bytes);
    true
}
pub(crate) fn clipboard(
    tabs: &TerminalStore,
    id: SessionId,
    shortcut: ClipboardShortcut,
    clipboard: &dyn Clipboard,
) -> Result<(), ClipboardError> {
    let Some(tab) = target(tabs, id) else {
        return Ok(());
    };
    run_clipboard_shortcut(&tab.controller, shortcut, clipboard)
}
pub(crate) fn mouse_position(tabs: &TerminalStore, id: SessionId, row: u16, col: u16) {
    if let Some(tab) = target(tabs, id) {
        tab.controller.set_mouse_position(row, col);
    }
}
pub(crate) fn begin_selection(tabs: &TerminalStore, id: SessionId, row: u16, col: u16) {
    if let Some(tab) = target(tabs, id) {
        tab.controller.begin_selection(row, col);
    }
}
pub(crate) fn extend_selection(tabs: &TerminalStore, id: SessionId, row: u16, col: u16) {
    if let Some(tab) = target(tabs, id) {
        tab.controller.extend_selection(row, col);
    }
}
pub(crate) fn wheel(
    tabs: &TerminalStore,
    id: SessionId,
    delta: f32,
    modifiers: crate::features::terminal::input::Modifiers,
) {
    if let Some(tab) = target(tabs, id) {
        tab.controller.wheel(delta, modifiers);
    }
}

pub(crate) fn shell_menu(state: &AppContext, tabs: &TerminalStore, id: SessionId) -> bool {
    if !select(state, tabs, id) {
        return false;
    }
    state.update(|app| {
        app.terminal.ui.terminal_shell_menu = !app.terminal.ui.terminal_shell_menu;
        if app.terminal.ui.terminal_shell_menu {
            app.terminal.ui.terminal_shell_scroll = 0;
            app.refresh_shells();
        }
    });
    true
}
