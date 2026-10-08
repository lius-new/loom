//! Connects the keymap to the application: resolves key events into actions,
//! performs them, and keeps the user keymap loaded.
//!
//! All bound keys are resolved in one capture-phase listener at the root, so
//! a binding wins before the focused element sees the key. Keys no binding
//! claims continue to the focused element (editor text input, terminal bytes,
//! text fields) unchanged.

use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use lgui::ApplicationHandle;
use lgui::application::ApplicationContext;
use lgui::core::{KeyboardEvent, UiEventKind, UiEventPayload, UiFocusHandle};
use lgui::prelude::{Element, State, UiRect};
use lgui::services::ServicesContextExt;
use notify::{RecursiveMode, Watcher};

use crate::editor::editor_view;
use crate::editor::vim_input;
use crate::input::action::Action;
use crate::input::context::KeyContext;
use crate::input::context_stack::{self, EDITOR, TERMINAL, VIM};
use crate::input::dispatcher::{self, Replay};
use crate::input::keymap;
use crate::input::keymap_file::{self, UserKeymap};
use crate::input::keystroke::Keystroke;
use crate::state::AppState;
use crate::terminal_session::{TerminalController, TerminalTabs};
use crate::ui::terminal::{self, ClipboardShortcut};
use crate::ui::{clone_repository, close_confirmation};
use crate::workspace_actions;

/// Everything actions need to act on, captured once per frame.
#[derive(Clone)]
pub struct KeyEnv {
    pub state: State<AppState>,
    pub application: ApplicationContext,
    pub editor_focus: UiFocusHandle,
    /// Bounds of the main editor surface, for scrolling and page sizes.
    pub editor_rect: UiRect,
    pub terminal: Option<TerminalEnv>,
}

#[derive(Clone)]
pub struct TerminalEnv {
    pub tabs: State<TerminalTabs>,
    pub focus: UiFocusHandle,
    pub controller: Option<TerminalController>,
}

/// Install the keymap dispatcher on `root`.
pub fn attach(root: Element, env: KeyEnv) -> Element {
    root.on_event_capture(UiEventKind::KeyDown, move |ctx, payload| {
        if let UiEventPayload::Keyboard { event } = payload
            && handle_key_down(&env, event)
        {
            ctx.prevent_default();
            ctx.stop_propagation();
        }
    })
}

/// Resolve one key-down event. Returns whether the keymap consumed it.
pub fn handle_key_down(env: &KeyEnv, event: &KeyboardEvent) -> bool {
    if event.is_composing {
        return false;
    }
    let Some(keystroke) = Keystroke::from_event(event) else {
        return false;
    };
    let mut composing = false;
    let mut pending = Vec::new();
    let mut stack = Vec::new();
    env.state.try_update(|app| {
        composing = app.editor.is_composing()
            || !app.explorer_create_input.preedit.is_empty()
            || !app.git_commit_input.preedit.is_empty()
            || !app.keymap_search.preedit.is_empty();
        pending = app.pending_keystrokes.clone();
        stack = context_stack::context_stack(app);
        false
    });
    // IME composition owns every key until it commits or cancels.
    if composing {
        return false;
    }
    let keymap = keymap::current();
    let typed = keystroke.clone();
    let result = dispatcher::dispatch_key(&keymap, pending, keystroke, &stack);
    // `LOOM_LOG_KEYS=1 cargo run` traces how each keystroke resolved.
    #[cfg(debug_assertions)]
    if std::env::var_os("LOOM_LOG_KEYS").is_some() {
        eprintln!(
            "[keymap] {typed} in {} -> actions {:?}, pending {:?}, replay {}",
            context_stack::describe(&stack),
            result
                .actions
                .iter()
                .map(|action| action.name())
                .collect::<Vec<_>>(),
            result
                .pending
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            result.replay.len(),
        );
    }
    for replay in result.replay {
        run_replay(env, &stack, replay);
    }
    let waiting = !result.pending.is_empty();
    set_pending(&env.state, result.pending);
    waiting || run_actions(env, &result.actions) || vim_key(env, &stack, &typed)
}

/// Offer a keystroke no binding claimed to Vim, when it drives the editor.
fn vim_key(env: &KeyEnv, stack: &[KeyContext], keystroke: &Keystroke) -> bool {
    if !stack.iter().any(|context| context.primary() == Some(VIM)) {
        return false;
    }
    let Some(key) = crate::vim::Key::from_keystroke(keystroke) else {
        return false;
    };
    let clipboard = env.application.clipboard();
    vim_input::handle_key(&env.state, key, env.editor_rect, clipboard.as_ref())
}

/// Resolve a sequence that timed out, if `seq` still identifies it.
pub fn flush_pending(env: &KeyEnv, seq: u64) {
    let mut pending = Vec::new();
    let mut stack = Vec::new();
    env.state.try_update(|app| {
        if app.pending_keystrokes_seq != seq || app.pending_keystrokes.is_empty() {
            return false;
        }
        pending = std::mem::take(&mut app.pending_keystrokes);
        app.pending_keystrokes_seq += 1;
        stack = context_stack::context_stack(app);
        true
    });
    if pending.is_empty() {
        return;
    }
    for replay in dispatcher::flush(&keymap::current(), pending, &stack) {
        run_replay(env, &stack, replay);
    }
}

/// Wait out the pending-sequence timeout on a worker thread and flush on the
/// UI thread. Dropping the returned guard (a newer keystroke) cancels it.
pub fn start_pending_timer(env: KeyEnv, seq: u64) -> impl FnOnce() {
    let (stop_sender, stop_receiver) = mpsc::channel::<()>();
    let handle = env.application.try_resource::<ApplicationHandle>();
    let worker = thread::Builder::new()
        .name("loom-key-sequence-timeout".to_owned())
        .spawn(move || {
            if let Err(RecvTimeoutError::Timeout) =
                stop_receiver.recv_timeout(dispatcher::PENDING_TIMEOUT)
            {
                match handle {
                    Some(handle) => handle.post(move || flush_pending(&env, seq)),
                    None => flush_pending(&env, seq),
                }
            }
        })
        .ok();
    move || {
        let _ = stop_sender.send(());
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }
}

fn set_pending(state: &State<AppState>, pending: Vec<Keystroke>) {
    state.try_update(move |app| {
        if app.pending_keystrokes == pending {
            return false;
        }
        app.pending_keystrokes = pending;
        app.pending_keystrokes_seq += 1;
        true
    });
}

/// Run actions in precedence order until one applies (Zed's fall-through
/// for actions that propagate).
fn run_actions(env: &KeyEnv, actions: &[Action]) -> bool {
    actions.iter().any(|&action| perform(env, action))
}

fn run_replay(env: &KeyEnv, stack: &[KeyContext], replay: Replay) {
    if run_actions(env, &replay.actions) || vim_key(env, stack, &replay.keystroke) {
        return;
    }
    let Some(text) = replay.keystroke.text() else {
        return;
    };
    match stack.last().and_then(KeyContext::primary) {
        Some(EDITOR | VIM) => {
            let rect = env.editor_rect;
            env.state
                .update(move |app| editor_view::insert_text(app, &text, rect));
        }
        Some(TERMINAL) => {
            if let Some(controller) = env
                .terminal
                .as_ref()
                .and_then(|terminal| terminal.controller.as_ref())
            {
                terminal::write_text(controller, &text);
            }
        }
        _ => {}
    }
}

/// Perform one action. Returns `false` when it does not apply right now, so
/// the next binding for the keystroke gets a chance.
pub fn perform(env: &KeyEnv, action: Action) -> bool {
    match action {
        Action::NoAction => false,
        Action::ToggleTerminal => {
            match &env.terminal {
                Some(terminal) => terminal::toggle_panel(
                    &env.state,
                    &terminal.tabs,
                    &env.editor_focus,
                    &terminal.focus,
                ),
                None => env.state.update(|app| app.apply(action)),
            }
            true
        }
        Action::VimKeys(keys) => {
            let clipboard = env.application.clipboard();
            vim_input::feed_keys(
                &env.state,
                &keys.keys(),
                env.editor_rect,
                clipboard.as_ref(),
            )
        }
        Action::OpenKeymapFile => {
            open_keymap_file(env);
            true
        }
        Action::Save => {
            workspace_actions::save_active_document(&env.state);
            true
        }
        Action::TerminalCopy | Action::TerminalCopyOrInterrupt | Action::TerminalPaste => {
            let Some(controller) = env
                .terminal
                .as_ref()
                .and_then(|terminal| terminal.controller.as_ref())
            else {
                return false;
            };
            let shortcut = match action {
                Action::TerminalCopy => ClipboardShortcut::Copy,
                Action::TerminalCopyOrInterrupt => ClipboardShortcut::CopyOrInterrupt,
                _ => ClipboardShortcut::Paste,
            };
            let clipboard = env.application.clipboard();
            let result = terminal::run_clipboard_shortcut(controller, shortcut, clipboard.as_ref());
            terminal::report_clipboard_error(&env.state, result);
            if let Some(handle) = env.application.try_resource::<ApplicationHandle>() {
                handle.request_frame();
            }
            true
        }
        Action::DialogConfirm | Action::DialogCancel | Action::DialogSecondary => {
            let (close_request, clone_dialog) = read(&env.state, |app| {
                (app.close_request.is_some(), app.show_clone_dialog)
            });
            if close_request {
                close_confirmation::perform(
                    &env.state,
                    &env.editor_focus,
                    &env.application.windows(),
                    action,
                )
            } else if clone_dialog {
                clone_repository::perform(&env.state, action)
            } else {
                false
            }
        }
        _ => {
            if let Some(command) = action.editor_command(editor_view::page_lines(env.editor_rect)) {
                editor_view::execute_command(
                    &env.state,
                    command,
                    env.editor_rect,
                    env.application.clipboard(),
                );
            } else {
                env.state.update(move |app| app.apply(action));
            }
            true
        }
    }
}

/// Read from the state without cloning all of it.
fn read<R: Default>(state: &State<AppState>, f: impl FnOnce(&AppState) -> R) -> R {
    let mut out = R::default();
    state.try_update(|app| {
        out = f(app);
        false
    });
    out
}

fn open_keymap_file(env: &KeyEnv) {
    if open_user_keymap(&env.state) {
        env.editor_focus.focus();
    }
}

/// Open `keymap.json` in a tab, creating it from the template first.
pub fn open_user_keymap(state: &State<AppState>) -> bool {
    match keymap_file::ensure_user_file() {
        Ok(path) => {
            state.update(move |app| workspace_actions::open_launch_paths(app, vec![path]));
            true
        }
        Err(error) => {
            state.update(move |app| app.show_error(format!("Could not open keymap.json: {error}")));
            false
        }
    }
}

// ---- User keymap -------------------------------------------------------------

/// Load `keymap.json` over the defaults and install the result. Problems are
/// reported in a toast; a file that does not parse keeps the previous keymap.
pub fn reload_user_keymap(state: &State<AppState>) {
    let user = match keymap_file::load_user() {
        UserKeymap::Missing => Vec::new(),
        UserKeymap::Loaded(outcome) => {
            if let Some(first) = outcome.errors.first() {
                let message = match outcome.errors.len() {
                    1 => format!("keymap.json: {first}"),
                    count => format!("keymap.json: {first} (and {} more)", count - 1),
                };
                state.update(move |app| app.show_error(message));
            }
            outcome.bindings
        }
        UserKeymap::Invalid(error) => {
            state
                .update(move |app| app.show_error(format!("keymap.json was not applied: {error}")));
            return;
        }
    };
    keymap::install(keymap_file::build(user));
    state.update(|app| app.keymap_version += 1);
}

enum WatchMessage {
    Changed,
    Stop,
}

/// Reloads the user keymap whenever `keymap.json` changes on disk.
pub struct KeymapWatcher {
    _watcher: notify::RecommendedWatcher,
    sender: Sender<WatchMessage>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for KeymapWatcher {
    fn drop(&mut self) {
        let _ = self.sender.send(WatchMessage::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

const RELOAD_QUIET_WINDOW: Duration = Duration::from_millis(200);

pub fn watch_user_keymap(state: State<AppState>) -> Option<KeymapWatcher> {
    let path = keymap_file::user_path()?;
    let directory = path.parent()?.to_path_buf();
    std::fs::create_dir_all(&directory).ok()?;
    let (sender, receiver) = mpsc::channel();
    let event_sender = sender.clone();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if let Ok(event) = event
            && event.paths.iter().any(|changed| is_keymap_file(changed))
        {
            let _ = event_sender.send(WatchMessage::Changed);
        }
    })
    .ok()?;
    watcher
        .watch(&directory, RecursiveMode::NonRecursive)
        .ok()?;
    let worker = thread::Builder::new()
        .name("loom-keymap-watcher".to_owned())
        .spawn(move || {
            while let Ok(WatchMessage::Changed) = receiver.recv() {
                // Editors often save in several steps; wait for them to settle.
                loop {
                    match receiver.recv_timeout(RELOAD_QUIET_WINDOW) {
                        Ok(WatchMessage::Changed) => continue,
                        Ok(WatchMessage::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                        Err(RecvTimeoutError::Timeout) => break,
                    }
                }
                reload_user_keymap(&state);
            }
        })
        .ok()?;
    Some(KeymapWatcher {
        _watcher: watcher,
        sender,
        worker: Some(worker),
    })
}

fn is_keymap_file(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case(keymap_file::FILE_NAME))
}

// ---- Labels ------------------------------------------------------------------

/// The label of the binding that triggers `action` in `stack`, if any.
pub fn shortcut_label(action: Action, stack: &[KeyContext]) -> Option<String> {
    keymap::current()
        .binding_for_action(action, stack)
        .map(|binding| binding.label())
}

/// Shortcut label for an editor action, as seen from the focused editor.
pub fn editor_shortcut_label(app: &AppState, action: Action) -> Option<String> {
    shortcut_label(
        action,
        &[
            context_stack::workspace_context(app),
            context_stack::editor_context(app),
        ],
    )
}
