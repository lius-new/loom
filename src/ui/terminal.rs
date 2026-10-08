//! Bottom terminal panel backed by a real platform PTY.

use std::cell::Cell;
use std::sync::Arc;

use lgui::ApplicationHandle;
use lgui::core::{
    CursorIcon, EventPolicy, IconStyle, KeyState, KeyboardEvent, LogicalKey, NamedKey, Point,
    PointerButton, SemanticRole, Semantics, UiElement, UiFocusHandle, UiId, WheelUnit, clip,
    precompiled,
};
use lgui::prelude::{Color, Element, State, UiRect, VisualStyle, group, panel, text};
use lgui::services::{Clipboard, ClipboardError, ServicesContextExt};
use lgui::text::{self as text_layout, TextLayoutRequest};

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
const BODY_PAD_X: f32 = 12.0;
const BODY_PAD_Y: f32 = 6.0;
const TERMINAL_FONT_SIZE: f32 = 12.0;
const FALLBACK_CHAR_W: f32 = TERMINAL_FONT_SIZE * 0.6;
const TERMINAL_LINE_H: f32 = 19.0;
const RESIZE_HIT_H: f32 = 8.0;
const SHELL_MENU_W: f32 = 172.0;
const SHELL_MENU_ROW_H: f32 = 28.0;

thread_local! {
    // Only cache a renderer measurement. An early call without a text system
    // must not pin the fallback width for the lifetime of the window.
    static CELL_WIDTH: Cell<Option<f32>> = const { Cell::new(None) };
}

fn terminal_cell_width() -> f32 {
    CELL_WIDTH.with(|cached| {
        if let Some(width) = cached.get() {
            return width;
        }
        const SAMPLE: &str = "0000000000000000";
        let mut request = TextLayoutRequest::single_line(
            SAMPLE,
            UiRect::new(0.0, 0.0, 10_000.0, TERMINAL_LINE_H),
            TERMINAL_FONT_SIZE,
            400,
        );
        request.font_families = theme::MONO_FAMILIES;
        let measured = text_layout::layout(&request)
            .map(|layout| layout.width / SAMPLE.len() as f32)
            .filter(|width| width.is_finite() && *width > 0.0);
        if let Some(width) = measured {
            cached.set(Some(width));
            width
        } else {
            FALLBACK_CHAR_W
        }
    })
}

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
    state.update(|app| {
        app.show_terminal = !app.show_terminal;
        app.terminal_tab_context_menu = None;
    });
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
    pty_size_for_body(UiRect::new(
        rect.left,
        rect.top + HEADER_H,
        rect.right,
        rect.bottom,
    ))
}

pub fn pane_rects(rect: UiRect, count: usize) -> Vec<UiRect> {
    if count == 0 {
        return Vec::new();
    }
    (0..count)
        .map(|index| {
            UiRect::new(
                rect.left
                    + rect.width() * index as f32 / count as f32
                    + if index > 0 { 1.0 } else { 0.0 },
                rect.top + HEADER_H,
                rect.left + rect.width() * (index + 1) as f32 / count as f32,
                rect.bottom,
            )
        })
        .collect()
}

pub fn pty_size_for_body(rect: UiRect) -> TerminalSize {
    let cell_width = terminal_cell_width();
    let body_w = (rect.width() - BODY_PAD_X * 2.0).max(cell_width);
    let body_h = (rect.height() - BODY_PAD_Y * 2.0).max(TERMINAL_LINE_H);
    TerminalSize {
        rows: (body_h / TERMINAL_LINE_H)
            .floor()
            .clamp(1.0, u16::MAX as f32) as u16,
        cols: (body_w / cell_width).floor().clamp(2.0, u16::MAX as f32) as u16,
        pixel_width: cell_width.round().max(1.0) as u16,
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
    let sessions = terminal_tab_snapshot.active_group();
    let body_rects = pane_rects(rect, sessions.len());
    let active_index = sessions
        .iter()
        .position(|session| session.id == active_tab_id)
        .unwrap_or(0);
    let active_body = body_rects.get(active_index).copied().unwrap_or(body);
    let active_content = UiRect::new(
        active_body.left + BODY_PAD_X,
        active_body.top + BODY_PAD_Y,
        active_body.right - BODY_PAD_X,
        active_body.bottom - BODY_PAD_Y,
    );
    let cursor_rect = terminal_cursor_rect(
        active_content,
        snapshot.cursor,
        snapshot.rows,
        snapshot.cols,
    );
    let mut terminal = Element::new(move |cx| {
        UiElement::panel(terminal_id, rect, VisualStyle::filled(theme::c().bg))
            .ime_cursor_rect(cursor_rect)
            .semantics(Semantics::new(SemanticRole::TextInput).name("Terminal"))
            .children(cx.children)
    })
    .event_policy(EventPolicy::INTERACTIVE);
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

    let tool_right = rect.right - 38.0;
    let add_tabs = terminal_tabs.clone();
    let add_focus = terminal_focus.clone();
    let add_shell = snapshot.shell;
    let add_cwd = terminal_tab_snapshot
        .active()
        .and_then(|tab| tab.cwd.clone())
        .or_else(|| app_state.workspace_folders.first().cloned());
    let add_rect = UiRect::new(
        rect.right - 32.0,
        header.top + 5.0,
        rect.right - 6.0,
        header.bottom - 5.0,
    );
    let add_center_x = (add_rect.left + add_rect.right) * 0.5;
    let add_center_y = (add_rect.top + add_rect.bottom) * 0.5;
    terminal = terminal.child(
        panel(
            add_rect,
            VisualStyle::filled(theme::c().surface).radius(3.0),
        )
        .key("terminal-new-tab")
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            let cwd = add_cwd.clone();
            add_tabs.update(move |tabs| {
                tabs.add_at(add_shell, cwd);
            });
            add_focus.focus();
        })
        .child(icon(
            "terminal.new.icon",
            "plus",
            UiRect::new(
                add_center_x - 6.0,
                add_center_y - 6.0,
                add_center_x + 6.0,
                add_center_y + 6.0,
            ),
            theme::c().text_soft,
        )),
    );
    let shell_label = snapshot.shell.short_label();
    // Reserve the leading icon and trailing arrow around the shaped label.
    let instance_width =
        (25.0 + measure(shell_label, theme::SMALL, 400) + TEXT_MARGIN + 22.0).ceil();
    let instance_rect = UiRect::new(
        tool_right - instance_width,
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
            shell_label,
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

    for (session, session_rect) in sessions.iter().zip(pane_rects(rect, sessions.len())) {
        let active = session.id == active_tab_id;
        let id = UiId::owned(format!("terminal-session-{}", session.id));
        terminal = terminal.child(render_session(
            session_rect,
            state.clone(),
            terminal_tabs.clone(),
            terminal_focus.clone(),
            id,
            session.controller.clone(),
            session.id,
            terminal_focused && active,
            application.clone(),
            cursor_blink_visible,
        ));
        if session_rect.left > body.left {
            terminal = terminal.child(panel(
                UiRect::new(
                    session_rect.left - 1.0,
                    body.top,
                    session_rect.left,
                    body.bottom,
                ),
                VisualStyle::filled(theme::c().border),
            ));
        }
    }

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
fn render_session(
    body: UiRect,
    state: State<AppState>,
    terminal_tabs: State<TerminalTabs>,
    terminal_focus: UiFocusHandle,
    terminal_id: UiId,
    controller: TerminalController,
    session_id: u64,
    focused: bool,
    application: Arc<ApplicationHandle>,
    cursor_blink_visible: bool,
) -> Element {
    let rect = body;
    let snapshot = controller.snapshot();
    let content = UiRect::new(
        body.left + BODY_PAD_X,
        body.top + BODY_PAD_Y,
        body.right - BODY_PAD_X,
        body.bottom - BODY_PAD_Y,
    );
    let cursor_rect = terminal_cursor_rect(content, snapshot.cursor, snapshot.rows, snapshot.cols);
    let mut semantics = Semantics::new(SemanticRole::TextInput)
        .name(format!("{} terminal", snapshot.shell.label()))
        .description("Interactive integrated terminal");
    semantics.state.multiline = true;

    let mut terminal = Element::new(move |cx| {
        UiElement::panel(terminal_id, rect, VisualStyle::filled(theme::c().bg))
            .ime_cursor_rect(cursor_rect)
            .children(cx.children)
    })
    .event_policy(EventPolicy {
        hover: true,
        press: true,
        focus: false,
    })
    .semantics(semantics)
    .cursor(CursorIcon::Text);

    let focus_on_click = terminal_focus.clone();
    let click_tabs = terminal_tabs.clone();
    terminal = terminal.on_click(move || {
        click_tabs.update(move |tabs| tabs.select(session_id));
        focus_on_click.focus();
    });

    let (screen_rows, screen_cols) = (snapshot.rows, snapshot.cols);
    let select_controller = controller.clone();
    let select_tabs = terminal_tabs.clone();
    let select_focus = terminal_focus.clone();
    let select_state = state.clone();
    let select_application = application.clone();
    terminal = terminal.on_pointer_down_with_button(move |cx, pointer, button| {
        if !content.contains(pointer.point) {
            return;
        }
        select_tabs.update(move |tabs| tabs.select(session_id));
        select_focus.focus();
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

    terminal.child(render_screen(
        content,
        &snapshot,
        focused,
        cursor_blink_visible,
    ))
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

    let labels = tabs
        .iter()
        .map(|tab| {
            let siblings = tabs
                .iter()
                .filter(|other| other.group_id == tab.group_id)
                .collect::<Vec<_>>();
            if siblings.len() > 1 {
                let index = siblings
                    .iter()
                    .position(|other| other.id == tab.id)
                    .unwrap();
                format!("{} {}", tab.controller.shell().short_label(), index + 1)
            } else {
                tab.controller.shell().short_label().to_owned()
            }
        })
        .collect::<Vec<_>>();
    let close_width = measure("✕", theme::SMALL, 400);
    let fixed_width = TAB_PAD + TAB_CONTENT_GAP + close_width + TAB_PAD;
    let widths = labels
        .iter()
        .map(|name| {
            (fixed_width + measure(name, theme::UI_SIZE, 400))
                .min(tab_cap)
                .max(MIN_TAB_W)
        })
        .collect::<Vec<_>>();
    let mut group_left = rect.left + TABS_PAD;
    let mut group_start = 0;
    while group_start < tabs.len() {
        let group_id = tabs[group_start].group_id;
        let group_end = tabs[group_start..]
            .iter()
            .position(|tab| tab.group_id != group_id)
            .map_or(tabs.len(), |offset| group_start + offset);
        let width: f32 = widths[group_start..group_end].iter().sum();
        let bounds = UiRect::new(
            group_left,
            rect.top + PILL_INSET,
            group_left + width,
            rect.bottom - PILL_INSET,
        );
        let active_group = tabs[group_start..group_end]
            .iter()
            .any(|tab| tab.id == active_id);
        strip = strip.child(theme::bordered(
            bounds,
            if active_group {
                theme::c().bg
            } else {
                theme::c().sidebar
            },
            theme::c().border,
            2.0,
            1.0,
        ));
        group_left += width + TAB_GAP;
        group_start = group_end;
    }
    let mut left = rect.left + TABS_PAD;
    for (index, tab) in tabs.iter().enumerate() {
        let name = &labels[index];
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
        let visual = group(tab_rect)
            .child(panel(
                UiRect::new(
                    tab_rect.left + 3.0,
                    tab_rect.top + 3.0,
                    tab_rect.right - 3.0,
                    tab_rect.bottom - 3.0,
                ),
                if active {
                    VisualStyle::filled(theme::c().active_line).radius(2.0)
                } else {
                    VisualStyle::default()
                },
            ))
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
        let select_tabs = terminal_tabs.clone();
        let select_focus = terminal_focus.clone();
        let menu_state = state.clone();
        let tab_id = tab.id;
        let mut tab_element = group(tab_rect)
            .key(format!("terminal-tab-{tab_id}"))
            .on_event_capture(lgui::core::UiEventKind::PointerDown, move |cx, payload| {
                if let lgui::core::UiEventPayload::PointerDown {
                    pointer,
                    button: PointerButton::Right,
                } = payload
                {
                    menu_state.update(move |app| {
                        app.editor.menu = None;
                        app.context_menu = None;
                        app.context_menu_target = None;
                        app.tab_context_menu = None;
                        app.terminal_shell_menu = false;
                        app.terminal_tab_context_menu =
                            Some(crate::state::TerminalTabContextMenuState {
                                position: (pointer.point.x, pointer.point.y),
                                target: tab_id,
                            });
                    });
                    cx.stop_propagation();
                }
            })
            .child(visual);
        tab_element = tab_element.child(
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
        tab_element = tab_element.child(
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

        strip = strip.child(tab_element);
        let same_group = tabs
            .get(index + 1)
            .is_some_and(|next| next.group_id == tab.group_id);
        if same_group {
            strip = strip.child(panel(
                UiRect::new(
                    tab_rect.right - 0.5,
                    tab_rect.top + 4.0,
                    tab_rect.right + 0.5,
                    tab_rect.bottom - 4.0,
                ),
                VisualStyle::filled(theme::c().border),
            ));
        }
        left += tab_width + if same_group { 0.0 } else { TAB_GAP };
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
            let left = content.left + f32::from(start_col) * terminal_cell_width();
            let right =
                (content.left + f32::from(end_col) * terminal_cell_width()).min(content.right);
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
                // Keep a terminal run on one row even when shaping/rounding
                // makes its natural width slightly exceed the allocated cells.
                let measured = crate::ui::components::text::SingleLineText::new(
                    run.text.clone(),
                    UiRect::new(left, top, left + 100_000.0, top + TERMINAL_LINE_H),
                    style,
                );
                let text_rect = UiRect::new(
                    left,
                    top,
                    left + measured.width().max(right - left) + 2.0,
                    top + TERMINAL_LINE_H,
                );
                screen = screen.child(clip(run_rect, 0.0, 0.0).child(text(
                    text_rect,
                    run.text.clone(),
                    style,
                )));
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
        let cursor = terminal_cursor_rect(content, snapshot.cursor, snapshot.rows, snapshot.cols);
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
    let cell_width = terminal_cell_width();
    let left = content.left + f32::from(run.start_col) * cell_width;
    let right = (left + f32::from(run.columns) * cell_width + 2.0).min(content.right + 2.0);
    (left, right)
}

/// Maps a pointer to the nearest cell boundary, clamped to the screen.
fn cell_at(content: UiRect, point: Point, rows: u16, cols: u16) -> (u16, u16) {
    let row = ((point.y - content.top) / TERMINAL_LINE_H)
        .floor()
        .clamp(0.0, f32::from(rows.saturating_sub(1)));
    let col = ((point.x - content.left) / terminal_cell_width())
        .round()
        .clamp(0.0, f32::from(cols));
    (row as u16, col as u16)
}

/// The terminal's clipboard actions (`terminal::Copy` and friends).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardShortcut {
    Copy,
    /// Copy when text is selected and otherwise interrupt, matching Windows Terminal.
    CopyOrInterrupt,
    Paste,
}

pub fn run_clipboard_shortcut(
    controller: &TerminalController,
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
        ClipboardShortcut::Paste => paste_clipboard(controller, clipboard),
    }
}

/// Type replayed text from an abandoned key sequence into the shell.
pub fn write_text(controller: &TerminalController, text: &str) {
    controller.write(text.as_bytes());
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

fn terminal_cursor_rect(content: UiRect, cursor: (u16, u16), rows: u16, cols: u16) -> UiRect {
    let cell_width = terminal_cell_width();
    // vt100 keeps col == cols while a full line waits for the next character
    // to trigger wrapping. The visible cursor stays on the last cell meanwhile.
    let left = content.left + f32::from(cursor.1.min(cols.saturating_sub(1))) * cell_width;
    let top = content.top + f32::from(cursor.0.min(rows.saturating_sub(1))) * TERMINAL_LINE_H;
    UiRect::new(
        left,
        top + 2.0,
        left + cell_width,
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
    fn split_panes_select_independently_preserve_drag_selection_and_route_input() {
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{InputEvent, PointerData, UiScale, dispatch_runtime_output};
        use lgui::session::UiSession;
        use std::sync::Mutex;

        let exposed = Arc::new(Mutex::new(None::<(State<AppState>, State<TerminalTabs>)>));
        let output = exposed.clone();
        let application = Arc::new(ApplicationHandle::new(|task| task(), || {}));
        let viewport = UiRect::new(0.0, 0.0, 600.0, 300.0);
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state_with(AppState::new);
            let tabs = cx.state_with(|| {
                let mut tabs = TerminalTabs::new();
                tabs.split(1);
                tabs
            });
            *output.lock().unwrap() = Some((state.clone(), tabs.clone()));
            let id = UiId::new("split-terminal-input");
            let focus = cx.focus_handle(id.clone());
            let editor_id = cx.use_stable_id();
            let editor_focus = cx.focus_handle(editor_id.clone());
            let controller = tabs.get().active().unwrap().controller.clone();
            group(viewport)
                .child(
                    Element::new(move |cx| {
                        UiElement::panel(
                            editor_id,
                            UiRect::new(0.0, 260.0, 600.0, 300.0),
                            VisualStyle::default(),
                        )
                        .children(cx.children)
                    })
                    .event_policy(EventPolicy::INTERACTIVE),
                )
                .child(render(
                    UiRect::new(0.0, 0.0, 600.0, 260.0),
                    state,
                    editor_focus,
                    focus,
                    id,
                    controller,
                    tabs,
                    application.clone(),
                    true,
                    260.0,
                ))
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let (state, tabs) = exposed.lock().unwrap().clone().unwrap();
        assert_eq!(
            session
                .tree()
                .node(&UiId::new("split-terminal-input"))
                .unwrap()
                .ime_cursor_rect
                .unwrap()
                .left,
            313.0
        );
        assert!(
            session
                .tree()
                .nodes()
                .iter()
                .any(|node| node.text.as_deref() == Some("pwsh 1"))
        );
        assert!(
            session
                .tree()
                .nodes()
                .iter()
                .any(|node| node.text.as_deref() == Some("pwsh 2"))
        );
        let context = ApplicationContext::empty(Default::default());
        let mut send = |input| {
            let events = session.handle_input(input);
            dispatch_runtime_output(
                events,
                &context,
                &lgui::window::WindowId::new("terminal-split-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
            session.render_view(&view, viewport, UiScale::ONE);
            session
                .tree()
                .node(&UiId::new("split-terminal-input"))
                .unwrap()
                .ime_cursor_rect
                .unwrap()
        };
        let start = PointerData::mouse(Point::new(50.0, 80.0));
        let caret = send(InputEvent::PointerDown {
            pointer: start,
            button: PointerButton::Left,
        });
        assert_eq!(tabs.get().active_id(), Some(1));
        assert_eq!(caret.left, 12.0);
        let end = PointerData::mouse(Point::new(85.0, 80.0));
        send(InputEvent::PointerMove(end));
        send(InputEvent::PointerUp {
            pointer: end,
            button: PointerButton::Left,
        });
        assert!(!state.get().selecting_terminal);
        let snapshot = tabs.get();
        let left = snapshot.tabs()[0].controller.clone();
        let right = snapshot.tabs()[1].controller.clone();
        assert!(!left.snapshot().selection.is_empty());
        assert!(right.snapshot().selection.is_empty());
        right.begin_selection(0, 0);
        right.extend_selection(0, 2);
        send(InputEvent::TextInput("left terminal input".into()));
        assert!(left.snapshot().selection.is_empty());
        assert!(!right.snapshot().selection.is_empty());

        let add = PointerData::mouse(Point::new(585.0, 17.0));
        send(InputEvent::PointerDown {
            pointer: add,
            button: PointerButton::Left,
        });
        send(InputEvent::PointerUp {
            pointer: add,
            button: PointerButton::Left,
        });
        assert_eq!(tabs.get().tabs().len(), 3);
        assert_eq!(tabs.get().active_group().len(), 1);
        assert_eq!(tabs.get().saved_groups(), vec![0, 0, 1]);
        let panes = pane_rects(UiRect::new(0.0, 0.0, 600.0, 260.0), 2);
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].right + 1.0, panes[1].left);
        assert!(
            pty_size_for_body(panes[0]).cols < pty_size(UiRect::new(0.0, 0.0, 600.0, 260.0)).cols
        );
    }

    #[test]
    fn tab_context_menu_targets_inactive_tabs_and_blocks_shell_input_until_dismissed() {
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{InputEvent, UiScale, dispatch_runtime_output};
        use lgui::session::UiSession;
        use std::sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        };

        let exposed = Arc::new(Mutex::new(None::<(State<AppState>, State<TerminalTabs>)>));
        let output = exposed.clone();
        let writes = Arc::new(AtomicUsize::new(0));
        let received = writes.clone();
        let application = Arc::new(ApplicationHandle::new(|task| task(), || {}));
        let viewport = UiRect::new(0.0, 0.0, 600.0, 300.0);
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state_with(|| {
                let mut app = AppState::new();
                app.show_terminal = true;
                app
            });
            let tabs = cx.state_with(|| {
                let mut tabs = TerminalTabs::new();
                tabs.add(ShellKind::Cmd);
                tabs.add(ShellKind::Bash);
                tabs
            });
            *output.lock().unwrap() = Some((state.clone(), tabs.clone()));
            let terminal_id = cx.use_stable_id();
            let terminal_focus = cx.focus_handle(terminal_id.clone());
            let editor_id = cx.use_stable_id();
            let editor_focus = cx.focus_handle(editor_id.clone());
            crate::ui::terminal_tab_context_menu::restore_focus_on_dismiss(
                cx,
                state.get().terminal_tab_context_menu.is_some(),
                state.get().show_terminal,
                terminal_focus.clone(),
                editor_focus.clone(),
            );
            let received = received.clone();
            let terminal = Element::new(move |cx| {
                UiElement::panel(
                    terminal_id,
                    UiRect::new(0.0, 40.0, 600.0, 200.0),
                    VisualStyle::default(),
                )
                .auto_focus()
                .children(cx.children)
            })
            .event_policy(EventPolicy::INTERACTIVE)
            .on_input(move |_, _| {
                received.fetch_add(1, Ordering::AcqRel);
            });
            let editor = Element::new(move |cx| {
                UiElement::panel(
                    editor_id,
                    UiRect::new(0.0, 200.0, 600.0, 300.0),
                    VisualStyle::default(),
                )
                .children(cx.children)
            })
            .event_policy(EventPolicy::INTERACTIVE);
            let snapshot = tabs.get();
            let mut root = group(viewport).child(editor);
            if state.get().show_terminal {
                root = root.child(terminal);
            }
            if let Some(active_id) = snapshot.active_id() {
                root = root.child(render_terminal_tabs(
                    UiRect::new(0.0, 0.0, 600.0, 34.0),
                    snapshot.tabs(),
                    active_id,
                    tabs.clone(),
                    state.clone(),
                    terminal_focus.clone(),
                    editor_focus.clone(),
                    application.clone(),
                ));
            }
            if state.get().terminal_tab_context_menu.is_some() {
                root = root.child(crate::ui::terminal_tab_context_menu::render(
                    viewport,
                    state,
                    tabs,
                    application.clone(),
                    terminal_focus,
                    editor_focus,
                ));
            }
            root
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let (state, tabs) = exposed.lock().unwrap().clone().unwrap();
        let context = ApplicationContext::empty(Default::default());
        let mut send = |input| {
            let events = session.handle_input(input);
            dispatch_runtime_output(
                events,
                &context,
                &lgui::window::WindowId::new("terminal-menu-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
            session.render_view(&view, viewport, UiScale::ONE);
            session.runtime().run_effects();
            session.render_view(&view, viewport, UiScale::ONE);
        };
        let point = lgui::core::PointerData::mouse(Point::new(30.0, 10.0));
        send(InputEvent::PointerDown {
            pointer: point,
            button: PointerButton::Right,
        });
        send(InputEvent::PointerUp {
            pointer: point,
            button: PointerButton::Right,
        });
        assert_eq!(state.get().terminal_tab_context_menu.unwrap().target, 1);
        assert_eq!(tabs.get().active_id(), Some(3));
        send(InputEvent::TextInput("must not reach shell".into()));
        assert_eq!(writes.load(Ordering::Acquire), 0);
        send(InputEvent::Keyboard(key(
            LogicalKey::Named(NamedKey::Escape),
            Default::default(),
        )));
        assert!(state.get().terminal_tab_context_menu.is_none());
        send(InputEvent::TextInput("shell input restored".into()));
        assert_eq!(writes.load(Ordering::Acquire), 1);

        send(InputEvent::PointerDown {
            pointer: point,
            button: PointerButton::Right,
        });
        send(InputEvent::PointerUp {
            pointer: point,
            button: PointerButton::Right,
        });
        let others = lgui::core::PointerData::mouse(Point::new(45.0, 97.0));
        send(InputEvent::PointerDown {
            pointer: others,
            button: PointerButton::Left,
        });
        send(InputEvent::PointerUp {
            pointer: others,
            button: PointerButton::Left,
        });
        assert!(state.get().terminal_tab_context_menu.is_none());
        assert_eq!(tabs.get().tabs().len(), 1);
        assert_eq!(tabs.get().active_id(), Some(1));
        send(InputEvent::PointerDown {
            pointer: point,
            button: PointerButton::Right,
        });
        send(InputEvent::PointerUp {
            pointer: point,
            button: PointerButton::Right,
        });
        let outside = lgui::core::PointerData::mouse(Point::new(500.0, 250.0));
        send(InputEvent::PointerDown {
            pointer: outside,
            button: PointerButton::Left,
        });
        send(InputEvent::PointerUp {
            pointer: outside,
            button: PointerButton::Left,
        });
        assert!(state.get().terminal_tab_context_menu.is_none());
        send(InputEvent::TextInput(
            "outside dismissal restores input".into(),
        ));
        assert_eq!(writes.load(Ordering::Acquire), 2);

        send(InputEvent::PointerDown {
            pointer: point,
            button: PointerButton::Right,
        });
        send(InputEvent::PointerUp {
            pointer: point,
            button: PointerButton::Right,
        });
        let split = lgui::core::PointerData::mouse(Point::new(45.0, 47.0));
        send(InputEvent::PointerDown {
            pointer: split,
            button: PointerButton::Left,
        });
        send(InputEvent::PointerUp {
            pointer: split,
            button: PointerButton::Left,
        });
        assert_eq!(tabs.get().active_group().len(), 2);
        assert_eq!(tabs.get().saved_groups(), vec![0, 0]);

        send(InputEvent::PointerDown {
            pointer: point,
            button: PointerButton::Right,
        });
        send(InputEvent::PointerUp {
            pointer: point,
            button: PointerButton::Right,
        });
        let new = lgui::core::PointerData::mouse(Point::new(45.0, 25.0));
        send(InputEvent::PointerDown {
            pointer: new,
            button: PointerButton::Left,
        });
        send(InputEvent::PointerUp {
            pointer: new,
            button: PointerButton::Left,
        });
        assert_eq!(tabs.get().tabs().len(), 3);
        assert_eq!(tabs.get().active_id(), Some(5));
        assert_eq!(tabs.get().saved_groups(), vec![0, 0, 1]);
        assert_eq!(
            tabs.get().active().unwrap().controller.shell(),
            ShellKind::default()
        );

        send(InputEvent::PointerDown {
            pointer: point,
            button: PointerButton::Right,
        });
        send(InputEvent::PointerUp {
            pointer: point,
            button: PointerButton::Right,
        });
        let all = lgui::core::PointerData::mouse(Point::new(45.0, 140.0));
        send(InputEvent::PointerDown {
            pointer: all,
            button: PointerButton::Left,
        });
        send(InputEvent::PointerUp {
            pointer: all,
            button: PointerButton::Left,
        });
        assert!(tabs.get().is_empty());
        assert!(!state.get().show_terminal);
        assert!(state.get().terminal_tab_context_menu.is_none());
    }

    #[test]
    fn measured_cell_width_drives_cursor_runs_hit_testing_and_pty_size() {
        struct RestoreWidth(Option<f32>);
        impl Drop for RestoreWidth {
            fn drop(&mut self) {
                CELL_WIDTH.with(|cached| cached.set(self.0));
            }
        }
        // Simulate a renderer font advance that differs from the old 7.2px.
        let _restore = RestoreWidth(CELL_WIDTH.with(|cached| cached.replace(Some(6.25))));
        let content = UiRect::new(10.0, 20.0, 2010.0, 100.0);
        let mut parser = vt100::Parser::new(4, 300, 0);
        for count in 1..=200 {
            parser.process(b"x");
            let cursor = terminal_cursor_rect(content, parser.screen().cursor_position(), 4, 300);
            assert_eq!(cursor.left, content.left + count as f32 * 6.25);
            let run = TerminalRun {
                start_col: 0,
                columns: count,
                text: "x".repeat(usize::from(count)),
                style: Default::default(),
            };
            assert_eq!(run_span(content, &run).1 - 2.0, cursor.left);
            assert_eq!(
                cell_at(content, Point::new(cursor.left, content.top), 4, 300),
                (0, count)
            );
        }
        assert_eq!(pty_size(UiRect::new(0.0, 0.0, 400.0, 160.0)).cols, 60);
    }

    #[test]
    fn cursor_remains_visible_while_a_full_line_waits_to_wrap() {
        let width = terminal_cell_width();
        let content = UiRect::new(0.0, 0.0, width * 8.0, TERMINAL_LINE_H * 2.0);
        let mut parser = vt100::Parser::new(2, 8, 0);
        parser.process(b"12345678");
        assert_eq!(parser.screen().cursor_position(), (0, 8));
        let cursor = terminal_cursor_rect(content, parser.screen().cursor_position(), 2, 8);
        assert!((cursor.right - content.right).abs() < 0.001);
        assert_eq!(cursor.top, 2.0);
        parser.process(b"9");
        let cursor = terminal_cursor_rect(content, parser.screen().cursor_position(), 2, 8);
        assert_eq!(cursor.left, width);
        assert_eq!(cursor.top, TERMINAL_LINE_H + 2.0);
    }

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
                point(
                    10.0 + terminal_cell_width() * 2.6,
                    20.0 + TERMINAL_LINE_H * 1.5
                ),
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
