use crate::app::terminal_runtime::TerminalDisplay;
use crate::features::terminal::actions::MenuAction;
use crate::state::AppState;
// Application command regressions against the real model, layout and weak runtime binding.
use super::terminal::*;
use crate::app::terminal_binding::{TerminalStore, TerminalTabs};
use crate::domain::application_layout::{ApplicationLayout, Content, Placement};
use crate::features::terminal::ShellKind;
use lgui::application::AppView;
use lgui::core::UiScale;
use lgui::prelude::{State, UiRect, VisualStyle, panel};
use lgui::services::{Clipboard, ClipboardError};
use lgui::session::UiSession;
use std::sync::{Arc, Mutex};

pub(super) fn fixture() -> (UiSession, State<AppState>, TerminalStore) {
    let exposed = Arc::new(Mutex::new(None));
    let out = exposed.clone();
    let rect = UiRect::new(0.0, 0.0, 800.0, 600.0);
    let view: AppView = Arc::new(move |cx| {
        let state = cx.state_with(|| {
            let mut app = AppState::new();
            app.terminal_chrome.show_terminal = true;
            app.terminal.ui.terminal_focused = true;
            app.layout.application_layout =
                Some(ApplicationLayout::initial(rect.into(), 100.0, 100.0, 200.0));
            app
        });
        let tabs =
            crate::app::terminal_binding::test_store(cx, Some(state.clone()), TerminalTabs::new);
        let snapshot = tabs.get();
        state.update(|app| snapshot.sync_layout(app.layout.application_layout.as_mut().unwrap()));
        *out.lock().unwrap() = Some((state, tabs));
        panel(rect, VisualStyle::default())
    });
    let mut session = UiSession::new();
    session.render_view(&view, rect, UiScale::ONE);
    let (state, tabs) = exposed.lock().unwrap().clone().unwrap();
    (session, state, tabs)
}
fn application() -> Arc<lgui::ApplicationHandle> {
    Arc::new(lgui::ApplicationHandle::new(|task| task(), || {}))
}
fn invariant(state: &State<AppState>, tabs: &TerminalStore) {
    let app = state.get();
    let model = tabs.get();
    let layout = app.layout.application_layout.as_ref().unwrap();
    let mut owned = layout
        .leaves()
        .into_iter()
        .flat_map(|(_, content)| match content {
            Content::Terminal { tabs, active } => {
                assert!(active.is_none_or(|id| tabs.contains(&id)));
                tabs
            }
            _ => vec![],
        })
        .collect::<Vec<_>>();
    owned.sort_unstable();
    let mut ids = model.tabs().iter().map(|t| t.id).collect::<Vec<_>>();
    ids.sort_unstable();
    assert_eq!(owned, ids);
    assert!(model.active_id().is_none_or(|id| ids.contains(&id)));
}

#[test]
fn menu_commands_resolve_inactive_target_shell_directory_and_group_at_execution() {
    let (_session, state, tabs) = fixture();
    let clicked = new(&state, &tabs, ShellKind::Cmd, Some("clicked-cwd".into()));
    let controller = target(&tabs, clicked).unwrap().controller.test_controller();
    *controller.test_cwd() = Some("live-cwd".into());
    let active = new(&state, &tabs, ShellKind::Bash, Some("active-cwd".into()));
    assert_eq!(tabs.get().active_id(), Some(active));
    assert!(menu_action(
        &state,
        &tabs,
        clicked,
        MenuAction::New,
        &application()
    ));
    let created = tabs.get().active().unwrap();
    assert_eq!(created.controller.shell(), ShellKind::Cmd);
    assert_eq!(created.cwd, Some("live-cwd".into()));
    select(&state, &tabs, active);
    assert!(menu_action(
        &state,
        &tabs,
        clicked,
        MenuAction::Split,
        &application()
    ));
    let sibling = tabs.get().active().unwrap();
    assert_eq!(sibling.group_id, target(&tabs, clicked).unwrap().group_id);
    assert_eq!(sibling.cwd, Some("live-cwd".into()));
    assert!(menu_action(
        &state,
        &tabs,
        clicked,
        MenuAction::Others,
        &application()
    ));
    assert_eq!(
        tabs.get().tabs().iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![clicked, sibling.id]
    );
    assert!(
        controller.test_same_runtime(&target(&tabs, clicked).unwrap().controller.test_controller())
    );
    invariant(&state, &tabs);
}

#[test]
fn closed_targets_cannot_select_create_split_close_or_send_input_to_survivor() {
    let (_session, state, tabs) = fixture();
    let closed = tabs.get().active_id().unwrap();
    let survivor = new(&state, &tabs, ShellKind::Cmd, None);
    let control = target(&tabs, survivor)
        .unwrap()
        .controller
        .test_controller();
    let app = application();
    assert!(close(&state, &tabs, closed, MenuAction::Close, &app));
    for action in [
        MenuAction::New,
        MenuAction::Split,
        MenuAction::Close,
        MenuAction::Others,
        MenuAction::Right,
        MenuAction::All,
    ] {
        assert!(!menu_action(&state, &tabs, closed, action, &app));
    }
    assert!(!select(&state, &tabs, closed));
    assert!(new_from_target(&state, &tabs, closed, Some(ShellKind::Bash), false).is_none());
    assert!(split(&state, &tabs, closed).is_none());
    assert!(!write(&tabs, closed, b"stale"));
    let clip = TestClipboard::default();
    clipboard(&tabs, closed, ClipboardShortcut::CopyOrInterrupt, &clip).unwrap();
    assert_eq!(*clip.calls.lock().unwrap(), 0);
    assert!(control.test_writes().is_empty());
    assert_eq!(tabs.get().active_id(), Some(survivor));
    assert_eq!(tabs.get().tabs().len(), 1);
    invariant(&state, &tabs);
}

#[test]
fn closing_last_session_prunes_layout_stops_runtime_and_hides_default_region() {
    let (_session, state, tabs) = fixture();
    let id = tabs.get().active_id().unwrap();
    let handle = target(&tabs, id).unwrap().controller;
    let control = handle.test_controller();
    assert!(close(&state, &tabs, id, MenuAction::Close, &application()));
    assert!(tabs.get().is_empty());
    assert!(!state.get().terminal_chrome.show_terminal);
    assert!(!state.get().terminal.ui.terminal_focused);
    assert!(matches!(
        control.snapshot().status,
        crate::features::terminal::TerminalStatus::Stopped
    ));
    assert!(matches!(
        handle.snapshot().status,
        crate::features::terminal::TerminalStatus::Stopped
    ));
    assert!(!close(&state, &tabs, id, MenuAction::All, &application()));
    invariant(&state, &tabs);
}

#[derive(Default)]
struct TestClipboard {
    text: Mutex<Option<String>>,
    calls: Mutex<usize>,
    fail: bool,
}
impl Clipboard for TestClipboard {
    fn read_text(&self) -> Result<Option<String>, ClipboardError> {
        *self.calls.lock().unwrap() += 1;
        if self.fail {
            Err(ClipboardError::AccessDenied)
        } else {
            Ok(self.text.lock().unwrap().clone())
        }
    }
    fn write_text(&self, text: &str) -> Result<(), ClipboardError> {
        *self.calls.lock().unwrap() += 1;
        if self.fail {
            return Err(ClipboardError::AccessDenied);
        }
        *self.text.lock().unwrap() = Some(text.into());
        Ok(())
    }
}

#[test]
fn clipboard_failure_preserves_selection_and_emits_neither_interrupt_nor_paste_bytes() {
    let (_session, state, tabs) = fixture();
    let selected = tabs.get().active_id().unwrap();
    let controller = target(&tabs, selected)
        .unwrap()
        .controller
        .test_controller();
    controller.test_parser().process(b"selected text");
    begin_selection(&tabs, selected, 0, 0);
    extend_selection(&tabs, selected, 0, 8);
    let before = controller.test_selection();
    let other = new(&state, &tabs, ShellKind::Cmd, None);
    let fail = TestClipboard {
        fail: true,
        ..Default::default()
    };
    for shortcut in [
        ClipboardShortcut::Copy,
        ClipboardShortcut::CopyOrInterrupt,
        ClipboardShortcut::CopyOrPaste,
        ClipboardShortcut::Paste,
    ] {
        assert!(clipboard(&tabs, selected, shortcut, &fail).is_err());
        assert_eq!(controller.test_selection(), before);
        assert!(controller.test_writes().is_empty());
    }
    let success = TestClipboard::default();
    clipboard(
        &tabs,
        selected,
        ClipboardShortcut::CopyOrInterrupt,
        &success,
    )
    .unwrap();
    assert_eq!(success.text.lock().unwrap().as_deref(), Some("selected"));
    assert!(controller.test_selection().is_none());
    assert!(controller.test_writes().is_empty());
    clipboard(
        &tabs,
        selected,
        ClipboardShortcut::CopyOrInterrupt,
        &success,
    )
    .unwrap();
    controller.test_parser().process(b"\x1b[?2004h");
    *success.text.lock().unwrap() = Some("literal\r\n$HOME".into());
    clipboard(&tabs, selected, ClipboardShortcut::Paste, &success).unwrap();
    write(&tabs, selected, b"replayed");
    assert_eq!(
        controller.test_writes(),
        vec![
            b"\x03".to_vec(),
            b"\x1b[200~literal\r$HOME\x1b[201~".to_vec(),
            b"replayed".to_vec()
        ]
    );
    assert!(
        target(&tabs, other)
            .unwrap()
            .controller
            .test_controller()
            .test_writes()
            .is_empty()
    );
}

#[test]
fn detached_session_selection_and_close_preserve_layout_ownership_and_runtime() {
    let (_session, state, tabs) = fixture();
    let id = tabs.get().active_id().unwrap();
    let other = new(&state, &tabs, ShellKind::Cmd, None);
    let before = target(&tabs, other).unwrap().controller.test_controller();
    state.update(|app| {
        let visible = visibility(app);
        let layout = app.layout.application_layout.as_mut().unwrap();
        let plan = layout
            .plan(
                other,
                &Placement::Side(1, crate::domain::pane_layout::Direction::Right),
                UiRect::new(0.0, 0.0, 800.0, 600.0).into(),
                visible,
                vec![id, other],
            )
            .unwrap();
        *layout = plan.layout;
    });
    assert!(select(&state, &tabs, other));
    assert!(close(&state, &tabs, id, MenuAction::Close, &application()));
    invariant(&state, &tabs);
    assert!(before.test_same_runtime(&target(&tabs, other).unwrap().controller.test_controller()));
    assert!(
        state
            .get()
            .layout
            .application_layout
            .unwrap()
            .terminal_region(other)
            .is_some()
    );
}

#[test]
fn profile_binding_preserves_literal_arguments_project_default_and_closed_draft_for_blur() {
    use crate::app::terminal_profiles::{binding, edit, settings_frame};
    use crate::ui::terminal::settings_binding::Command;
    let (_session, state, _tabs) = fixture();
    let root = std::path::PathBuf::from("project-shell");
    state.update(|app| {
        app.explorer.model.restore(vec![root.clone()], vec![]);
        app.default_shell = ShellKind::Cmd;
        app.project_settings.push((
            root.clone(),
            crate::infrastructure::project_settings::ProjectSettings {
                overrides: [("default_shell".into(), serde_json::json!("bash"))]
                    .into_iter()
                    .collect(),
                errors: vec![],
            },
        ));
        edit(app, None);
        let draft = app.terminal.ui.shell_editor.as_mut().unwrap();
        draft.name.set_text("Literal profile");
        draft
            .program
            .set_text(std::env::current_exe().unwrap().display().to_string());
        draft
            .args
            .set_text(r#""$HOME" "%PATH%" "$(echo never-expanded)" "~" """#);
    });
    let port = binding(&state);
    assert_eq!(
        port.frame().project_shell.as_ref().map(|p| p.0.as_str()),
        Some("project-shell")
    );
    port.run(Command::Save);
    let snapshot = state.get();
    let saved = &snapshot.terminal_profiles[0];
    assert_eq!(
        saved.args,
        ["$HOME", "%PATH%", "$(echo never-expanded)", "~", ""]
    );
    let id = saved.id;
    assert!(!snapshot.terminal.ui.shell_editor.as_ref().unwrap().open);
    // A late blur still updates the retained draft, through the narrow projection.
    port.ui
        .update(|ui| ui.shell_editor.as_mut().unwrap().name.focused = false);
    assert_eq!(
        port.ui.get().shell_editor.unwrap().name.text(),
        "Literal profile"
    );
    port.run(Command::Select(ShellKind::Custom(id)));
    assert_eq!(state.get().default_shell, ShellKind::Custom(id));
    assert_eq!(
        state.get().settings_at(Some(&root)).default_shell,
        ShellKind::Bash
    );
    assert_eq!(
        settings_frame(&state.get()).project_shell.unwrap().1,
        state.get().shell_name(ShellKind::Bash)
    );
    port.run(Command::Remove(id));
    port.run(Command::Select(ShellKind::Custom(id)));
    assert_eq!(state.get().default_shell, ShellKind::default());
    assert_eq!(
        state.get().settings_at(Some(&root)).default_shell,
        ShellKind::Bash
    );
}
