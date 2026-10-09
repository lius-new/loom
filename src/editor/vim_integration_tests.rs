//! Vim through the real event path: focus, the keymap's capture listener,
//! key events, and text input that the platform only sends for keys no
//! handler prevented.

use std::sync::{Arc, Mutex};

use lgui::application::{AppView, ApplicationContext};
use lgui::core::{
    ImeEvent, InputEvent, KeyModifiers, KeyState, KeyboardEvent, LogicalKey, NamedKey, Point,
    PointerButton, PointerData, SemanticRole, UiScale, dispatch_runtime_output,
};
use lgui::prelude::{State, UiRect, group};
use lgui::session::UiSession;

use crate::git::GitStoreSnapshot;
use crate::model::pane_layout::PaneId;
use crate::state::AppState;
use crate::theme;
use crate::vim::Mode;

struct Ui {
    session: UiSession,
    view: AppView,
    app: ApplicationContext,
    state: State<AppState>,
    viewport: UiRect,
}

impl Ui {
    fn new(contents: &str) -> Self {
        Self::new_with_home(contents, None)
    }

    fn new_with_home(contents: &str, workspace_home: Option<bool>) -> Self {
        let exposed = Arc::new(Mutex::new(None::<State<AppState>>));
        let output = exposed.clone();
        let viewport = UiRect::new(0.0, 0.0, 600.0, 300.0);
        let app = ApplicationContext::empty(Default::default());
        let view_app = app.clone();
        let contents = contents.to_owned();
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state({
                let mut app = AppState::new();
                if let Some(project) = workspace_home {
                    if project {
                        app.workspace_folders.push("focus-test-project".into());
                    }
                } else {
                    app.workspace.open_path("vim.txt".into(), contents.clone());
                    app.vim.options.enabled = true;
                }
                app
            });
            let git_store = cx.state(GitStoreSnapshot::default());
            let terminals = cx.state(crate::terminal_session::TerminalTabs::new());
            *output.lock().unwrap() = Some(state.clone());
            let id = cx.use_stable_id();
            let focus = cx.focus_handle(id.clone());
            let terminal_id = cx.use_stable_id();
            let terminal_focus = cx.focus_handle(terminal_id);
            let env = crate::key_actions::KeyEnv {
                state: state.clone(),
                application: view_app.clone(),
                editor_focus: focus.clone(),
                editor_rect: viewport,
                terminal: None,
            };
            let surface = if state.get().workspace.active_editor().is_some() {
                super::editor_view::render(
                    PaneId::new(1),
                    viewport,
                    state.clone(),
                    git_store,
                    id,
                    focus,
                )
            } else if workspace_home == Some(true) {
                crate::ui::workspace_home::render(
                    viewport,
                    state.clone(),
                    id,
                    focus,
                    terminal_focus,
                    terminals,
                )
            } else {
                crate::ui::welcome::render(viewport, state.clone(), id, focus)
            };
            crate::key_actions::attach(group(viewport).child(surface), env)
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let state = exposed.lock().unwrap().clone().unwrap();
        let mut ui = Self {
            session,
            view,
            app,
            state,
            viewport,
        };
        // Focus the editor with a click at the start of the text.
        let point = PointerData::mouse(Point::new(theme::GUTTER_W + theme::CODE_PAD, 5.0));
        ui.send(InputEvent::PointerDown {
            pointer: point,
            button: PointerButton::Left,
        });
        ui.send(InputEvent::PointerUp {
            pointer: point,
            button: PointerButton::Left,
        });
        ui
    }

    /// Dispatch one input event; returns whether a handler prevented its
    /// default action.
    fn send(&mut self, input: InputEvent) -> bool {
        let events = self.session.handle_input(input);
        let mut prevented = false;
        let session = &mut self.session;
        dispatch_runtime_output(
            events,
            &self.app,
            &lgui::window::WindowId::new("vim-test"),
            |action| session.handle_default_action(action),
            |cx| prevented |= cx.default_prevented(),
        );
        self.session
            .render_view(&self.view, self.viewport, UiScale::ONE);
        prevented
    }

    /// Press a key the way the platform reports it: the key event, then its
    /// text unless a handler prevented the key's default action.
    fn press(&mut self, key: LogicalKey, modifiers: KeyModifiers) {
        let text = match &key {
            LogicalKey::Character(text) if !modifiers.ctrl() => Some(text.to_string()),
            _ => None,
        };
        let prevented = self.send(InputEvent::Keyboard(KeyboardEvent {
            state: KeyState::Down,
            key,
            modifiers,
            ..Default::default()
        }));
        if !prevented && let Some(text) = text {
            self.send(InputEvent::TextInput(text));
        }
    }

    /// Type Vim notation: characters, `<Esc>`, `<CR>`, `<BS>` and `<C-x>`.
    fn keys(&mut self, keys: &str) {
        for key in crate::vim::Key::parse(keys) {
            use crate::vim::Key;
            let (logical, modifiers) = match key {
                Key::Char(c) => {
                    let shift = c.is_uppercase();
                    (
                        LogicalKey::Character(c.to_string()),
                        if shift {
                            KeyModifiers::SHIFT
                        } else {
                            KeyModifiers::empty()
                        },
                    )
                }
                Key::Ctrl(c) => (LogicalKey::Character(c.to_string()), KeyModifiers::CONTROL),
                Key::Esc => (LogicalKey::Named(NamedKey::Escape), KeyModifiers::empty()),
                Key::Enter => (LogicalKey::Named(NamedKey::Enter), KeyModifiers::empty()),
                Key::Backspace => (
                    LogicalKey::Named(NamedKey::Backspace),
                    KeyModifiers::empty(),
                ),
                Key::Tab => (LogicalKey::Named(NamedKey::Tab), KeyModifiers::empty()),
                Key::Left => (
                    LogicalKey::Named(NamedKey::ArrowLeft),
                    KeyModifiers::empty(),
                ),
                other => panic!("no key event for {other:?}"),
            };
            self.press(logical, modifiers);
        }
    }

    fn text(&self) -> String {
        self.state
            .get()
            .workspace
            .active_editor()
            .unwrap()
            .text()
            .to_owned()
    }

    fn cursor(&self) -> usize {
        self.state.get().workspace.active_editor().unwrap().cursor()
    }

    fn mode(&self) -> Mode {
        self.state.get().vim.mode()
    }

    /// The editor's semantic role, which decides whether the platform input
    /// method is enabled.
    fn role(&self) -> SemanticRole {
        let tree = self.session.tree();
        let focused = self
            .session
            .runtime()
            .interaction_state()
            .focused
            .clone()
            .expect("the editor has focus");
        tree.node(&focused)
            .and_then(|node| node.semantics.as_ref())
            .map(|semantics| semantics.role)
            .expect("the editor has semantics")
    }
}

#[test]
fn first_file_inherits_focus_from_welcome() {
    first_file_inherits_focus(false);
}

#[test]
fn first_file_inherits_focus_from_workspace_home() {
    first_file_inherits_focus(true);
}

fn first_file_inherits_focus(project: bool) {
    let mut ui = Ui::new_with_home("", Some(project));
    let focused = ui.session.runtime().interaction_state().focused.clone();
    assert!(
        focused.is_some(),
        "the home surface owns focus before opening a file"
    );
    ui.state.update(|app| {
        app.workspace.open_path("first.txt".into(), "abc".into());
    });
    ui.session.render_view(&ui.view, ui.viewport, UiScale::ONE);
    assert_eq!(ui.session.runtime().interaction_state().focused, focused);
    assert!(
        ui.state.get().focused,
        "the first editor must draw its caret and accept editor keys"
    );
    ui.press(
        LogicalKey::Named(NamedKey::ArrowRight),
        KeyModifiers::empty(),
    );
    assert_eq!(ui.cursor(), 1);
    ui.send(InputEvent::TextInput("X".into()));
    assert_eq!(ui.text(), "aXbc");
}

#[test]
fn commands_do_not_type_and_insert_mode_does() {
    let mut ui = Ui::new("hello world");
    assert_eq!(ui.mode(), Mode::Normal);
    assert_eq!(
        ui.role(),
        SemanticRole::Group,
        "no input method in Normal mode"
    );
    ui.keys("w");
    assert_eq!((ui.text().as_str(), ui.cursor()), ("hello world", 6));
    ui.keys("i");
    assert_eq!(ui.mode(), Mode::Insert);
    assert_eq!(ui.role(), SemanticRole::TextInput);
    assert_eq!(ui.text(), "hello world", "the i itself is not typed");
    ui.keys("big ");
    assert_eq!(ui.text(), "hello big world");
    ui.keys("<Esc>");
    assert_eq!((ui.mode(), ui.cursor()), (Mode::Normal, 9));
    ui.keys("u");
    assert_eq!(ui.text(), "hello world", "the insert is one undo step");
    ui.keys("<C-r>");
    assert_eq!(ui.text(), "hello big world");
    ui.keys("0dw");
    assert_eq!(ui.text(), "big world");
    ui.keys("x.");
    assert_eq!(ui.text(), "g world");
}

#[test]
fn insert_mode_keys_go_to_vim_and_are_repeated() {
    let mut ui = Ui::new("a\nb");
    ui.keys("A12<BS>3<CR>x<Esc>");
    assert_eq!(ui.text(), "a13\nx\nb");
    ui.keys("j.");
    assert_eq!(ui.text(), "a13\nx\nb13\nx");
    ui.keys("uu");
    assert_eq!(ui.text(), "a\nb");
}

#[test]
fn input_method_text_in_insert_mode_then_normal_mode_commands() {
    let mut ui = Ui::new("end");
    ui.keys("i");
    ui.send(InputEvent::Ime(ImeEvent::Preedit {
        text: "nihao".into(),
        cursor: Some(0..5),
    }));
    // Keys belong to the composition while it shows text.
    ui.keys("<Esc>");
    assert_eq!(ui.mode(), Mode::Insert);
    ui.send(InputEvent::Ime(ImeEvent::Preedit {
        text: String::new(),
        cursor: None,
    }));
    ui.send(InputEvent::Ime(ImeEvent::Commit("你好".into())));
    assert_eq!(ui.text(), "你好end");
    ui.keys("<Esc>");
    assert_eq!(ui.mode(), Mode::Normal);
    ui.keys("x");
    assert_eq!(
        ui.text(),
        "你end",
        "no character was lost or typed as a command"
    );
    ui.keys("u");
    assert_eq!(ui.text(), "你好end");
    ui.keys("u");
    assert_eq!(ui.text(), "end");
}

#[test]
fn application_shortcuts_and_vim_keys_share_the_keyboard() {
    let mut ui = Ui::new("one\ntwo");
    // Ctrl+V starts a visual block instead of pasting.
    ui.keys("<C-v>j");
    assert_eq!(ui.mode(), Mode::Visual(crate::vim::VisualKind::Block));
    ui.keys("d");
    assert_eq!(ui.text(), "ne\nwo");
    // Ctrl+Z is still the application's undo.
    ui.press(LogicalKey::Character("z".into()), KeyModifiers::CONTROL);
    assert_eq!(ui.text(), "one\ntwo");
    assert_eq!(ui.mode(), Mode::Normal);
}

#[test]
fn turning_vim_off_restores_default_editing() {
    let mut ui = Ui::new("text");
    ui.keys("A!");
    assert_eq!(ui.mode(), Mode::Insert);
    ui.state
        .update(|app| super::vim_input::set_enabled(app, false));
    ui.session.render_view(&ui.view, ui.viewport, UiScale::ONE);
    assert_eq!(ui.role(), SemanticRole::TextInput);
    ui.keys("jk");
    assert_eq!(ui.text(), "text!jk");
    ui.keys("<Esc>");
    // Default editing: Ctrl+Z undoes the typing, and the insert session's
    // undo step was closed when Vim turned off.
    ui.press(LogicalKey::Character("z".into()), KeyModifiers::CONTROL);
    assert_eq!(ui.text(), "text!");
}

impl Ui {
    /// The painted caret rect and the motion offset still applied to it.
    fn caret(&self) -> (UiRect, (f32, f32)) {
        let tree = self.session.tree();
        let node = tree.node(&lgui::core::UiId::new("editor-caret-1")).unwrap();
        let layer = tree.node(node.parent.as_ref().unwrap()).unwrap();
        let transform = layer.compositing_layer.unwrap().transform;
        (
            node.layout_rect,
            (transform.translation_x(), transform.translation_y()),
        )
    }

    fn advance(&mut self, elapsed_ms: f32) {
        self.session.advance(elapsed_ms);
        self.session
            .render_view(&self.view, self.viewport, UiScale::ONE);
    }

    fn settle(&mut self) {
        self.session.advance(0.0);
        self.advance(100.0);
        assert_eq!(self.caret().1, (0.0, 0.0));
    }
}

#[test]
fn smooth_caret_animates_every_vim_shape_without_sliding_on_mode_switches() {
    let mut ui = Ui::new("hello world\nsecond line");
    ui.settle();
    let (start, _) = ui.caret();
    assert!(start.width() > 2.0, "Normal mode paints a block");

    ui.keys("w");
    let (target, offset) = ui.caret();
    assert!(target.left > start.left);
    assert_eq!(
        target.left + offset.0,
        start.left,
        "motions retarget without jumping"
    );
    ui.session.advance(0.0);
    ui.advance(40.0);
    let (_, offset) = ui.caret();
    assert!(offset.0 < 0.0 && target.left + offset.0 > start.left);
    ui.settle();

    ui.keys("i");
    let (bar, offset) = ui.caret();
    assert_eq!(offset, (0.0, 0.0), "a shape change alone does not move");
    assert_eq!((bar.left, bar.width()), (target.left, 2.0));
    assert!(bar.top > target.top);

    ui.keys("<Esc>");
    let (block, offset) = ui.caret();
    assert!(block.left < bar.left && block.width() > 2.0);
    assert_eq!(offset.1, 0.0, "switching shapes never slides vertically");
    assert_eq!(
        block.left + offset.0,
        bar.left,
        "leaving Insert mode animates the step left"
    );
    ui.settle();

    ui.keys("vl");
    assert_eq!(
        ui.caret().1,
        (0.0, 0.0),
        "the caret stays on the edge of a Visual selection"
    );
    ui.keys("<Esc>");
    ui.settle();

    ui.state.update(|app| app.smooth_caret = false);
    ui.keys("j");
    assert_eq!(ui.caret().1, (0.0, 0.0), "disabled motion snaps");
}
