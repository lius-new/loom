//! Composes terminal frames and semantic UI commands without exposing resources.
use crate::app::context::AppContext;
use crate::app::state::AppState;
use crate::app::terminal_binding::{TerminalStore, TerminalTabs};
use crate::app::terminal_runtime::TerminalDisplay;
use crate::features::terminal::SessionId;
use crate::infrastructure::terminal::runtime::TerminalHandle;
use crate::ui::terminal::{
    bindings::{self, Binding, Command, FrameRequest, Outcome, SessionFrame, Sessions, View},
    view,
};
use lgui::ApplicationHandle;
use lgui::core::{UiFocusHandle, UiId};
use lgui::prelude::{Element, UiRect};
use std::sync::Arc;

fn session_frame(tab: crate::app::terminal_binding::TerminalTab) -> SessionFrame {
    let short_label = tab.controller.short_label();
    let title = match tab.purpose.as_ref().or(tab.running_program.as_ref()) {
        Some(name) => format!("{short_label} · {name}"),
        None => short_label.clone(),
    };
    SessionFrame {
        id: tab.id,
        group_id: tab.group_id,
        width_weight: tab.width_weight,
        shell: tab.controller.shell(),
        short_label,
        title,
        cwd: tab.working_directory(),
        purpose: tab.purpose,
        running_program: tab.running_program,
    }
}
pub(crate) fn project(tabs: TerminalTabs) -> View {
    let mut live = tabs.frame_metadata();
    let values = tabs
        .model()
        .tabs()
        .iter()
        .map(|tab| {
            let (short_label, cwd) = live
                .remove(&tab.id())
                .unwrap_or_else(|| (tab.shell().short_label().to_owned(), None));
            let purpose = tab.purpose();
            let running_program = tab.running_program();
            let title = match purpose.as_ref().or(running_program.as_ref()) {
                Some(name) => format!("{short_label} · {name}"),
                None => short_label.clone(),
            };
            SessionFrame {
                id: tab.id(),
                group_id: tab.group_id(),
                width_weight: tab.width_weight(),
                shell: tab.shell(),
                short_label,
                title,
                cwd: cwd.or_else(|| tab.cwd()),
                purpose,
                running_program,
            }
        })
        .collect();
    View::new(tabs.into_model(), values)
}
pub(crate) fn binding(
    state: &AppContext,
    tabs: &TerminalStore,
    application: &Arc<ApplicationHandle>,
) -> (Binding, Sessions, FrameRequest) {
    let frame = state.clone();
    let command_state = state.clone();
    let command_tabs = tabs.clone();
    let command_application = application.clone();
    let state_binding = Binding::new(
        crate::ui::components::state_access::StateAccess::project(
            state.model(),
            |app| &app.terminal.ui,
            |app| &mut app.terminal.ui,
        ),
        move || {
            let app = frame.get();
            bindings::Frame {
                workspace_roots: app.explorer.model.roots().to_vec(),
                ui: app.terminal.ui,
                chrome: bindings::Chrome {
                    resizing_terminal: app.terminal_chrome.resizing_terminal,
                    terminal_h: app.terminal_chrome.terminal_h,
                },
                modifiers: app.editor.modifiers,
                shell_options: app.preferences.values().shell_options.clone(),
            }
        },
        move |command| run(&command_state, &command_tabs, &command_application, command),
    );
    let reader = tabs.clone();
    let screens = tabs.clone();
    let command_state = state.clone();
    let command_tabs = tabs.clone();
    let command_application = application.clone();
    let clips = tabs.clone();
    let sessions = Sessions::new(
        move || project(reader.get()),
        move |id| {
            let current = screens.get();
            let Some(tab) = current.tabs().into_iter().find(|tab| tab.id == id) else {
                return TerminalHandle::detached(
                    id,
                    crate::features::terminal::ShellKind::default(),
                )
                .snapshot();
            };
            tab.controller.snapshot()
        },
        move |command| run(&command_state, &command_tabs, &command_application, command),
        move |id, shortcut, clipboard| {
            let shortcut = match shortcut {
                bindings::ClipboardShortcut::Copy => super::terminal::ClipboardShortcut::Copy,
                bindings::ClipboardShortcut::CopyOrInterrupt => {
                    super::terminal::ClipboardShortcut::CopyOrInterrupt
                }
                bindings::ClipboardShortcut::Paste => super::terminal::ClipboardShortcut::Paste,
                bindings::ClipboardShortcut::CopyOrPaste => {
                    super::terminal::ClipboardShortcut::CopyOrPaste
                }
            };
            super::terminal::clipboard(&clips, id, shortcut, clipboard)
        },
    );
    let requested = application.clone();
    (
        state_binding,
        sessions,
        FrameRequest::new(move || requested.request_frame()),
    )
}
fn open_menu(
    app: &mut AppState,
    target: SessionId,
    position: (f32, f32),
    kind: crate::ui::terminal::state::TerminalTabMenuKind,
) {
    app.editor.menu = None;
    app.explorer.ui.menu = None;
    app.explorer.ui.menu_target = None;
    app.editor.tabs.tab_context_menu = None;
    app.terminal.ui.terminal_shell_menu = false;
    app.terminal.ui.terminal_tab_context_menu =
        Some(crate::ui::terminal::state::TerminalTabContextMenuState {
            target,
            position,
            kind,
        });
}
pub(crate) fn run(
    state: &AppContext,
    tabs: &TerminalStore,
    application: &Arc<ApplicationHandle>,
    command: Command,
) -> Outcome {
    use super::terminal as commands;
    let mut result = Outcome {
        applied: true,
        created: None,
    };
    match command {
        Command::ChooseWorkspace(root) => {
            result.created = commands::choose_workspace(state, tabs, root);
            result.applied = result.created.is_some();
        }
        Command::Select(id) => result.applied = commands::select(state, tabs, id),
        Command::New { target, shell } => {
            result.created = commands::new_from_target(state, tabs, target, shell);
            result.applied = result.created.is_some();
        }
        Command::Split(id) => {
            result.created = commands::split(state, tabs, id);
            result.applied = result.created.is_some();
        }
        Command::Close { target, action } => {
            result.applied = commands::close(state, tabs, target, action, application)
        }
        Command::Menu { target, action } => {
            result.applied = commands::menu_action(state, tabs, target, action, application)
        }
        Command::Purpose { target, purpose } => {
            commands::purpose(state, tabs, target, purpose.as_deref())
        }
        Command::ShellMenu(id) => result.applied = commands::shell_menu(state, tabs, id),
        Command::Write { target, bytes } => result.applied = commands::write(tabs, target, &bytes),
        Command::ResizePair {
            left,
            right,
            fraction,
        } => commands::resize_pair(tabs, left, right, fraction),
        Command::Mouse { target, row, col } => commands::mouse_position(tabs, target, row, col),
        Command::BeginSelection { target, row, col } => {
            commands::begin_selection(tabs, target, row, col)
        }
        Command::ExtendSelection { target, row, col } => {
            commands::extend_selection(tabs, target, row, col)
        }
        Command::Wheel {
            target,
            delta,
            modifiers,
        } => commands::wheel(tabs, target, delta, modifiers),
        Command::Focus => state.update(|app| {
            app.terminal.ui.terminal_focused = true;
            app.editor.focused = false;
        }),
        Command::ResizeStart => state.update(|app| app.terminal_chrome.resizing_terminal = true),
        Command::Resize(height) => state.update(|app| {
            if app.terminal_chrome.resizing_terminal {
                app.terminal_chrome.terminal_h = height;
            }
        }),
        Command::ResizeEnd => state.update(|app| app.terminal_chrome.resizing_terminal = false),
        Command::OpenMenu {
            target,
            position,
            kind,
        } => {
            if tabs.get().tabs().iter().any(|tab| tab.id == target) {
                state.update(|app| open_menu(app, target, position, kind));
            } else {
                result.applied = false;
            }
        }
        Command::TabClick {
            target,
            point,
            menu_position,
        } => {
            result.applied = false;
            if tabs.get().tabs().iter().any(|tab| tab.id == target) {
                state.update(|app| {
                    if app
                        .terminal
                        .ui
                        .terminal_tab_click_count(target, point.0, point.1)
                        == 2
                    {
                        app.terminal.ui.last_terminal_tab_click = None;
                        open_menu(
                            app,
                            target,
                            menu_position,
                            crate::ui::terminal::state::TerminalTabMenuKind::Purpose,
                        );
                        result.applied = true;
                    }
                });
            }
        }
        Command::ClipboardError(error) => {
            state.update(|app| app.show_error(format!("Clipboard: {error}")))
        }
    }
    result
}

pub(crate) fn render(
    rect: UiRect,
    state: AppContext,
    editor_focus: UiFocusHandle,
    terminal_focus: UiFocusHandle,
    terminal_id: UiId,
    controller: TerminalHandle,
    terminal_tabs: TerminalStore,
    application: Arc<ApplicationHandle>,
    cursor_blink_visible: bool,
    max_height: f32,
) -> Element {
    let map = super::layout_map::MapStore::default();
    let map = super::layout_map::binding(&state, &map, &terminal_tabs);
    let (state, tabs, request) = binding(&state, &terminal_tabs, &application);
    view::render_view(
        rect,
        state,
        map,
        editor_focus,
        terminal_focus,
        terminal_id,
        controller.id(),
        tabs,
        request,
        cursor_blink_visible,
        max_height,
        None,
    )
}
pub(crate) fn render_region(
    rect: UiRect,
    state: AppContext,
    map: super::layout_map::MapStore,
    editor_focus: UiFocusHandle,
    terminal_focus: UiFocusHandle,
    terminal_id: UiId,
    terminal_tabs: TerminalStore,
    application: Arc<ApplicationHandle>,
    cursor_blink_visible: bool,
    region: crate::domain::application_layout::RegionId,
) -> Element {
    let app = state.get();
    let (ids, active) = app
        .layout
        .application_layout
        .as_ref()
        .unwrap()
        .terminal_view(region)
        .unwrap();
    let frame = terminal_tabs.get().region_view(&ids, active);
    let Some(controller) = frame.active().map(|t| t.id) else {
        return lgui::prelude::group(rect);
    };
    let primary = terminal_tabs
        .get()
        .active_id()
        .is_some_and(|id| ids.contains(&id));
    let id = if primary {
        terminal_id
    } else {
        UiId::owned(format!("terminal-region-{region}"))
    };
    let map = super::layout_map::binding(&state, &map, &terminal_tabs);
    let (state, tabs, request) = binding(&state, &terminal_tabs, &application);
    view::render_view(
        rect,
        state,
        map,
        editor_focus,
        terminal_focus,
        id,
        controller,
        tabs,
        request,
        cursor_blink_visible,
        rect.height(),
        Some((region, project(frame), primary)),
    )
    .key(format!("terminal-region-{region}"))
}
pub(crate) fn menu(
    viewport: UiRect,
    state: AppContext,
    tabs: TerminalStore,
    application: Arc<ApplicationHandle>,
    terminal_focus: UiFocusHandle,
    editor_focus: UiFocusHandle,
) -> Element {
    let (state, tabs, request) = binding(&state, &tabs, &application);
    crate::ui::terminal::menu::render(viewport, state, tabs, request, terminal_focus, editor_focus)
}

#[cfg(test)]
pub(crate) fn session_rects(
    rect: UiRect,
    tabs: &[crate::app::terminal_binding::TerminalTab],
) -> Vec<UiRect> {
    view::session_rects(
        rect,
        &tabs.iter().cloned().map(session_frame).collect::<Vec<_>>(),
    )
}
#[cfg(test)]
pub(crate) fn render_terminal_tabs(
    rect: UiRect,
    values: &[crate::app::terminal_binding::TerminalTab],
    active: u64,
    tabs: TerminalStore,
    state: AppContext,
    map: super::layout_map::MapStore,
    terminal_focus: UiFocusHandle,
    editor_focus: UiFocusHandle,
    application: Arc<ApplicationHandle>,
) -> Element {
    let values = values
        .iter()
        .cloned()
        .map(session_frame)
        .collect::<Vec<_>>();
    let map = super::layout_map::binding(&state, &map, &tabs);
    let (state, tabs, request) = binding(&state, &tabs, &application);
    view::render_terminal_tabs(
        rect,
        &values,
        active,
        tabs,
        state,
        map,
        terminal_focus,
        editor_focus,
        request,
    )
}
#[cfg(test)]
mod ownership_tests {
    use super::*;
    use crate::app::runtime::AppRuntime;
    use crate::features::terminal::{ShellKind, TerminalStatus};
    use lgui::application::AppView;
    use lgui::core::UiScale;
    use lgui::session::UiSession;
    use std::sync::Mutex;
    #[test]
    fn retained_terminal_ports_and_frame_values_do_not_keep_runtime_or_sessions_alive() {
        let owner = Arc::new(AppRuntime::default());
        let weak = Arc::downgrade(&owner);
        let captured = weak.clone();
        let exposed = Arc::new(Mutex::new(None));
        let output = exposed.clone();
        let view: AppView = Arc::new(move |cx| {
            let runtime = captured.upgrade().unwrap();
            let state = AppContext::detached(cx.state_with(|| {
                let app = AppState::new();
                app
            }))
            .with_runtime(&runtime);
            let model = cx.state_with(|| {
                crate::features::terminal::model::TerminalModel::restored(
                    vec![ShellKind::Cmd],
                    None,
                )
            });
            let tabs = TerminalStore::new(model, runtime.terminals());
            let application = Arc::new(ApplicationHandle::new(|task| task(), || {}));
            let ports = binding(&state, &tabs, &application);
            let values = ports.1.get();
            *output.lock().unwrap() = Some((ports, values));
            lgui::prelude::panel(cx.viewport(), lgui::prelude::VisualStyle::default())
        });
        let mut session = UiSession::new();
        session.render_view(&view, UiRect::new(0.0, 0.0, 200.0, 100.0), UiScale::ONE);
        assert_eq!(Arc::strong_count(&owner), 1);
        let (ports, values) = exposed.lock().unwrap().take().unwrap();
        drop(owner);
        assert!(weak.upgrade().is_none());
        assert_eq!(values.active_id(), Some(1));
        assert_eq!(values.active().unwrap().shell, ShellKind::Cmd);
        assert!(matches!(ports.1.screen(1).status, TerminalStatus::Stopped));
        assert_eq!(ports.1.get().active_id(), Some(1));
    }
}
