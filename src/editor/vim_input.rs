//! Connects the Vim layer to the editor: the active view, keys and text,
//! pointer selections, the clipboard and the application operations Vim
//! asks for.

use std::ops::Range;

use lgui::prelude::{State, UiRect};
use lgui::services::Clipboard as SystemClipboard;

use super::editor_view::{page_lines, reveal_cursor};
use crate::state::AppState;
use crate::theme;
use crate::vim::{self, Key, Request, ScrollPosition};

/// The application as Vim's host for one key or command.
struct AppHost<'a> {
    clipboard: &'a dyn SystemClipboard,
    page_lines: usize,
    visible: Range<usize>,
    requests: Vec<Request>,
}

impl vim::Clipboard for AppHost<'_> {
    fn read(&mut self) -> Result<Option<String>, String> {
        self.clipboard
            .read_text()
            .map_err(|error| format!("Clipboard: {error}"))
    }

    fn write(&mut self, text: &str) -> Result<(), String> {
        self.clipboard
            .write_text(text)
            .map_err(|error| format!("Clipboard: {error}"))
    }
}

impl vim::Host for AppHost<'_> {
    fn page_lines(&self) -> usize {
        self.page_lines
    }

    fn visible_lines(&self) -> Range<usize> {
        self.visible.clone()
    }

    fn request(&mut self, request: Request) {
        self.requests.push(request);
    }
}

/// Whether Vim drives the active view: it is enabled and a text file is
/// active in the focused pane.
pub fn is_active(app: &AppState) -> bool {
    app.vim.is_enabled() && app.workspace.active_editor().is_some()
}

/// Keep Vim's state on the active view. When the active view changed, the
/// previous view leaves Vim (ending an insert) before the new one enters.
pub fn sync_view(app: &mut AppState) {
    if !app.vim.is_enabled() {
        return;
    }
    let pane = app.workspace.active_pane();
    let current = app.workspace.active().map(|document| (pane, document));
    if app.vim.view() == current {
        return;
    }
    if let Some((old_pane, old_document)) = app.vim.view() {
        let mut editor = app.workspace.view_editor_mut(old_pane, old_document);
        app.vim.leave_view(editor.as_mut());
    }
    if let Some(view) = current
        && let Some(mut editor) = app.workspace.active_editor_mut()
    {
        app.vim.enter_view(view, &mut editor);
    }
}

/// Turn Vim on or off from Settings.
pub fn set_enabled(app: &mut AppState, enabled: bool) {
    let view = app.vim.view();
    let mut editor =
        view.and_then(|(pane, document)| app.workspace.view_editor_mut(pane, document));
    app.vim.set_enabled(enabled, editor.as_mut());
    drop(editor);
    sync_view(app);
}

fn visible_lines(app: &AppState, rect: UiRect) -> Range<usize> {
    let (_, scroll_y) = app.workspace.active_scroll();
    let first = (scroll_y / theme::LINE_H).ceil() as usize;
    let last = ((scroll_y + rect.height()) / theme::LINE_H).floor() as usize;
    first..last.max(first + 1)
}

/// Run `f` with Vim, the active editor and a host, then promote an edited
/// preview, follow the cursor and return what Vim asked the application to do.
fn with_vim<R>(
    app: &mut AppState,
    rect: UiRect,
    clipboard: &dyn SystemClipboard,
    f: impl FnOnce(&mut vim::Vim, &mut crate::model::text::EditorMut<'_>, &mut AppHost<'_>) -> R,
) -> Option<(R, Vec<Request>)> {
    sync_view(app);
    let mut host = AppHost {
        clipboard,
        page_lines: page_lines(rect),
        visible: visible_lines(app, rect),
        requests: Vec::new(),
    };
    let (result, changed) = {
        let AppState { workspace, vim, .. } = app;
        let mut editor = workspace.active_editor_mut()?;
        let version = editor.version();
        let result = f(vim, &mut editor, &mut host);
        (result, editor.version() != version)
    };
    if changed {
        app.workspace.promote_active_preview();
    }
    app.editor.menu = None;
    app.editor.drag = None;
    let mut requests = Vec::new();
    for request in host.requests {
        match request {
            Request::Scroll(position) => scroll_cursor(app, rect, position),
            other => requests.push(other),
        }
    }
    reveal_cursor(app, rect);
    Some((result, requests))
}

/// Offer a key the keymap did not claim to Vim. Returns whether Vim used it.
pub fn handle_key(
    state: &State<AppState>,
    key: Key,
    rect: UiRect,
    clipboard: &dyn SystemClipboard,
) -> bool {
    let mut consumed = false;
    let mut requests = Vec::new();
    state.update(|app| {
        if !is_active(app) {
            return;
        }
        let scrolls_page = matches!(key, Key::Ctrl('d' | 'u' | 'f' | 'b'));
        let before = app
            .workspace
            .active_editor()
            .map(|editor| editor.line_of(editor.cursor()));
        if let Some((used, asked)) = with_vim(app, rect, clipboard, |vim, editor, host| {
            vim.handle_key(editor, key, host)
        }) {
            consumed = used;
            requests = asked;
        }
        if scrolls_page {
            // Page keys scroll the view with the cursor, as Vim does.
            let after = app
                .workspace
                .active_editor()
                .map(|editor| editor.line_of(editor.cursor()));
            if let (Some(before), Some(after)) = (before, after) {
                let (x, y) = app.workspace.active_scroll();
                let delta = (after as f32 - before as f32) * theme::LINE_H;
                app.workspace.set_active_scroll(x, (y + delta).max(0.0));
                reveal_cursor(app, rect);
            }
        }
    });
    perform(state, requests);
    consumed
}

/// Send the keys of a `vim::Keys` mapping to Vim. Returns false when Vim does
/// not drive the editor, so other bindings for the keystroke can run.
pub fn feed_keys(
    state: &State<AppState>,
    keys: &str,
    rect: UiRect,
    clipboard: &dyn SystemClipboard,
) -> bool {
    let keys = Key::parse(keys);
    let mut fed = false;
    let mut requests = Vec::new();
    state.update(|app| {
        if !is_active(app) {
            return;
        }
        if let Some(((), asked)) = with_vim(app, rect, clipboard, |vim, editor, host| {
            vim.feed_keys(editor, &keys, host)
        }) {
            fed = true;
            requests = asked;
        }
    });
    perform(state, requests);
    fed
}

/// Copy or cut the visual selection through Vim's `"+` register. Returns
/// whether Vim handled it (it was in Visual mode).
pub fn visual_clipboard(
    app: &mut AppState,
    cut: bool,
    rect: UiRect,
    clipboard: &dyn SystemClipboard,
) -> bool {
    if !is_active(app) || !matches!(app.vim.mode(), vim::Mode::Visual(_)) {
        return false;
    }
    let keys = if cut { "\"+d" } else { "\"+y" };
    with_vim(app, rect, clipboard, |vim, editor, host| {
        for key in Key::parse(keys) {
            vim.handle_key(editor, key, host);
        }
    });
    true
}

/// Text input for Vim. Returns whether Vim took it; in Normal and Visual mode
/// typed text is not a command and is left to the caller.
pub fn handle_text(app: &mut AppState, text: &str, rect: UiRect) -> bool {
    if !is_active(app) {
        return false;
    }
    sync_view(app);
    let handled = {
        let AppState { workspace, vim, .. } = &mut *app;
        let Some(mut editor) = workspace.active_editor_mut() else {
            return false;
        };
        let version = editor.version();
        let handled = vim.handle_text(&mut editor, text);
        if editor.version() != version {
            drop(editor);
            workspace.promote_active_preview();
        }
        handled
    };
    if handled {
        reveal_cursor(app, rect);
    }
    handled
}

/// An edit the default editing commands made while Vim is typing, so `.`
/// and counts repeat it; `None` for a cursor move.
pub fn record_insert(app: &mut AppState, event: Option<vim::InsertEvent>) {
    if is_active(app) && app.vim.is_inserting() {
        app.vim.record_insert(event);
    }
}

/// After the pointer moved the cursor or selected text.
pub fn after_pointer(app: &mut AppState) {
    if !is_active(app) {
        return;
    }
    sync_view(app);
    let AppState { workspace, vim, .. } = app;
    if let Some(mut editor) = workspace.active_editor_mut() {
        vim.after_pointer(&mut editor);
    }
}

/// After a default editing command changed the selection (select all,
/// undo), let Vim take over the selection in Normal and Visual mode.
pub fn after_command(app: &mut AppState, undo: bool) {
    if !is_active(app) || app.vim.is_inserting() {
        return;
    }
    sync_view(app);
    let AppState { workspace, vim, .. } = app;
    if let Some(mut editor) = workspace.active_editor_mut() {
        if undo {
            // Like `u`: the cursor goes to the change, without a selection.
            let selection = editor.selection_state();
            let start = selection
                .range()
                .map_or(selection.cursor, |range| range.start);
            editor.set_cursor(start);
        }
        vim.adopt_selection(&mut editor);
    }
}

fn scroll_cursor(app: &mut AppState, rect: UiRect, position: ScrollPosition) {
    let Some(line) = app
        .workspace
        .active_editor()
        .map(|editor| editor.line_of(editor.cursor()))
    else {
        return;
    };
    let top = line as f32 * theme::LINE_H;
    let height = rect.height();
    let y = match position {
        ScrollPosition::Top => top,
        ScrollPosition::Center => top - (height - theme::LINE_H) / 2.0,
        ScrollPosition::Bottom => top + theme::LINE_H - height,
    };
    let (x, _) = app.workspace.active_scroll();
    app.workspace.set_active_scroll(x, y.max(0.0));
}

/// Carry out what Vim asked of the application.
fn perform(state: &State<AppState>, requests: Vec<Request>) {
    for request in requests {
        match request {
            Request::Save => {
                crate::workspace_actions::save_active_document(state);
            }
            Request::Quit { force } => quit(state, force),
            Request::SaveAndQuit => {
                if crate::workspace_actions::save_active_document(state) {
                    quit(state, false);
                }
            }
            Request::Scroll(_) => {}
        }
    }
}

/// `:q` closes the active tab. Unsaved changes stop it unless forced or the
/// document is still open in another tab.
fn quit(state: &State<AppState>, force: bool) {
    state.update(move |app| {
        let pane = app.workspace.active_pane();
        let Some(document) = app.workspace.active() else {
            return;
        };
        let shared = app.workspace.tabs_of(document).len() > 1;
        if !force && !shared && app.workspace.is_dirty(document) {
            app.vim
                .show_message("E37: No write since last change (add ! to override)", true);
            return;
        }
        sync_view(app);
        let view = app.vim.view();
        let mut editor =
            view.and_then(|(pane, document)| app.workspace.view_editor_mut(pane, document));
        app.vim.leave_view(editor.as_mut());
        drop(editor);
        app.workspace.close_item(pane, document);
        sync_view(app);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::pane_layout::Direction;
    use lgui::services::ClipboardError;

    struct NoClipboard;

    impl SystemClipboard for NoClipboard {
        fn read_text(&self) -> Result<Option<String>, ClipboardError> {
            Ok(None)
        }
        fn write_text(&self, _: &str) -> Result<(), ClipboardError> {
            Ok(())
        }
    }

    const RECT: UiRect = UiRect {
        left: 0.0,
        top: 0.0,
        right: 600.0,
        bottom: 400.0,
    };

    fn keys(app: &mut AppState, keys: &str) {
        for key in Key::parse(keys) {
            let used = with_vim(app, RECT, &NoClipboard, |vim, editor, host| {
                vim.handle_key(editor, key, host)
            });
            if used.is_some_and(|(used, _)| !used)
                && let Key::Char(c) = key
            {
                handle_text(app, &c.to_string(), RECT);
            }
        }
    }

    fn text(app: &AppState) -> String {
        app.workspace.active_editor().unwrap().text().to_owned()
    }

    #[test]
    fn switching_panes_ends_the_insert_and_shares_text_and_history() {
        let mut app = AppState::new();
        app.workspace
            .open_path("shared.txt".into(), "one\ntwo".into());
        app.vim.options.enabled = true;
        let left = app.workspace.active_pane();
        let right = app.workspace.split(left, Direction::Right).unwrap();
        app.workspace.activate_pane(left);
        keys(&mut app, "jA!");
        assert_eq!(app.vim.mode(), vim::Mode::Insert);
        keys(&mut app, "?");

        // The other pane: the left pane's insert ends, this pane starts in
        // Normal mode and sees the shared text.
        app.workspace.activate_pane(right);
        sync_view(&mut app);
        assert_eq!(app.vim.mode(), vim::Mode::Normal);
        assert_eq!(app.vim.view().map(|view| view.0), Some(right));
        assert_eq!(text(&app), "one\ntwo!?");
        keys(&mut app, "ggI> <Esc>");
        assert_eq!(text(&app), "> one\ntwo!?");
        // The left view's cursor followed the edit made in the right pane.
        assert_eq!(app.workspace.editor(left).unwrap().cursor(), 11);

        // One history: undo from the right pane reverses its insert, then
        // the left pane's insert as a single step.
        keys(&mut app, "u");
        assert_eq!(text(&app), "one\ntwo!?");
        keys(&mut app, "u");
        assert_eq!(text(&app), "one\ntwo");
        app.workspace.activate_pane(left);
        keys(&mut app, "<C-r>");
        assert_eq!(text(&app), "one\ntwo!?");
        assert_eq!(app.workspace.editor(left).unwrap().cursor(), 7);
    }

    #[test]
    fn clicking_leaves_visual_mode_and_dragging_enters_it() {
        let mut app = AppState::new();
        app.workspace.open_path("click.txt".into(), "abcdef".into());
        app.vim.options.enabled = true;
        keys(&mut app, "vl");
        assert!(matches!(app.vim.mode(), vim::Mode::Visual(_)));
        app.workspace.active_editor_mut().unwrap().set_cursor(4);
        after_pointer(&mut app);
        assert_eq!(app.vim.mode(), vim::Mode::Normal);
        app.workspace
            .active_editor_mut()
            .unwrap()
            .select_range(1..3);
        after_pointer(&mut app);
        assert!(matches!(app.vim.mode(), vim::Mode::Visual(_)));
        keys(&mut app, "d");
        assert_eq!(text(&app), "adef");
    }
}
