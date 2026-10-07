//! Derives the key context stack from application state.
//!
//! lgui has no per-element key contexts, so the focus path Zed would build
//! from the element tree is reconstructed here from `AppState`'s focus flags.

use super::context::KeyContext;
use crate::state::{AppState, MainSurface};

pub const WORKSPACE: &str = "Workspace";
pub const EDITOR: &str = "Editor";
pub const TERMINAL: &str = "Terminal";
pub const INPUT: &str = "Input";
pub const MENU: &str = "Menu";
pub const DIALOG: &str = "Dialog";
pub const CLOSE_CONFIRMATION: &str = "CloseConfirmation";
pub const CLONE_REPOSITORY: &str = "CloneRepository";

pub fn os_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}

fn surface_name(surface: MainSurface) -> &'static str {
    match surface {
        MainSurface::Editor => "editor",
        MainSurface::Diff => "diff",
        MainSurface::Settings => "settings",
        MainSurface::Keymap => "keymap",
        MainSurface::WorkspaceHome => "home",
        MainSurface::Welcome => "welcome",
        MainSurface::Empty => "empty",
    }
}

/// The workspace root node, as menus without a focused child see it.
pub fn workspace_context(app: &AppState) -> KeyContext {
    let mut workspace = KeyContext::new(WORKSPACE);
    workspace.set("os", os_name());
    workspace.set("surface", surface_name(app.main_surface()));
    workspace
}

/// The editor node, used for editor-menu labels as well as dispatch.
pub fn editor_context(app: &AppState) -> KeyContext {
    let mut editor = KeyContext::new(EDITOR);
    if let Some(extension) = app
        .workspace
        .active_path()
        .and_then(|path| path.extension())
        .and_then(|extension| extension.to_str())
    {
        editor.set("extension", &extension.to_ascii_lowercase());
    }
    editor
}

/// The context stack from the root to the focused element.
pub fn context_stack(app: &AppState) -> Vec<KeyContext> {
    // Modal dialogs are roots of their own, so workspace bindings never reach
    // through them.
    if app.close_request.is_some() {
        let mut dialog = KeyContext::new(DIALOG);
        dialog.add(CLOSE_CONFIRMATION);
        return vec![dialog];
    }
    if app.show_clone_dialog {
        let mut dialog = KeyContext::new(DIALOG);
        dialog.add(CLONE_REPOSITORY);
        return vec![dialog];
    }

    let mut stack = vec![workspace_context(app)];
    if app.explorer_create_input.focused
        || app.git_commit_input.focused
        || (app.keymap_search.focused && app.main_surface() == MainSurface::Keymap)
    {
        stack.push(KeyContext::new(INPUT));
    } else if app.terminal_focused {
        stack.push(KeyContext::new(TERMINAL));
    } else if app.focused && app.main_surface() == MainSurface::Editor {
        stack.push(editor_context(app));
    }
    if app.editor.menu.is_some() || app.context_menu.is_some() || app.tab_context_menu.is_some() {
        stack.push(KeyContext::new(MENU));
    }
    stack
}

/// Contexts joined for display, e.g. `Workspace os=windows > Editor`.
pub fn describe(stack: &[KeyContext]) -> String {
    stack
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" > ")
}

#[cfg(test)]
mod tests {
    use lgui::core::{KeyModifiers, KeyState, KeyboardEvent, LogicalKey, NamedKey};

    use super::*;
    use crate::input::action::Action;
    use crate::input::dispatcher::dispatch_key;
    use crate::input::keymap_file::default_keymap;
    use crate::input::keystroke::Keystroke;
    use crate::state::{CloseContinuation, CloseRequest};

    fn secondary() -> KeyModifiers {
        if cfg!(target_os = "macos") {
            KeyModifiers::META
        } else {
            KeyModifiers::CONTROL
        }
    }

    fn event(key: LogicalKey, modifiers: KeyModifiers) -> KeyboardEvent {
        KeyboardEvent {
            state: KeyState::Down,
            key,
            modifiers,
            ..Default::default()
        }
    }

    fn character(value: &str, modifiers: KeyModifiers) -> KeyboardEvent {
        event(LogicalKey::Character(value.into()), modifiers)
    }

    fn named(key: NamedKey, modifiers: KeyModifiers) -> KeyboardEvent {
        event(LogicalKey::Named(key), modifiers)
    }

    /// The first action the default keymap runs for `event` in `app`.
    fn action(app: &AppState, event: KeyboardEvent) -> Option<Action> {
        let keystroke = Keystroke::from_event(&event).unwrap();
        dispatch_key(
            &default_keymap(),
            Vec::new(),
            keystroke,
            &context_stack(app),
        )
        .actions
        .first()
        .copied()
    }

    fn editor_app() -> AppState {
        let mut app = AppState::new();
        app.workspace
            .open_path("main.rs".into(), "fn main() {}".into());
        app.focused = true;
        app
    }

    #[test]
    fn stacks_follow_focus_and_modals() {
        let app = editor_app();
        let stack = context_stack(&app);
        assert_eq!(describe(&stack[1..]), "Editor extension=rs");
        assert_eq!(stack[0].get("surface"), Some("editor"));

        let mut terminal = editor_app();
        terminal.focused = false;
        terminal.terminal_focused = true;
        terminal.editor.menu = Some((0.0, 0.0));
        let primaries = context_stack(&terminal)
            .iter()
            .map(|context| context.primary().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(primaries, [WORKSPACE, TERMINAL, MENU]);

        let mut dialog = editor_app();
        dialog.show_clone_dialog = true;
        assert_eq!(describe(&context_stack(&dialog)), "Dialog CloneRepository");
    }

    #[test]
    fn workspace_shortcuts_match_the_previous_hardcoded_keymap() {
        let app = AppState::new();
        let s = secondary();
        let cases = [
            (character("b", s), Action::ToggleDrawer),
            (
                character("G", s | KeyModifiers::SHIFT),
                Action::OpenSourceControl,
            ),
            (character("`", s), Action::ToggleTerminal),
            (character("w", s), Action::CloseActiveItem),
            (character(",", s), Action::OpenSettings),
            (character("1", s), Action::ActivateItem(0)),
            (character("8", s), Action::ActivateItem(7)),
            (character("9", s), Action::ActivateLastItem),
            (
                named(NamedKey::Tab, KeyModifiers::CONTROL),
                Action::ActivateNextItem,
            ),
            (
                named(NamedKey::Tab, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
                Action::ActivatePrevItem,
            ),
            (
                named(NamedKey::Escape, KeyModifiers::empty()),
                Action::CloseOverlay,
            ),
        ];
        for (event, expected) in cases {
            assert_eq!(
                action(&app, event.clone()),
                Some(expected),
                "{:?}",
                event.key
            );
        }
        // Ctrl+P stays unbound.
        assert_eq!(action(&app, character("p", s)), None);
    }

    #[test]
    fn editor_shortcuts_match_the_previous_command_table() {
        let app = editor_app();
        let s = secondary();
        let none = KeyModifiers::empty();
        let shift = KeyModifiers::SHIFT;
        let cases = [
            (character("s", s), Action::Save),
            (character("z", s), Action::Undo),
            (character("Z", s | shift), Action::Redo),
            (character("y", s), Action::Redo),
            (character("a", s), Action::SelectAll),
            (character("c", s), Action::Copy),
            (character("x", s), Action::Cut),
            (character("v", s), Action::Paste),
            (named(NamedKey::ArrowLeft, none), Action::MoveLeft),
            (
                named(NamedKey::ArrowLeft, s),
                Action::MoveToPreviousWordStart,
            ),
            (
                named(NamedKey::ArrowRight, s | shift),
                Action::SelectToNextWordEnd,
            ),
            (named(NamedKey::Home, s | shift), Action::SelectToBeginning),
            (named(NamedKey::PageDown, shift), Action::SelectPageDown),
            (
                named(NamedKey::Backspace, s),
                Action::DeleteToPreviousWordStart,
            ),
            (named(NamedKey::Delete, none), Action::Delete),
            (named(NamedKey::Enter, none), Action::Newline),
            (named(NamedKey::Tab, none), Action::Tab),
            (named(NamedKey::Tab, shift), Action::Backtab),
            (named(NamedKey::Escape, none), Action::Cancel),
        ];
        for (event, expected) in cases {
            assert_eq!(
                action(&app, event.clone()),
                Some(expected),
                "{:?}",
                event.key
            );
        }
        // Workspace bindings still apply from inside the editor.
        assert_eq!(action(&app, character("b", s)), Some(Action::ToggleDrawer));
        // Ctrl+Up has no binding and reaches the editor as an unbound key.
        assert_eq!(action(&app, named(NamedKey::ArrowUp, s)), None);
    }

    #[test]
    fn terminal_keeps_shell_keys_and_clipboard_conventions() {
        let mut app = AppState::new();
        app.terminal_focused = true;
        let ctrl = KeyModifiers::CONTROL;
        assert_eq!(
            action(&app, character("c", ctrl)),
            Some(Action::TerminalCopyOrInterrupt)
        );
        assert_eq!(
            action(&app, character("C", ctrl | KeyModifiers::SHIFT)),
            Some(Action::TerminalCopy)
        );
        assert_eq!(
            action(&app, character("v", ctrl)),
            Some(Action::TerminalPaste)
        );
        assert_eq!(
            action(&app, named(NamedKey::Insert, KeyModifiers::SHIFT)),
            Some(Action::TerminalPaste)
        );
        assert_eq!(action(&app, character("d", ctrl)), None);
        assert_eq!(
            action(&app, named(NamedKey::Escape, KeyModifiers::empty())),
            None
        );
        assert_eq!(action(&app, named(NamedKey::Tab, ctrl)), None);
        if !cfg!(target_os = "macos") {
            assert_eq!(action(&app, character("w", ctrl)), None);
            assert_eq!(
                action(&app, character("b", ctrl)),
                Some(Action::ToggleDrawer)
            );
            // Ctrl+K is the shell's kill-line, not the start of a sequence.
            let keystroke = Keystroke::parse("ctrl-k").unwrap();
            assert!(
                dispatch_key(
                    &default_keymap(),
                    Vec::new(),
                    keystroke,
                    &context_stack(&app)
                )
                .is_unbound()
            );
        }
    }

    #[test]
    fn inputs_and_dialogs_own_their_keys() {
        let mut input = AppState::new();
        input.git_commit_input.focused = true;
        assert_eq!(
            action(&input, named(NamedKey::Escape, KeyModifiers::empty())),
            None
        );

        let mut closing = editor_app();
        closing.close_request = Some(CloseRequest {
            targets: Vec::new(),
            continuation: CloseContinuation::CloseTabs,
        });
        assert_eq!(
            action(&closing, named(NamedKey::Enter, KeyModifiers::empty())),
            Some(Action::DialogConfirm)
        );
        assert_eq!(
            action(&closing, character("d", secondary())),
            Some(Action::DialogSecondary)
        );
        // Modal: workspace shortcuts do not reach through the dialog.
        assert_eq!(action(&closing, character("b", secondary())), None);
    }

    #[test]
    fn sequences_open_the_keymap_page() {
        let app = AppState::new();
        let keymap = default_keymap();
        let stack = context_stack(&app);
        let first = dispatch_key(
            &keymap,
            Vec::new(),
            Keystroke::parse("secondary-k").unwrap(),
            &stack,
        );
        assert!(!first.pending.is_empty());
        let second = dispatch_key(
            &keymap,
            first.pending,
            Keystroke::parse("secondary-s").unwrap(),
            &stack,
        );
        assert_eq!(second.actions, [Action::OpenKeymap]);
    }
}
