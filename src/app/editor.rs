//! Application editor commands preserve one shared Workspace transaction.
use super::editor_vim as vim_input;
use crate::app::context::AppContext;
use crate::app::state::AppState;
use crate::app::state::editor::{DragSelection, ImeSession, SelectionUnit};
use crate::ui::editor::view::*;
use crate::ui::fonts;
use crate::ui::theme;
use lgui::core::PointerButton;
use lgui::prelude::UiRect;
use loom_editor_core::commands::{self, Command};
use loom_editor_core::normal;
use loom_editor_core::vim;
use std::ops::Range;
const CURSOR_REVEAL_MARGIN: f32 = 12.0;
pub fn drag_scrollbars(app: &mut AppState, rect: UiRect, pointer_x: f32, pointer_y: f32) -> bool {
    if !app.editor.editor_vertical_scrollbar_dragging
        && !app.editor.editor_horizontal_scrollbar_dragging
    {
        return false;
    }

    let code_left = rect.left + fonts::editor().gutter_w() + theme::CODE_PAD;
    let Some(metrics) = app
        .workspace
        .active_editor()
        .map(|buffer| scroll_metrics(rect, code_left, &buffer))
    else {
        return finish_scrollbar_drag(app);
    };
    drag_scrollbars_with_metrics(app, rect, code_left, metrics, pointer_x, pointer_y)
}

pub(crate) fn drag_scrollbars_with_metrics(
    app: &mut AppState,
    rect: UiRect,
    code_left: f32,
    metrics: ScrollMetrics,
    pointer_x: f32,
    pointer_y: f32,
) -> bool {
    let (mut scroll_x, mut scroll_y) = app.workspace.active_scroll();

    if app.editor.editor_vertical_scrollbar_dragging && metrics.max_y > 0.0 {
        let corner = if metrics.max_x > 0.0 {
            SCROLLBAR_SIZE
        } else {
            0.0
        };
        let track_top = rect.top;
        let track_length = (rect.bottom - corner) - track_top;
        let geometry = thumb_geometry(
            track_top,
            track_length,
            metrics.viewport_h,
            metrics.content_h,
            scroll_y,
        );
        let travel = track_length - geometry.length;
        let thumb_start =
            (pointer_y - track_top - app.editor.editor_vertical_scrollbar_drag_offset)
                .clamp(0.0, travel);
        scroll_y = scroll_from_thumb(thumb_start, travel, metrics.max_y);
    }

    if app.editor.editor_horizontal_scrollbar_dragging && metrics.max_x > 0.0 {
        let corner = if metrics.max_y > 0.0 {
            SCROLLBAR_SIZE
        } else {
            0.0
        };
        let track_left = code_left;
        let track_length = (rect.right - corner) - track_left;
        let geometry = thumb_geometry(
            track_left,
            track_length,
            metrics.viewport_w,
            metrics.content_w,
            scroll_x,
        );
        let travel = track_length - geometry.length;
        let thumb_start =
            (pointer_x - track_left - app.editor.editor_horizontal_scrollbar_drag_offset)
                .clamp(0.0, travel);
        scroll_x = scroll_from_thumb(thumb_start, travel, metrics.max_x);
    }

    let previous = app.workspace.active_scroll();
    app.workspace.set_active_scroll(scroll_x, scroll_y);
    previous != app.workspace.active_scroll()
}

pub fn finish_scrollbar_drag(app: &mut AppState) -> bool {
    let changed = app.editor.editor_vertical_scrollbar_dragging
        || app.editor.editor_horizontal_scrollbar_dragging;
    app.editor.editor_vertical_scrollbar_dragging = false;
    app.editor.editor_horizontal_scrollbar_dragging = false;
    changed
}

pub(crate) fn reveal_cursor(app: &mut AppState, rect: UiRect) {
    let Some(buffer) = app.workspace.active_editor() else {
        return;
    };
    let (line, column) = buffer.line_col();
    let code_left = rect.left + fonts::editor().gutter_w() + theme::CODE_PAD;
    let metrics = scroll_metrics(rect, code_left, &buffer);
    let (mut scroll_x, mut scroll_y) = app.workspace.active_scroll();

    let line_top = line as f32 * fonts::editor().line_h;
    let line_bottom = line_top + fonts::editor().line_h;
    if line_top < scroll_y {
        scroll_y = line_top;
    } else if line_bottom > scroll_y + metrics.viewport_h {
        scroll_y = line_bottom - metrics.viewport_h;
    }

    let caret = document_caret(
        &buffer,
        line,
        column,
        code_left,
        rect.top,
        code_left + metrics.content_w,
        (scroll_x, metrics.viewport_w),
    ) - code_left;
    if caret < scroll_x + CURSOR_REVEAL_MARGIN {
        scroll_x = (caret - CURSOR_REVEAL_MARGIN).max(0.0);
    } else if caret > scroll_x + metrics.viewport_w - CURSOR_REVEAL_MARGIN {
        scroll_x = caret - metrics.viewport_w + CURSOR_REVEAL_MARGIN;
    }

    app.workspace.set_active_scroll(
        scroll_x.clamp(0.0, metrics.max_x),
        scroll_y.clamp(0.0, metrics.max_y),
    );
}

pub fn execute_command(
    state: &AppContext,
    command: Command,
    rect: UiRect,
    clipboard: lgui::services::ClipboardHandle,
) {
    state.update(|app| {
        if command == Command::Escape
            && (app.editor.menu.take().is_some()
                || app.editor.editor_hunk_peek.take().is_some()
                || super::language_ui::dismiss(app))
        {
            return;
        }
        app.editor.menu = None;
        app.editor.drag = None;
        // Text selected in a hover card or the list's documentation is what
        // the person means to copy.
        if command == Command::Copy
            && let Some(text) = super::language_ui::selected_text(app)
        {
            if let Err(error) = clipboard.write_text(&text) {
                app.show_error(format!("Clipboard: {error}"));
            }
            return;
        }
        // Copy and Cut of a Vim visual selection go through Vim's registers.
        if matches!(command, Command::Copy | Command::Cut)
            && vim_input::visual_clipboard(app, command == Command::Cut, rect, clipboard.as_ref())
        {
            return;
        }
        if matches!(command, Command::Undo | Command::Redo) {
            super::language_ui::snippet_session::end(app);
        }
        if matches!(command, Command::Undo | Command::Redo)
            && let Some(outcome) =
                super::language_ui::edit::undo_redo(app, command == Command::Undo)
        {
            if let Err(error) = outcome {
                app.show_toast(error);
            }
            reveal_cursor(app, rect);
            return;
        }
        match apply_command(app, command, clipboard.as_ref()) {
            Ok(pasted) => vim_input::record_insert(app, insert_event(command, pasted)),
            Err(error) => app.show_error(format!("Clipboard: {error}")),
        }
        vim_input::after_command(app, matches!(command, Command::Undo | Command::Redo));
        reveal_cursor(app, rect);
    });
}

fn insert_event(command: Command, pasted: Option<String>) -> Option<vim::InsertEvent> {
    use vim::InsertEvent;
    Some(match command {
        Command::Delete(false, false) => InsertEvent::Backspace,
        Command::Delete(true, false) => InsertEvent::Delete,
        Command::Delete(false, true) => InsertEvent::DeleteWordBack,
        Command::Delete(true, true) => InsertEvent::DeleteWordForward,
        Command::Enter => InsertEvent::Newline,
        Command::Indent(false) => InsertEvent::Tab,
        Command::Indent(true) => InsertEvent::Backtab,
        Command::Paste => InsertEvent::Paste(pasted?),
        _ => return None,
    })
}

pub(crate) fn apply_command(
    app: &mut AppState,
    command: Command,
    clipboard: &dyn lgui::services::Clipboard,
) -> Result<Option<String>, lgui::services::ClipboardError> {
    if app.workspace.active_editor().is_none() {
        return Ok(None);
    }
    if let Some(view) = app.workspace.active() {
        super::editing::configure(app, view);
    }
    match command {
        Command::Copy | Command::Cut => {
            let selected = app
                .workspace
                .active_editor()
                .and_then(|buffer| buffer.selected_text())
                .map(str::to_owned);
            if let Some(value) = selected {
                // A failed copy must leave the source text and selection untouched.
                clipboard.write_text(&value)?;
                if command == Command::Cut {
                    app.workspace.promote_active_preview();
                }
                if let Some(mut buffer) = app.workspace.active_editor_mut() {
                    buffer.break_undo_group();
                    if command == Command::Cut {
                        normal::backspace(&mut buffer);
                        buffer.break_undo_group();
                    }
                }
            }
        }
        Command::Paste => {
            if let Some(value) = clipboard.read_text()? {
                let changes_text = !value.is_empty()
                    || app
                        .workspace
                        .active_editor()
                        .is_some_and(|buffer| buffer.selection().is_some());
                if changes_text {
                    app.workspace.promote_active_preview();
                }
                if let Some(mut buffer) = app.workspace.active_editor_mut() {
                    normal::paste(&mut buffer, &value);
                }
                return Ok(Some(value));
            }
        }
        Command::ToggleComment => {
            let prefix = app
                .workspace
                .active()
                .and_then(|id| app.workspace.view_meta(id))
                .and_then(|m| m.lang.definition().comments.line.as_deref());
            if let Some(prefix) = prefix {
                app.workspace.promote_active_preview();
                if let Some(mut buffer) = app.workspace.active_editor_mut() {
                    normal::comment(&mut buffer, prefix);
                }
            } else {
                app.show_toast("This language has no line comment marker");
            }
        }
        Command::Line(..)
        | Command::Delete(..)
        | Command::Enter
        | Command::Indent(..)
        | Command::Undo
        | Command::Redo => {
            let changes_text = match command {
                Command::Undo => app
                    .workspace
                    .active_editor()
                    .is_some_and(|editor| editor.can_undo()),
                Command::Redo => app
                    .workspace
                    .active_editor()
                    .is_some_and(|editor| editor.can_redo()),
                _ => true,
            };
            if changes_text {
                app.workspace.promote_active_preview();
            }
            if let Some(mut buffer) = app.workspace.active_editor_mut() {
                commands::apply(&mut buffer, command);
            }
        }
        _ => {
            if let Some(mut buffer) = app.workspace.active_editor_mut() {
                commands::apply(&mut buffer, command);
            }
        }
    }
    Ok(None)
}

pub(crate) fn update_drag_selection(app: &mut AppState, rect: UiRect) {
    let Some(drag) = app.editor.drag.clone() else {
        return;
    };
    if app.workspace.active() != Some(drag.document) {
        app.editor.drag = None;
        return;
    }
    let (sx, sy) = app.workspace.active_scroll();
    let left = rect.left + fonts::editor().gutter_w() + theme::CODE_PAD;
    if let Some(mut buffer) = app.workspace.active_editor_mut() {
        let metrics = scroll_metrics(rect, left, &buffer);
        let cursor = cursor_offset_from_point(
            &buffer,
            rect,
            left,
            metrics,
            sx,
            sy,
            drag.point.0,
            drag.point.1,
        );
        match drag.unit {
            SelectionUnit::Character => {
                buffer.set_cursor(drag.origin.start);
                buffer.select_to(cursor, true);
            }
            SelectionUnit::Word | SelectionUnit::Line => {
                let range = if drag.unit == SelectionUnit::Word {
                    buffer.word_range(cursor)
                } else {
                    buffer.line_range_with_break(line_index_from_point(
                        drag.point.1,
                        rect.top,
                        sy,
                        buffer.line_count(),
                    ))
                };
                if range.start < drag.origin.start {
                    buffer.set_cursor(drag.origin.end);
                    buffer.select_to(range.start, true);
                } else {
                    buffer.set_cursor(drag.origin.start);
                    buffer.select_to(range.end.max(drag.origin.end), true);
                }
            }
        }
    }
    vim_input::after_pointer(app);
}

pub fn drag_scroll_tick(app: &mut AppState, rect: UiRect) -> bool {
    let Some(drag) = app.editor.drag.as_ref() else {
        return false;
    };
    if app.workspace.active() != Some(drag.document) {
        app.editor.drag = None;
        return true;
    }
    let left = rect.left + fonts::editor().gutter_w() + theme::CODE_PAD;
    let speed = |p: f32, low: f32, high: f32| {
        if p < low {
            -(low - p).clamp(4.0, 80.0)
        } else if p > high {
            (p - high).clamp(4.0, 80.0)
        } else {
            0.0
        }
    };
    let dx = if drag.unit == SelectionUnit::Line {
        0.0
    } else {
        speed(drag.point.0, left, rect.right - SCROLLBAR_SIZE)
    };
    let dy = speed(drag.point.1, rect.top, rect.bottom - SCROLLBAR_SIZE);
    if dx == 0.0 && dy == 0.0 {
        return false;
    }
    let Some(buffer) = app.workspace.active_editor() else {
        return false;
    };
    let metrics = scroll_metrics(rect, left, &buffer);
    let (x, y) = app.workspace.active_scroll();
    let next = (
        (x + dx).clamp(0.0, metrics.max_x),
        (y + dy).clamp(0.0, metrics.max_y),
    );
    if next == (x, y) {
        return false;
    }
    app.workspace.set_active_scroll(next.0, next.1);
    update_drag_selection(app, rect);
    true
}

pub(crate) fn update_composition(app: &mut AppState, text: &str, cursor: Option<Range<usize>>) {
    let pane = app.workspace.active_pane();
    let Some(document) = app.workspace.active() else {
        return;
    };
    let Some(view) = app.workspace.view(document).filter(|v| v.accepts_text()) else {
        return;
    };
    let input_generation = view.input_generation;
    match app.editor.ime.as_mut() {
        Some(session) => {
            // An emptied composition has ended; a new one starts where the
            // focus is now.
            if session.text.is_empty() {
                (session.pane, session.document) = (pane, document);
                session.input_generation = input_generation;
            }
            session.text = text.to_owned();
            session.cursor = cursor;
        }
        None if !text.is_empty() => {
            app.editor.ime = Some(ImeSession {
                pane,
                input_generation,
                document,
                text: text.to_owned(),
                cursor,
            });
        }
        None => {}
    }
}

pub fn insert_text(app: &mut AppState, input: &str, rect: UiRect) {
    app.editor.language.rect = Some(rect);
    if !input.is_empty() && (!vim_input::is_active(app) || app.editor.vim.is_inserting()) {
        super::language_ui::completion::note_typed(app, input);
    }
    app.editor.drag = None;
    app.editor.menu = None;
    let session = app.editor.ime.take();
    let pane = session
        .as_ref()
        .map_or(app.workspace.active_pane(), |session| session.pane);
    let Some(document) = session
        .as_ref()
        .map(|session| session.document)
        .or_else(|| app.workspace.active())
    else {
        return;
    };
    if app.workspace.view(document).is_none_or(|v| {
        !v.accepts_text()
            || session
                .as_ref()
                .is_some_and(|s| s.input_generation != v.input_generation)
    }) {
        return;
    }
    let pane = app.workspace.pane_of(document).unwrap_or(pane);
    let in_active_view =
        app.workspace.active_pane() == pane && app.workspace.active() == Some(document);
    if in_active_view && vim_input::is_active(app) {
        if vim_input::handle_text(app, input, rect) {
            return;
        }
        // Text in Normal or Visual mode is not a command. Only a composition
        // the input method commits late is still typed.
        if session.is_none() {
            return;
        }
    }
    if !input.is_empty() {
        app.workspace.promote_preview(pane, document);
    }
    super::editing::configure(app, document);
    let language = app
        .workspace
        .view_meta(document)
        .map(|m| m.lang.name())
        .unwrap_or("text");
    if let Some(mut editor) = app.workspace.view_editor_mut(pane, document) {
        // A composition commit is an undo step of its own.
        if session.is_some() {
            editor.break_undo_group();
        }
        if session.is_some() {
            normal::insert(&mut editor, input);
        } else {
            normal::type_text(&mut editor, input, language);
        }
        if session.is_some() {
            editor.break_undo_group();
        }
    }
    if app.workspace.active_pane() == pane && app.workspace.active() == Some(document) {
        reveal_cursor(app, rect);
    }
}

#[cfg(test)]
#[path = "editor_view_tests.rs"]
mod tests;

pub(crate) fn pointer_down(
    app: &mut AppState,
    pane: crate::domain::identity::PaneId,
    id: crate::domain::identity::ViewId,
    rect: UiRect,
    code_left: f32,
    metrics: ScrollMetrics,
    point: (f32, f32),
    button: PointerButton,
) {
    let point = lgui::core::Point::new(point.0, point.1);

    app.editor.caret_dragging = false;
    app.editor.editor_hunk_peek = None;
    app.workspace.activate_pane(pane);
    let (scroll_x, scroll_y) = app.workspace.scroll(pane);
    let cursor = app.workspace.active_editor().map(|buffer| {
        cursor_offset_from_point(
            &buffer, rect, code_left, metrics, scroll_x, scroll_y, point.x, point.y,
        )
    });
    if let Some(cursor) = cursor
        && let Some(mut buffer) = app.workspace.active_editor_mut()
    {
        if button == PointerButton::Right {
            if !buffer.selection().is_some_and(|r| r.contains(&cursor)) {
                buffer.set_cursor(cursor);
            }
            app.editor.menu = Some((point.x, point.y));
            app.editor.menu_hover = None;
            app.editor.drag = None;
            app.editor.tabs.tab_context_menu = None;
            app.explorer.ui.menu = None;
            app.explorer.ui.menu_target = None;
            app.explorer.ui.menu_hover = None;
        } else {
            app.editor.menu = None;
            let clicks = app.editor.click_count(id, point.x, point.y);
            let shift = app.editor.modifiers.shift();
            let unit = if point.x < code_left || clicks == 3 {
                SelectionUnit::Line
            } else if clicks == 2 {
                SelectionUnit::Word
            } else {
                SelectionUnit::Character
            };
            let origin = match unit {
                SelectionUnit::Line => {
                    let line =
                        line_index_from_point(point.y, rect.top, scroll_y, buffer.line_count());
                    buffer.line_range_with_break(line)
                }
                SelectionUnit::Word => buffer.word_range(cursor),
                SelectionUnit::Character => {
                    buffer.select_to(cursor, shift);
                    // Dragging extends from where the selection is anchored.
                    let anchor = buffer.selection_state().anchor.unwrap_or(cursor);
                    anchor..anchor
                }
            };
            if unit != SelectionUnit::Character {
                buffer.select_range(origin.clone());
            }
            app.editor.drag = Some(DragSelection {
                document: id,
                origin,
                unit,
                point: (point.x, point.y),
            });
        }
    }
    vim_input::after_pointer(app);
    reveal_cursor(app, rect);
}

#[cfg(test)]
fn ime_cursor_rect(app: &AppState, rect: UiRect) -> UiRect {
    let view = super::editor_view::snapshot(
        app.clone(),
        crate::features::git::GitStoreSnapshot::default(),
        app.workspace.active_pane(),
        rect,
    );
    crate::ui::editor::view::ime_cursor_rect(&view, rect)
}
