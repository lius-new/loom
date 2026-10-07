//! Bottom terminal panel backed by a real platform PTY.

use std::sync::Arc;

use lgui::ApplicationHandle;
use lgui::core::{
    CursorIcon, EventPolicy, IconStyle, KeyState, KeyboardEvent, LogicalKey, NamedKey, Point,
    PointerButton, SemanticRole, Semantics, UiElement, UiFocusHandle, UiId, WheelUnit, clip,
    precompiled,
};
use lgui::prelude::{Color, Element, State, UiRect, VisualStyle, group, panel, text};
use lgui::services::{Clipboard, ClipboardError, ServicesContextExt};

use crate::state::AppState;
use crate::terminal_session::{
    ShellKind, TerminalController, TerminalRun, TerminalSize, TerminalTab, TerminalTabs,
};
use crate::theme;
use crate::ui::tabs::{
    GAP as TAB_CONTENT_GAP, MAX_TAB_W, MIN_TAB_W, PAD as TAB_PAD, PILL_INSET, TAB_GAP, TABS_PAD,
    TEXT_MARGIN, ellipsize, measure,
};

const HEADER_H: f32 = 34.0;
const INSTANCE_W: f32 = 132.0;
const BODY_PAD_X: f32 = 12.0;
const BODY_PAD_Y: f32 = 6.0;
const TERMINAL_FONT_SIZE: f32 = 12.0;
const TERMINAL_CHAR_W: f32 = 7.2;
const TERMINAL_LINE_H: f32 = 19.0;
const RESIZE_HIT_H: f32 = 8.0;
const SHELL_MENU_W: f32 = 172.0;
const SHELL_MENU_ROW_H: f32 = 28.0;

/// Toggle the shared terminal panel, creating its first session on demand and
/// transferring focus to the surface that becomes active.
pub fn toggle_panel(
    state: &State<AppState>,
    terminal_tabs: &State<TerminalTabs>,
    editor_focus: &UiFocusHandle,
    terminal_focus: &UiFocusHandle,
) {
    let opening = !state.get().show_terminal;
    if opening && terminal_tabs.get().is_empty() {
        let shell = state.get().default_shell;
        terminal_tabs.update(move |tabs| {
            tabs.add(shell);
        });
    }
    state.update(|app| app.show_terminal = !app.show_terminal);
    if opening {
        terminal_focus.focus();
    } else {
        editor_focus.focus();
    }
}

fn icon(id: &'static str, key: &'static str, rect: UiRect, color: Color) -> Element {
    precompiled(UiElement::icon(UiId::new(id), rect, key).icon_style(IconStyle::new(color)))
}

pub fn pty_size(rect: UiRect) -> TerminalSize {
    let body_w = (rect.width() - BODY_PAD_X * 2.0).max(TERMINAL_CHAR_W);
    let body_h = (rect.height() - HEADER_H - BODY_PAD_Y * 2.0).max(TERMINAL_LINE_H);
    TerminalSize {
        rows: (body_h / TERMINAL_LINE_H)
            .floor()
            .clamp(1.0, u16::MAX as f32) as u16,
        cols: (body_w / TERMINAL_CHAR_W)
            .floor()
            .clamp(2.0, u16::MAX as f32) as u16,
        pixel_width: TERMINAL_CHAR_W.round() as u16,
        pixel_height: TERMINAL_LINE_H.round() as u16,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    rect: UiRect,
    state: State<AppState>,
    editor_focus: UiFocusHandle,
    terminal_focus: UiFocusHandle,
    terminal_id: UiId,
    controller: TerminalController,
    terminal_tabs: State<TerminalTabs>,
    application: Arc<ApplicationHandle>,
    cursor_blink_visible: bool,
    max_height: f32,
) -> Element {
    let app_state = state.get();
    let resizing = app_state.resizing_terminal;
    let terminal_focused = app_state.terminal_focused;
    let shell_menu_open = app_state.terminal_shell_menu;
    let snapshot = controller.snapshot();
    let terminal_tab_snapshot = terminal_tabs.get();
    let active_tab_id = terminal_tab_snapshot
        .active_id()
        .expect("a visible terminal panel must have an active tab");
    let tab_items = terminal_tab_snapshot.tabs().to_vec();
    let header = UiRect::new(rect.left, rect.top + 1.0, rect.right, rect.top + HEADER_H);
    let body = UiRect::new(rect.left, header.bottom, rect.right, rect.bottom);
    let content = UiRect::new(
        body.left + BODY_PAD_X,
        body.top + BODY_PAD_Y,
        body.right - BODY_PAD_X,
        body.bottom - BODY_PAD_Y,
    );
    let cursor_rect = terminal_cursor_rect(content, snapshot.cursor);
    let mut semantics = Semantics::new(SemanticRole::TextInput)
        .name(format!("{} terminal", snapshot.shell.label()))
        .description("Interactive integrated terminal");
    semantics.state.multiline = true;

    let mut terminal = Element::new(move |cx| {
        UiElement::panel(terminal_id, rect, VisualStyle::filled(theme::c().bg))
            .ime_cursor_rect(cursor_rect)
            .children(cx.children)
    })
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(semantics)
    .cursor(CursorIcon::Text);

    let focus_on_click = terminal_focus.clone();
    terminal = terminal.on_click(move || focus_on_click.focus());

    let (screen_rows, screen_cols) = (snapshot.rows, snapshot.cols);
    let select_controller = controller.clone();
    let select_state = state.clone();
    let select_application = application.clone();
    terminal = terminal.on_pointer_down_with_button(move |cx, pointer, button| {
        if !content.contains(pointer.point) {
            return;
        }
        let (row, col) = cell_at(content, pointer.point, screen_rows, screen_cols);
        match button {
            PointerButton::Left => {
                select_controller.begin_selection(row, col);
                select_state.update(|app| app.selecting_terminal = true);
            }
            // Right click copies a selection, or pastes when there is none.
            PointerButton::Right => {
                let clipboard = cx.application().clipboard();
                let result = match copy_selection(&select_controller, clipboard.as_ref()) {
                    Ok(true) => Ok(()),
                    Ok(false) => paste_clipboard(&select_controller, clipboard.as_ref()),
                    Err(error) => Err(error),
                };
                report_clipboard_error(&select_state, result);
            }
            _ => return,
        }
        select_application.request_frame();
    });

    let drag_controller = controller.clone();
    let drag_state = state.clone();
    let drag_application = application.clone();
    terminal = terminal.on_pointer_drag(move |cx, pointer| {
        if drag_state.get().selecting_terminal {
            let (row, col) = cell_at(content, pointer.point, screen_rows, screen_cols);
            drag_controller.extend_selection(row, col);
            drag_application.request_frame();
            cx.stop_propagation();
        }
    });
    let select_up_state = state.clone();
    terminal = terminal.on_pointer_up(move |_cx, _pointer| {
        select_up_state.try_update(|app| std::mem::take(&mut app.selecting_terminal));
    });

    let input_controller = controller.clone();
    let input_application = application.clone();
    terminal = terminal.on_input(move |cx, input| {
        if !input.is_empty() {
            input_controller.write(input.as_bytes());
            input_application.request_frame();
            cx.stop_propagation();
        }
    });

    let key_controller = controller.clone();
    let key_application = application.clone();
    let application_cursor = snapshot.application_cursor;
    terminal = terminal.on_key_down(move |cx, event| {
        if let Some(bytes) = key_bytes(event, application_cursor) {
            key_controller.write(&bytes);
            key_application.request_frame();
            cx.prevent_default();
            cx.stop_propagation();
        }
    });

    let wheel_controller = controller.clone();
    let wheel_application = application.clone();
    terminal = terminal.on_wheel(move |cx, delta| {
        let lines = match delta.unit {
            WheelUnit::Lines => (delta.y * 3.0).round() as i32,
            WheelUnit::Pixels => (delta.y / TERMINAL_LINE_H).round() as i32,
        };
        if lines != 0 {
            wheel_controller.scroll(lines);
            wheel_application.request_frame();
            cx.stop_propagation();
        }
    });

    let focus_state = state.clone();
    terminal = terminal.on_focus(move |_cx| {
        focus_state.update(|app| {
            app.terminal_focused = true;
            app.focused = false;
        });
    });
    let blur_state = state.clone();
    terminal = terminal.on_blur(move |_cx| {
        blur_state.update(|app| app.terminal_focused = false);
    });

    // Crisp separation from the editor above and from terminal content below.
    terminal = terminal.child(panel(
        UiRect::new(rect.left, rect.top, rect.right, rect.top + 1.0),
        VisualStyle::filled(if resizing {
            theme::c().accent
        } else {
            theme::c().border
        }),
    ));
    terminal = terminal.child(panel(
        UiRect::new(rect.left, header.bottom - 1.0, rect.right, header.bottom),
        VisualStyle::filled(theme::c().border),
    ));

    let tool_right = rect.right - 6.0;
    let instance_rect = UiRect::new(
        tool_right - INSTANCE_W,
        header.top + 5.0,
        tool_right,
        header.bottom - 5.0,
    );

    let tab_strip_left = rect.left;
    let tab_strip = UiRect::new(
        tab_strip_left,
        header.top,
        (instance_rect.left - 8.0).max(tab_strip_left),
        header.bottom,
    );
    terminal = terminal.child(render_terminal_tabs(
        tab_strip,
        &tab_items,
        active_tab_id,
        terminal_tabs.clone(),
        state.clone(),
        terminal_focus.clone(),
        editor_focus.clone(),
        application.clone(),
    ));

    let menu_state = state.clone();
    let menu_focus = terminal_focus.clone();
    terminal = terminal.child(
        panel(
            instance_rect,
            VisualStyle::filled(theme::c().surface).radius(3.0),
        )
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            menu_state.update(|app| app.terminal_shell_menu = !app.terminal_shell_menu);
            menu_focus.focus();
        })
        .child(icon(
            "terminal.instance.icon",
            "terminal",
            UiRect::new(
                instance_rect.left + 7.0,
                instance_rect.top + 5.0,
                instance_rect.left + 19.0,
                instance_rect.top + 17.0,
            ),
            theme::c().text_muted,
        ))
        .child(text(
            UiRect::new(
                instance_rect.left + 25.0,
                instance_rect.top,
                instance_rect.right - 22.0,
                instance_rect.bottom,
            ),
            snapshot.shell.short_label(),
            theme::mono(theme::c().text_soft, theme::SMALL),
        ))
        .child(icon(
            "terminal.instance.chevron",
            "chevron-down",
            UiRect::new(
                instance_rect.right - 17.0,
                instance_rect.top + 5.0,
                instance_rect.right - 5.0,
                instance_rect.top + 17.0,
            ),
            theme::c().text_dim,
        )),
    );

    terminal = terminal.child(render_screen(
        content,
        &snapshot,
        terminal_focused,
        cursor_blink_visible,
    ));

    if shell_menu_open {
        terminal = terminal.child(shell_menu(
            instance_rect,
            state.clone(),
            terminal_focus.clone(),
            terminal_tabs.clone(),
        ));
    }

    let handle = UiRect::new(
        rect.left,
        rect.top - RESIZE_HIT_H / 2.0,
        rect.right,
        rect.top + RESIZE_HIT_H / 2.0,
    );
    let panel_bottom = rect.bottom;
    let resize_down = state.clone();
    let resize_drag = state.clone();
    let resize_up = state;
    terminal = terminal.child(
        panel(handle, VisualStyle::default())
            .key("terminal-resize-handle")
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(CursorIcon::ResizeVertical)
            .on_pointer_down_with_button(move |_cx, _pointer, button| {
                if button == PointerButton::Left {
                    resize_down.update(|app| app.resizing_terminal = true);
                }
            })
            .on_pointer_drag(move |_cx, pointer| {
                resize_drag.update(move |app| {
                    if app.resizing_terminal {
                        app.terminal_h = resized_height(panel_bottom, pointer.point.y, max_height);
                    }
                });
            })
            .on_pointer_up(move |_cx, _pointer| {
                resize_up.update(|app| app.resizing_terminal = false);
            }),
    );

    terminal
}

#[allow(clippy::too_many_arguments)]
fn render_terminal_tabs(
    rect: UiRect,
    tabs: &[TerminalTab],
    active_id: u64,
    terminal_tabs: State<TerminalTabs>,
    state: State<AppState>,
    terminal_focus: UiFocusHandle,
    editor_focus: UiFocusHandle,
    application: Arc<ApplicationHandle>,
) -> Element {
    let mut strip = group(rect);
    if tabs.is_empty() || rect.width() <= 0.0 {
        return strip;
    }

    let count = tabs.len() as f32;
    let available_tabs_width = (rect.width() - TABS_PAD).max(MIN_TAB_W);
    let tab_cap =
        ((available_tabs_width - TAB_GAP * (count - 1.0)) / count).clamp(MIN_TAB_W, MAX_TAB_W);

    let mut left = rect.left + TABS_PAD;
    for tab in tabs {
        let name = tab.controller.shell().short_label();
        let name_width = measure(name, theme::UI_SIZE, 400);
        let close_width = measure("✕", theme::SMALL, 400);
        let fixed_width = TAB_PAD + TAB_CONTENT_GAP + close_width + TAB_PAD;
        let tab_width = (fixed_width + name_width).min(tab_cap).max(MIN_TAB_W);
        let tab_rect = UiRect::new(
            left,
            rect.top + PILL_INSET,
            left + tab_width,
            rect.bottom - PILL_INSET,
        );
        let name_left = tab_rect.left + TAB_PAD;
        let close_left = tab_rect.right - TAB_PAD - close_width;
        let close_rect = UiRect::new(
            close_left,
            tab_rect.top,
            tab_rect.right - TAB_PAD,
            tab_rect.bottom,
        );
        let name_slot_width = (close_left - TAB_CONTENT_GAP - name_left).max(0.0);
        let display_name = ellipsize(
            name,
            (name_slot_width - TEXT_MARGIN).max(0.0),
            theme::UI_SIZE,
            400,
        );
        let label_rect = UiRect::new(
            name_left,
            tab_rect.top,
            name_left + name_slot_width,
            tab_rect.bottom,
        );
        let active = tab.id == active_id;
        let visual = if active {
            theme::bordered(tab_rect, theme::c().bg, theme::c().border, 2.0, 1.0)
        } else {
            panel(tab_rect, VisualStyle::default().radius(2.0))
        }
        .child(text(
            label_rect,
            display_name,
            theme::mono(
                if active {
                    theme::c().text_bright
                } else {
                    theme::c().text_muted
                },
                theme::UI_SIZE,
            ),
        ));
        strip = strip.child(visual);

        let select_tabs = terminal_tabs.clone();
        let select_focus = terminal_focus.clone();
        let tab_id = tab.id;
        strip = strip.child(
            panel(
                UiRect::new(
                    tab_rect.left,
                    tab_rect.top,
                    (close_left - TAB_CONTENT_GAP).max(tab_rect.left),
                    tab_rect.bottom,
                ),
                VisualStyle::default(),
            )
            .key(format!("terminal-tab-select-{tab_id}"))
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(CursorIcon::Pointer)
            .on_click(move || {
                select_tabs.update(move |tabs| tabs.select(tab_id));
                select_focus.focus();
            }),
        );

        let close_controller = tab.controller.clone();
        let close_tabs = terminal_tabs.clone();
        let close_state = state.clone();
        let close_terminal_focus = terminal_focus.clone();
        let close_editor_focus = editor_focus.clone();
        let close_application = application.clone();
        strip = strip.child(
            panel(close_rect, VisualStyle::default())
                .key(format!("terminal-tab-close-{tab_id}"))
                .event_policy(EventPolicy::INTERACTIVE)
                .cursor(CursorIcon::Pointer)
                .on_click(move || {
                    close_controller.terminate(&close_application);
                    let closing_last_tab = close_tabs.get().tabs().len() == 1;
                    close_tabs.update(move |tabs| {
                        tabs.close(tab_id);
                    });
                    if closing_last_tab {
                        close_state.update(|app| {
                            app.show_terminal = false;
                            app.terminal_shell_menu = false;
                            app.terminal_focused = false;
                        });
                        close_editor_focus.focus();
                    } else {
                        close_terminal_focus.focus();
                    }
                })
                .child(text(
                    UiRect::new(
                        close_left,
                        tab_rect.top,
                        close_left + close_width + TEXT_MARGIN,
                        tab_rect.bottom,
                    ),
                    "✕",
                    theme::mono(theme::c().text_dim, theme::SMALL),
                )),
        );

        left += tab_width + TAB_GAP;
    }

    clip(rect, 0.0, 0.0).child(strip)
}

fn render_screen(
    content: UiRect,
    snapshot: &crate::terminal_session::TerminalSnapshot,
    focused: bool,
    cursor_blink_visible: bool,
) -> Element {
    let mut screen = group(content);
    for (row, runs) in snapshot.lines.iter().enumerate() {
        let top = content.top + row as f32 * TERMINAL_LINE_H;
        if top >= content.bottom {
            break;
        }
        for run in runs {
            if let Some(background) = run.style.background {
                let (left, right) = run_span(content, run);
                screen = screen.child(panel(
                    UiRect::new(left, top, right, top + TERMINAL_LINE_H),
                    VisualStyle::filled(Color(background)),
                ));
            }
        }
        // Selection sits above cell backgrounds and below the glyphs.
        for &(_, start_col, end_col) in snapshot
            .selection
            .iter()
            .filter(|(selected_row, ..)| usize::from(*selected_row) == row)
        {
            let left = content.left + f32::from(start_col) * TERMINAL_CHAR_W;
            let right = (content.left + f32::from(end_col) * TERMINAL_CHAR_W).min(content.right);
            screen = screen.child(panel(
                UiRect::new(left, top, right, top + TERMINAL_LINE_H),
                VisualStyle::filled(theme::c().selection),
            ));
        }
        for run in runs {
            let (left, right) = run_span(content, run);
            let run_rect = UiRect::new(left, top, right, top + TERMINAL_LINE_H);
            if !run.text.is_empty() {
                let foreground = run.style.foreground.map(Color).unwrap_or(theme::c().text);
                let mut style = if run.style.bold {
                    theme::mono_bold(foreground, TERMINAL_FONT_SIZE)
                } else {
                    theme::mono(foreground, TERMINAL_FONT_SIZE)
                };
                if run.style.dim {
                    style = style.alpha(0x90);
                }
                screen = screen.child(text(run_rect, run.text.clone(), style));
                if run.style.underline {
                    screen = screen.child(panel(
                        UiRect::new(
                            left,
                            top + TERMINAL_LINE_H - 3.0,
                            right,
                            top + TERMINAL_LINE_H - 2.0,
                        ),
                        VisualStyle::filled(foreground),
                    ));
                }
            }
        }
    }

    if cursor_is_drawn(snapshot.cursor_visible, focused, cursor_blink_visible) {
        let cursor = terminal_cursor_rect(content, snapshot.cursor);
        let cursor_style = if focused {
            VisualStyle::filled(theme::c().text_soft).alpha(0xc8)
        } else {
            VisualStyle::default().stroked(theme::hairline(theme::c().text_dim))
        };
        screen = screen.child(panel(cursor, cursor_style));
    }

    if let Some(message) = snapshot.status.message() {
        let top = (content.bottom - TERMINAL_LINE_H).max(content.top);
        screen = screen.child(text(
            UiRect::new(content.left, top, content.right, top + TERMINAL_LINE_H),
            message,
            theme::mono(theme::c().text_dim, theme::SMALL),
        ));
    }

    clip(content, 0.0, 0.0).child(screen)
}

fn run_span(content: UiRect, run: &TerminalRun) -> (f32, f32) {
    let left = content.left + f32::from(run.start_col) * TERMINAL_CHAR_W;
    let right = (left + f32::from(run.columns) * TERMINAL_CHAR_W + 2.0).min(content.right + 2.0);
    (left, right)
}

/// Maps a pointer to the nearest cell boundary, clamped to the screen.
fn cell_at(content: UiRect, point: Point, rows: u16, cols: u16) -> (u16, u16) {
    let row = ((point.y - content.top) / TERMINAL_LINE_H)
        .floor()
        .clamp(0.0, f32::from(rows.saturating_sub(1)));
    let col = ((point.x - content.left) / TERMINAL_CHAR_W)
        .round()
        .clamp(0.0, f32::from(cols));
    (row as u16, col as u16)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClipboardShortcut {
    Copy,
    CopyOrInterrupt,
    Paste,
}

fn clipboard_shortcut_for(event: &KeyboardEvent) -> Option<ClipboardShortcut> {
    if event.state != KeyState::Down || event.modifiers.alt() {
        return None;
    }
    let ctrl = event.modifiers.ctrl();
    let shift = event.modifiers.shift();
    match &event.key {
        LogicalKey::Character(value) if ctrl && value.eq_ignore_ascii_case("c") => Some(if shift {
            ClipboardShortcut::Copy
        } else {
            ClipboardShortcut::CopyOrInterrupt
        }),
        LogicalKey::Character(value) if ctrl && value.eq_ignore_ascii_case("v") => {
            Some(ClipboardShortcut::Paste)
        }
        LogicalKey::Named(NamedKey::Insert) if shift && !ctrl => Some(ClipboardShortcut::Paste),
        _ => None,
    }
}

/// Handles the terminal's copy/paste keys. Ctrl+C copies when text is
/// selected and otherwise interrupts, matching Windows Terminal.
///
/// Returns `None` when the event is not a clipboard shortcut.
pub fn handle_clipboard_shortcut(
    controller: &TerminalController,
    event: &KeyboardEvent,
    clipboard: &dyn Clipboard,
) -> Option<Result<(), ClipboardError>> {
    Some(match clipboard_shortcut_for(event)? {
        ClipboardShortcut::Copy => copy_selection(controller, clipboard).map(|_| ()),
        ClipboardShortcut::CopyOrInterrupt => copy_selection(controller, clipboard).map(|copied| {
            if !copied {
                controller.write(b"\x03");
            }
        }),
        ClipboardShortcut::Paste => paste_clipboard(controller, clipboard),
    })
}

fn copy_selection(
    controller: &TerminalController,
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
    controller: &TerminalController,
    clipboard: &dyn Clipboard,
) -> Result<(), ClipboardError> {
    if let Some(value) = clipboard.read_text()? {
        controller.paste(&value);
    }
    Ok(())
}

pub fn report_clipboard_error(state: &State<AppState>, result: Result<(), ClipboardError>) {
    if let Err(error) = result {
        state.update(|app| app.show_error(format!("Clipboard: {error}")));
    }
}

fn cursor_is_drawn(pty_visible: bool, focused: bool, blink_visible: bool) -> bool {
    pty_visible && (!focused || blink_visible)
}

fn shell_menu(
    instance_rect: UiRect,
    state: State<AppState>,
    focus: UiFocusHandle,
    terminal_tabs: State<TerminalTabs>,
) -> Element {
    let menu_rect = UiRect::new(
        instance_rect.right - SHELL_MENU_W,
        instance_rect.bottom + 4.0,
        instance_rect.right,
        instance_rect.bottom + 4.0 + SHELL_MENU_ROW_H * ShellKind::ALL.len() as f32 + 8.0,
    );
    let mut menu = theme::bordered(menu_rect, theme::c().surface, theme::c().border, 4.0, 1.0);
    for (index, shell) in ShellKind::ALL.into_iter().enumerate() {
        let top = menu_rect.top + 4.0 + index as f32 * SHELL_MENU_ROW_H;
        let row = UiRect::new(
            menu_rect.left + 4.0,
            top,
            menu_rect.right - 4.0,
            top + SHELL_MENU_ROW_H,
        );
        let row_state = state.clone();
        let row_focus = focus.clone();
        let row_tabs = terminal_tabs.clone();
        let item = panel(row, VisualStyle::default().radius(3.0))
            .key(format!("terminal-shell-{}", shell.short_label()))
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(CursorIcon::Pointer)
            .on_click(move || {
                row_tabs.update(move |tabs| {
                    tabs.add(shell);
                });
                row_state.update(|app| app.terminal_shell_menu = false);
                row_focus.focus();
            });
        let item = item.child(text(
            UiRect::new(row.left + 8.0, row.top, row.right - 8.0, row.bottom),
            shell.label(),
            theme::mono(theme::c().text, theme::UI_SIZE),
        ));
        menu = menu.child(item);
    }
    menu
}

fn terminal_cursor_rect(content: UiRect, cursor: (u16, u16)) -> UiRect {
    let left = content.left + f32::from(cursor.1) * TERMINAL_CHAR_W;
    let top = content.top + f32::from(cursor.0) * TERMINAL_LINE_H;
    UiRect::new(
        left,
        top + 2.0,
        left + TERMINAL_CHAR_W,
        top + TERMINAL_LINE_H - 2.0,
    )
}

fn key_bytes(event: &KeyboardEvent, application_cursor: bool) -> Option<Vec<u8>> {
    if event.state != KeyState::Down {
        return None;
    }

    let shift = event.modifiers.shift();
    let alt = event.modifiers.alt();
    let ctrl = event.modifiers.ctrl();
    // xterm modifier parameter: 1 + Shift(1) + Alt(2) + Ctrl(4).
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    if let LogicalKey::Named(named) = &event.key {
        let cursor_final = match named {
            NamedKey::ArrowUp => Some('A'),
            NamedKey::ArrowDown => Some('B'),
            NamedKey::ArrowRight => Some('C'),
            NamedKey::ArrowLeft => Some('D'),
            NamedKey::Home => Some('H'),
            NamedKey::End => Some('F'),
            _ => None,
        };
        if let Some(final_byte) = cursor_final {
            return Some(
                if modifier > 1 {
                    format!("\x1b[1;{modifier}{final_byte}")
                } else if application_cursor {
                    format!("\x1bO{final_byte}")
                } else {
                    format!("\x1b[{final_byte}")
                }
                .into_bytes(),
            );
        }
        let tilde_code = match named {
            NamedKey::Insert => Some(2),
            NamedKey::Delete => Some(3),
            NamedKey::PageUp => Some(5),
            NamedKey::PageDown => Some(6),
            _ => None,
        };
        if let Some(code) = tilde_code {
            return Some(
                if modifier > 1 {
                    format!("\x1b[{code};{modifier}~")
                } else {
                    format!("\x1b[{code}~")
                }
                .into_bytes(),
            );
        }
        if *named == NamedKey::Backspace {
            // Ctrl+Backspace deletes a word (^H), Alt+Backspace sends ESC DEL.
            return Some(match (ctrl, alt) {
                (true, _) => b"\x08".to_vec(),
                (false, true) => b"\x1b\x7f".to_vec(),
                (false, false) => b"\x7f".to_vec(),
            });
        }
    }

    if event.modifiers.ctrl()
        && let LogicalKey::Character(value) = &event.key
        && let Some(character) = value.chars().next()
        && value.chars().count() == 1
    {
        let lower = character.to_ascii_lowercase();
        // These remain editor-wide shortcuts while the terminal is focused.
        if matches!(lower, 'p' | 'b' | 'g' | '`') {
            return None;
        }
        if lower.is_ascii_lowercase() {
            return Some(vec![(lower as u8) & 0x1f]);
        }
    }

    let bytes: &[u8] = match &event.key {
        LogicalKey::Named(NamedKey::Enter) => b"\r",
        LogicalKey::Named(NamedKey::Tab) if shift => b"\x1b[Z",
        LogicalKey::Named(NamedKey::Tab) => b"\t",
        LogicalKey::Named(NamedKey::Escape) => b"\x1b",
        _ => return None,
    };
    Some(bytes.to_vec())
}

fn resized_height(panel_bottom: f32, pointer_y: f32, max_height: f32) -> f32 {
    (panel_bottom - pointer_y).clamp(theme::TERMINAL_MIN_H, max_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focused_terminal_cursor_follows_blink_phase() {
        assert!(cursor_is_drawn(true, true, true));
        assert!(!cursor_is_drawn(true, true, false));
        assert!(cursor_is_drawn(true, false, false));
        assert!(!cursor_is_drawn(false, true, true));
    }

    fn key(key: LogicalKey, modifiers: lgui::core::KeyModifiers) -> KeyboardEvent {
        KeyboardEvent {
            state: KeyState::Down,
            key,
            modifiers,
            ..Default::default()
        }
    }

    #[test]
    fn clipboard_shortcuts_follow_windows_terminal_conventions() {
        use lgui::core::KeyModifiers;
        let ctrl = KeyModifiers::CONTROL;
        let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        let character = |value: &str| LogicalKey::Character(value.into());

        assert_eq!(
            clipboard_shortcut_for(&key(character("c"), ctrl)),
            Some(ClipboardShortcut::CopyOrInterrupt)
        );
        assert_eq!(
            clipboard_shortcut_for(&key(character("C"), ctrl_shift)),
            Some(ClipboardShortcut::Copy)
        );
        assert_eq!(
            clipboard_shortcut_for(&key(character("v"), ctrl)),
            Some(ClipboardShortcut::Paste)
        );
        assert_eq!(
            clipboard_shortcut_for(&key(
                LogicalKey::Named(NamedKey::Insert),
                KeyModifiers::SHIFT
            )),
            Some(ClipboardShortcut::Paste)
        );
        assert_eq!(clipboard_shortcut_for(&key(character("d"), ctrl)), None);
    }

    #[test]
    fn modified_navigation_keys_use_xterm_sequences() {
        use lgui::core::KeyModifiers;
        let left = LogicalKey::Named(NamedKey::ArrowLeft);
        assert_eq!(
            key_bytes(&key(left.clone(), KeyModifiers::empty()), false).unwrap(),
            b"\x1b[D"
        );
        assert_eq!(
            key_bytes(&key(left.clone(), KeyModifiers::empty()), true).unwrap(),
            b"\x1bOD"
        );
        assert_eq!(
            key_bytes(&key(left, KeyModifiers::CONTROL), false).unwrap(),
            b"\x1b[1;5D"
        );
        assert_eq!(
            key_bytes(
                &key(LogicalKey::Named(NamedKey::Delete), KeyModifiers::CONTROL),
                false
            )
            .unwrap(),
            b"\x1b[3;5~"
        );
        assert_eq!(
            key_bytes(
                &key(
                    LogicalKey::Named(NamedKey::Backspace),
                    KeyModifiers::CONTROL
                ),
                false
            )
            .unwrap(),
            b"\x08"
        );
    }

    #[test]
    fn pointer_maps_to_the_nearest_cell_boundary() {
        let content = UiRect::new(10.0, 20.0, 400.0, 200.0);
        let point = |x, y| Point { x, y };
        assert_eq!(cell_at(content, point(10.0, 20.0), 5, 40), (0, 0));
        assert_eq!(
            cell_at(
                content,
                point(10.0 + TERMINAL_CHAR_W * 2.6, 20.0 + TERMINAL_LINE_H * 1.5),
                5,
                40
            ),
            (1, 3)
        );
        assert_eq!(cell_at(content, point(-50.0, 900.0), 5, 40), (4, 0));
        assert_eq!(cell_at(content, point(9_000.0, 0.0), 5, 40), (0, 40));
    }

    #[test]
    fn terminal_resize_height_is_clamped_to_its_allowed_range() {
        assert_eq!(resized_height(600.0, 450.0, 420.0), 150.0);
        assert_eq!(resized_height(600.0, 550.0, 420.0), theme::TERMINAL_MIN_H);
        assert_eq!(resized_height(600.0, 100.0, 420.0), 420.0);
    }

    #[test]
    fn pty_size_tracks_the_visible_terminal_body() {
        let small = pty_size(UiRect::new(0.0, 0.0, 400.0, 160.0));
        let large = pty_size(UiRect::new(0.0, 0.0, 800.0, 320.0));
        assert!(large.cols > small.cols);
        assert!(large.rows > small.rows);
        assert!(small.rows > 0 && small.cols > 1);
    }
}
