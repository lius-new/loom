//! Bottom terminal panel backed by a real platform PTY.

use super::keys::{self, key_bytes};
use std::sync::Arc;

use super::bindings as commands;
use super::bindings::{
    Binding, Command, FrameRequest, SessionFrame as TerminalTab, Sessions, View as TerminalTabs,
};
use super::layout_binding::{MapBinding, MapCommand};

use lgui::core::{
    CursorIcon, EventPolicy, IconStyle, Point, PointerButton, SemanticRole, Semantics, UiElement,
    UiEventKind, UiEventPayload, UiFocusHandle, UiId, WheelUnit, clip,
};
use lgui::prelude::{Color, Element, UiRect, VisualStyle, component, group, panel, text};
use lgui::services::ServicesContextExt;

use crate::features::terminal::SessionId;
use crate::features::terminal::{TerminalRun, TerminalSize};
use crate::ui::components::scrollbar;
use crate::ui::fonts;
use crate::ui::tabs::{
    GAP as TAB_CONTENT_GAP, MAX_TAB_W, MIN_TAB_W, PAD as TAB_PAD, PILL_INSET, TAB_GAP, TABS_PAD,
    TEXT_MARGIN, ellipsize, measure,
};
use crate::ui::theme;

const HEADER_H: f32 = theme::TABS_H;
const BODY_PAD_X: f32 = 12.0;
const BODY_PAD_Y: f32 = 6.0;
const RESIZE_HIT_H: f32 = 8.0;
const SHELL_MENU_W: f32 = 172.0;
const SHELL_MENU_ROW_H: f32 = 28.0;

/// Observe the whole window so leaving a session also hides its scrollbar.
pub(crate) fn track_hover(
    root: Element,
    state: Binding,
    sessions: Vec<(SessionId, UiRect)>,
) -> Element {
    root.on_event_capture(UiEventKind::PointerMove, move |_, payload| {
        if let UiEventPayload::PointerMove { pointer } = payload {
            let hovered = sessions
                .iter()
                .find(|(_, rect)| rect.contains(pointer.point))
                .map(|(id, _)| *id);
            state.ui.try_update(|ui| {
                let changed = ui.hovered_terminal != hovered;
                ui.hovered_terminal = hovered;
                changed
            });
        }
    })
}

pub(crate) fn terminal_cell_width() -> f32 {
    fonts::terminal().char_w
}

pub(crate) fn terminal_line_h() -> f32 {
    fonts::terminal().line_h
}

pub(crate) fn icon(id: &'static str, key: &'static str, rect: UiRect, color: Color) -> Element {
    Element::new(move |cx| UiElement::icon(cx.id, rect, key).icon_style(IconStyle::new(color)))
        .key(id)
}

pub(crate) fn pty_size(rect: UiRect) -> TerminalSize {
    pty_size_for_body(UiRect::new(
        rect.left,
        rect.top + HEADER_H,
        rect.right,
        rect.bottom,
    ))
}

#[cfg(test)]
pub(crate) fn pane_rects(rect: UiRect, count: usize) -> Vec<UiRect> {
    weighted_pane_rects(rect, &vec![1.0; count])
}

pub(crate) fn session_rects(rect: UiRect, sessions: &[TerminalTab]) -> Vec<UiRect> {
    weighted_pane_rects(
        rect,
        &sessions
            .iter()
            .map(|session| session.width_weight)
            .collect::<Vec<_>>(),
    )
}

pub(crate) fn weighted_pane_rects(rect: UiRect, weights: &[f32]) -> Vec<UiRect> {
    let count = weights.len();
    if count == 0 {
        return Vec::new();
    }
    let minimum = crate::ui::pane_sash::PANE_MIN
        .width
        .min(rect.width().max(0.0) / count as f32);
    let remaining = (rect.width() - minimum * count as f32).max(0.0);
    let total: f32 = weights.iter().sum();
    let mut left = rect.left;
    (0..count)
        .map(|index| {
            let right = if index + 1 == count {
                rect.right
            } else {
                left + minimum + remaining * weights[index] / total
            };
            let pane = UiRect::new(
                left + if index > 0 { 1.0 } else { 0.0 },
                rect.top + HEADER_H,
                right,
                rect.bottom,
            );
            left = right;
            pane
        })
        .collect()
}

pub(crate) fn pty_size_for_body(rect: UiRect) -> TerminalSize {
    let cell_width = terminal_cell_width();
    let body_w = (rect.width() - BODY_PAD_X * 2.0).max(cell_width);
    let body_h = (rect.height() - BODY_PAD_Y * 2.0).max(terminal_line_h());
    TerminalSize {
        rows: (body_h / terminal_line_h())
            .floor()
            .clamp(1.0, u16::MAX as f32) as u16,
        cols: (body_w / cell_width).floor().clamp(2.0, u16::MAX as f32) as u16,
        pixel_width: cell_width.round().max(1.0) as u16,
        pixel_height: terminal_line_h().round() as u16,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_view(
    rect: UiRect,
    state: Binding,
    map: MapBinding,
    editor_focus: UiFocusHandle,
    terminal_focus: UiFocusHandle,
    terminal_id: UiId,
    controller: SessionId,
    terminal_tabs: Sessions,
    application: FrameRequest,
    cursor_blink_visible: bool,
    max_height: f32,
    region_view: Option<(u64, TerminalTabs, bool)>,
) -> Element {
    let app_state = state.get();
    let resizing = app_state.chrome.resizing_terminal;
    let primary = region_view.as_ref().is_none_or(|v| v.2);
    let terminal_focused = app_state.ui.terminal_focused && primary;
    let shell_menu_open = app_state.ui.terminal_shell_menu && primary;
    let snapshot = terminal_tabs.screen(controller);
    let terminal_tab_snapshot = region_view
        .as_ref()
        .map(|v| v.1.clone())
        .unwrap_or_else(|| terminal_tabs.get());
    let active_tab_id = terminal_tab_snapshot
        .active_id()
        .expect("a visible terminal panel must have an active tab");
    let tab_items = terminal_tab_snapshot.tabs().to_vec();
    let header = UiRect::new(rect.left, rect.top + 1.0, rect.right, rect.top + HEADER_H);
    let body = UiRect::new(rect.left, header.bottom, rect.right, rect.bottom);
    let sessions = terminal_tab_snapshot.active_group();
    let body_rects = session_rects(rect, &sessions);
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
    let input_tabs = terminal_tabs.clone();
    let input_application = application.clone();
    terminal = terminal.on_input(move |cx, input| {
        if !input.is_empty() {
            commands::write(&input_tabs, active_tab_id, input.as_bytes());
            input_application.request_frame();
            cx.stop_propagation();
        }
    });

    let key_tabs = terminal_tabs.clone();
    let key_application = application.clone();
    let application_cursor = snapshot.application_cursor;
    terminal = terminal.on_key_down(move |cx, event| {
        if let Some(bytes) = key_bytes(event, application_cursor) {
            commands::write(&key_tabs, active_tab_id, &bytes);
            key_application.request_frame();
            cx.prevent_default();
            cx.stop_propagation();
        }
    });

    let focus_state = state.clone();
    terminal = terminal.on_focus(move |_cx| {
        focus_state.run(Command::Focus);
    });
    let blur_state = state.clone();
    terminal = terminal.on_blur(move |_cx| {
        blur_state.ui.update(|app| app.terminal_focused = false);
    });

    terminal = terminal.child(panel(header, VisualStyle::filled(theme::c().sidebar)));

    let selected_shell = app_state.ui.terminal_shell_selection.and_then(|kind| {
        app_state
            .shell_options
            .iter()
            .find(|option| option.kind == kind && option.available())
    });
    let new_shell = selected_shell.map(|option| option.kind);
    let add_tabs = terminal_tabs.clone();
    let add_state = state.clone();
    let add_focus = terminal_focus.clone();
    let add_size = (header.height() - 6.0).max(0.0);
    let add_rect = UiRect::new(
        rect.right - 6.0 - add_size,
        header.top + 3.0,
        rect.right - 6.0,
        header.bottom - 3.0,
    );
    let tool_right = add_rect.left - 6.0;
    terminal = terminal.child(
        panel(
            add_rect,
            VisualStyle::filled(theme::c().surface).radius(3.0),
        )
        .key("terminal-new-tab")
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            if commands::new_from_target(&add_state, &add_tabs, active_tab_id, new_shell).is_none()
            {
                return;
            }
            add_focus.focus();
        })
        .child(icon(
            "terminal.new.icon",
            "plus",
            theme::icon_rect(add_rect),
            theme::c().text_soft,
        )),
    );
    let available_width = (tool_right - rect.left).max(0.0);
    let max_instance_width = (available_width * 0.5)
        .clamp(52.0, 220.0)
        .min(available_width);
    let shell_label = ellipsize(
        &selected_shell
            .map(|option| option.name.clone())
            .unwrap_or_else(|| terminal_tabs.short_label(controller)),
        (max_instance_width - 47.0 - TEXT_MARGIN).max(0.0),
        theme::SMALL,
        400,
    );
    // Reserve the leading icon and trailing arrow around the shaped label.
    let instance_width = (25.0 + measure(&shell_label, theme::SMALL, 400) + TEXT_MARGIN + 22.0)
        .ceil()
        .min(max_instance_width);
    let instance_rect = UiRect::new(
        tool_right - instance_width,
        header.top + 3.0,
        tool_right,
        header.bottom - 3.0,
    );

    let tab_strip_left = rect.left;
    let tab_strip = UiRect::new(
        tab_strip_left,
        header.top,
        (instance_rect.left - 8.0).max(tab_strip_left),
        header.bottom,
    );
    let tab_strip_element = render_terminal_tabs(
        tab_strip,
        &tab_items,
        active_tab_id,
        terminal_tabs.clone(),
        state.clone(),
        map.clone(),
        terminal_focus.clone(),
        editor_focus.clone(),
        application.clone(),
    );

    let menu_state = state.clone();
    let menu_focus = terminal_focus.clone();
    let menu_tabs = terminal_tabs.clone();
    terminal = terminal.child(
        panel(
            instance_rect,
            VisualStyle::filled(theme::c().surface).radius(3.0),
        )
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            if !commands::shell_menu(&menu_state, &menu_tabs, active_tab_id) {
                return;
            }
            menu_focus.focus();
        })
        .child(icon(
            "terminal.instance.icon",
            "terminal",
            theme::icon_rect(UiRect::new(
                instance_rect.left + 7.0,
                instance_rect.top,
                instance_rect.left + 19.0,
                instance_rect.bottom,
            )),
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
            theme::icon_rect(UiRect::new(
                instance_rect.right - 17.0,
                instance_rect.top,
                instance_rect.right - 5.0,
                instance_rect.bottom,
            )),
            theme::c().text_dim,
        )),
    );

    for (session, session_rect) in sessions.iter().zip(body_rects.iter().copied()) {
        let active = session.id == active_tab_id;
        let id = UiId::owned(format!("terminal-session-{}", session.id));
        terminal = terminal.child(render_session(
            session_rect,
            state.clone(),
            terminal_tabs.clone(),
            terminal_focus.clone(),
            id,
            session.id,
            session.id,
            terminal_focused && active,
            application.clone(),
            cursor_blink_visible,
        ));
    }

    // Keep the tab backgrounds and underline above the session surfaces.
    terminal = terminal
        .child(crate::ui::components::divider::bottom(header))
        .child(tab_strip_element);

    // Add handles after all session surfaces so they win hit testing.
    for index in 0..sessions.len().saturating_sub(1) {
        let left_id = sessions[index].id;
        let right_id = sessions[index + 1].id;
        let pair_left = body_rects[index].left - if index > 0 { 1.0 } else { 0.0 };
        let pair_right = body_rects[index + 1].right;
        let minimum = crate::ui::pane_sash::PANE_MIN
            .width
            .min(rect.width() / sessions.len() as f32);
        let remaining = (rect.width() - minimum * sessions.len() as f32).max(0.0);
        let down = state.clone();
        let moving = state.clone();
        let up = state.clone();
        let tabs = terminal_tabs.clone();
        terminal = terminal.child(crate::ui::pane_sash::horizontal(
            format!("terminal-split-{left_id}-{right_id}"),
            body_rects[index].right + 0.5,
            (body.top, body.bottom),
            app_state.ui.terminal_split_drag == Some(left_id),
            move || {
                down.ui
                    .update(|app| app.terminal_split_drag = Some(left_id))
            },
            move |x| {
                if moving.get().ui.terminal_split_drag != Some(left_id) || remaining <= 0.0 {
                    return;
                }
                let position = crate::ui::pane_sash::split_position(pair_left, pair_right, x);
                let pair_extra = (pair_right - pair_left - minimum * 2.0).max(0.0);
                if pair_extra > 0.0 {
                    let fraction =
                        ((position - pair_left - minimum) / pair_extra).clamp(0.001, 0.999);
                    commands::resize_pair(&tabs, left_id, right_id, fraction);
                }
            },
            move || up.ui.update(|app| app.terminal_split_drag = None),
        ));
    }

    if shell_menu_open {
        terminal = terminal.child(shell_menu(
            instance_rect,
            rect,
            state.clone(),
            terminal_focus.clone(),
        ));
    }

    if region_view.is_some() {
        return terminal;
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
        crate::ui::components::resize_handle::render(
            "terminal-resize-handle",
            handle,
            UiRect::new(rect.left, rect.top, rect.right, rect.top + 1.0),
            CursorIcon::ResizeVertical,
            resizing,
        )
        .on_pointer_down_with_button(move |_cx, _pointer, button| {
            if button == PointerButton::Left {
                resize_down.run(Command::ResizeStart);
            }
        })
        .on_pointer_drag(move |_cx, pointer| {
            resize_drag.run(Command::Resize(resized_height(
                panel_bottom,
                pointer.point.y,
                max_height,
            )));
        })
        .on_pointer_up(move |_cx, _pointer| {
            resize_up.run(Command::ResizeEnd);
        }),
    );

    terminal
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_session(
    body: UiRect,
    state: Binding,
    terminal_tabs: Sessions,
    terminal_focus: UiFocusHandle,
    terminal_id: UiId,
    controller: SessionId,
    session_id: u64,
    focused: bool,
    application: FrameRequest,
    cursor_blink_visible: bool,
) -> Element {
    let rect = body;
    let snapshot = terminal_tabs.screen(controller);
    let content = UiRect::new(
        body.left + BODY_PAD_X,
        body.top + BODY_PAD_Y,
        body.right - BODY_PAD_X,
        body.bottom - BODY_PAD_Y,
    );
    let cursor_rect = terminal_cursor_rect(content, snapshot.cursor, snapshot.rows, snapshot.cols);
    let mut semantics = Semantics::new(SemanticRole::TextInput)
        .name(format!("{} terminal", snapshot.shell_name))
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
    .semantics(semantics);

    let focus_on_click = terminal_focus.clone();
    let click_tabs = terminal_tabs.clone();
    let click_state = state.clone();
    terminal = terminal.on_click(move || {
        commands::select(&click_state, &click_tabs, session_id);
        focus_on_click.focus();
    });

    let (screen_rows, screen_cols) = (snapshot.rows, snapshot.cols);
    // lgui wheel payloads contain the delta only. Retain the latest pointer
    // cell in the session controller across redraws for application reports.
    let mouse_tabs = terminal_tabs.clone();
    terminal = terminal.on_pointer_move(move |_cx, pointer| {
        let (row, col) = mouse_cell_at(content, pointer.point, screen_rows, screen_cols);
        commands::mouse_position(&mouse_tabs, session_id, row, col);
    });
    let select_input_tabs = terminal_tabs.clone();
    let select_tabs = terminal_tabs.clone();
    let select_focus = terminal_focus.clone();
    let select_state = state.clone();
    let select_application = application.clone();
    terminal = terminal.on_pointer_down_with_button(move |cx, pointer, button| {
        if !content.contains(pointer.point) {
            return;
        }
        if !commands::select(&select_state, &select_tabs, session_id) {
            return;
        }
        select_focus.focus();
        let (row, col) = cell_at(content, pointer.point, screen_rows, screen_cols);
        match button {
            PointerButton::Left => {
                commands::begin_selection(&select_input_tabs, session_id, row, col);
                select_state.ui.update(|app| app.selecting_terminal = true);
            }
            // Right click copies a selection, or pastes when there is none.
            PointerButton::Right => {
                let clipboard = cx.application().clipboard();
                let result = commands::clipboard(
                    &select_input_tabs,
                    session_id,
                    commands::ClipboardShortcut::CopyOrPaste,
                    clipboard.as_ref(),
                );
                commands::report_clipboard_error(&select_state, result);
            }
            _ => return,
        }
        select_application.request_frame();
    });

    let drag_tabs = terminal_tabs.clone();
    let drag_state = state.clone();
    let drag_application = application.clone();
    terminal = terminal.on_pointer_drag(move |cx, pointer| {
        if drag_state.get().ui.selecting_terminal {
            let (row, col) = cell_at(content, pointer.point, screen_rows, screen_cols);
            commands::extend_selection(&drag_tabs, session_id, row, col);
            drag_application.request_frame();
            cx.stop_propagation();
        }
    });
    let select_up_state = state.clone();
    terminal = terminal.on_pointer_up(move |_cx, _pointer| {
        select_up_state
            .ui
            .try_update(|app| std::mem::take(&mut app.selecting_terminal));
    });

    let wheel_tabs = terminal_tabs.clone();
    let wheel_application = application.clone();
    let wheel_state = state.clone();
    terminal = terminal.on_wheel(move |cx, delta| {
        let lines = match delta.unit {
            WheelUnit::Lines => delta.y,
            WheelUnit::Pixels => delta.y / (terminal_line_h() * 3.0),
        };
        if lines.is_finite() && lines != 0.0 {
            commands::wheel(
                &wheel_tabs,
                session_id,
                lines,
                keys::modifiers(wheel_state.get().modifiers),
            );
            wheel_application.request_frame();
            cx.stop_propagation();
        }
    });

    // Default cursors inherit in lgui, so keep the text cursor off the
    // scrollbar's ancestors and apply it only to the screen content.
    terminal = terminal.child(
        render_screen(content, &snapshot, focused, cursor_blink_visible).cursor(CursorIcon::Text),
    );
    let ui = state.get().ui;
    if (snapshot.scrollback_rows > 0 && ui.hovered_terminal == Some(session_id))
        || ui
            .terminal_scrollbar_drag
            .is_some_and(|(id, _)| id == session_id)
    {
        terminal = terminal.child(vertical_scrollbar(
            body,
            &snapshot,
            state,
            terminal_tabs,
            terminal_focus,
            session_id,
            application,
        ));
    }
    terminal
}

fn vertical_scrollbar(
    body: UiRect,
    snapshot: &crate::features::terminal::TerminalSnapshot,
    state: Binding,
    tabs: Sessions,
    focus: UiFocusHandle,
    session: SessionId,
    application: FrameRequest,
) -> Element {
    let track = UiRect::new(
        body.right - scrollbar::SIZE,
        body.top + BODY_PAD_Y,
        body.right,
        body.bottom - BODY_PAD_Y,
    );
    if track.height() <= 0.0 {
        return group(body);
    }
    let history = snapshot.scrollback_rows;
    let rows = usize::from(snapshot.rows);
    let height = (track.height() * rows as f32 / (history + rows).max(1) as f32)
        .max(scrollbar::MIN_THUMB_LENGTH)
        .min(track.height());
    let travel = track.height() - height;
    let top = track.top
        + travel * (1.0 - snapshot.scrollback.min(history) as f32 / history.max(1) as f32);
    // Keep the pressed node alive until release if a program switches screens
    // or clears history during the drag. An empty rect paints no scrollbar.
    let thumb_rect = if history == 0 {
        UiRect::new(track.right, track.top, track.right, track.top)
    } else {
        UiRect::new(
            track.left + scrollbar::INSET,
            top,
            track.right - scrollbar::INSET,
            top + height,
        )
    };
    let down = state.clone();
    let moving = state.clone();
    let up = state;
    let select = tabs.clone();
    scrollbar::thumb(thumb_rect, 0xb8)
        .key(format!("terminal-scrollbar-thumb-{session}"))
        .event_policy(EventPolicy {
            hover: true,
            press: true,
            focus: false,
        })
        .on_pointer_down_with_button(move |cx, pointer, button| {
            if button != PointerButton::Left {
                return;
            }
            commands::select(&down, &select, session);
            focus.focus();
            down.ui.update(|ui| {
                ui.selecting_terminal = false;
                ui.terminal_scrollbar_drag = Some((session, pointer.point.y - top));
            });
            cx.stop_propagation();
        })
        .on_pointer_drag(move |cx, pointer| {
            if let Some((id, offset)) = moving.get().ui.terminal_scrollbar_drag
                && id == session
                && travel > 0.0
            {
                let fraction = ((pointer.point.y - track.top - offset) / travel).clamp(0.0, 1.0);
                tabs.run(Command::Scrollback {
                    target: session,
                    rows: ((1.0 - fraction) * history as f32).round() as usize,
                });
                application.request_frame();
                cx.stop_propagation();
            }
        })
        .on_pointer_up(move |cx, _| {
            up.ui.try_update(|ui| {
                if ui
                    .terminal_scrollbar_drag
                    .is_some_and(|(id, _)| id == session)
                {
                    ui.terminal_scrollbar_drag = None;
                    true
                } else {
                    false
                }
            });
            cx.stop_propagation();
        })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_terminal_tabs(
    rect: UiRect,
    tabs: &[TerminalTab],
    active_id: u64,
    terminal_tabs: Sessions,
    state: Binding,
    map: MapBinding,
    terminal_focus: UiFocusHandle,
    editor_focus: UiFocusHandle,
    application: FrameRequest,
) -> Element {
    let mut strip = panel(rect, VisualStyle::filled(theme::c().sidebar));
    if tabs.is_empty() || rect.width() <= 0.0 {
        return strip;
    }

    let count = tabs.len() as f32;
    let available_tabs_width = (rect.width() - TABS_PAD).max(MIN_TAB_W);
    let labels = tabs.iter().map(TerminalTab::label).collect::<Vec<_>>();
    let close_width = measure("✕", theme::SMALL, 400);
    let fixed_width = TAB_PAD + TAB_CONTENT_GAP + close_width + TAB_PAD;
    let active_width = tabs
        .iter()
        .position(|tab| tab.id == active_id)
        .map(|index| {
            (fixed_width + measure(&labels[index], theme::UI_SIZE, 400) + TEXT_MARGIN)
                .max(MIN_TAB_W)
        })
        .unwrap_or(0.0);
    let inactive_count = count - f32::from(active_width > 0.0);
    let tab_cap = if inactive_count > 0.0 {
        ((available_tabs_width - active_width - TAB_GAP * (count - 1.0)) / inactive_count)
            .clamp(MIN_TAB_W, MAX_TAB_W)
    } else {
        MAX_TAB_W
    };
    let widths = labels
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let natural_width = fixed_width + measure(name, theme::UI_SIZE, 400);
            if tabs[index].id == active_id {
                (natural_width + TEXT_MARGIN).max(MIN_TAB_W)
            } else {
                natural_width.min(tab_cap).max(MIN_TAB_W)
            }
        })
        .collect::<Vec<_>>();
    let mut active_rect = None;
    let mut left = rect.left + TABS_PAD;
    for (index, tab) in tabs.iter().enumerate() {
        let name = &labels[index];
        let active = tab.id == active_id;
        let tab_width = widths[index];
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
        let display_name = if active {
            name.clone()
        } else {
            ellipsize(
                name,
                (name_slot_width - TEXT_MARGIN).max(0.0),
                theme::UI_SIZE,
                400,
            )
        };
        let label_rect = UiRect::new(
            name_left,
            tab_rect.top,
            name_left + name_slot_width,
            tab_rect.bottom,
        );
        if active {
            active_rect = Some(tab_rect);
        }
        let visual = panel(
            tab_rect,
            if active {
                VisualStyle::filled(theme::c().active_tab).alpha(51)
            } else {
                VisualStyle::default()
            },
        )
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
        let press_state = state.clone();
        let select_map = map.clone();
        let menu_state = state.clone();
        let tab_id = tab.id;
        let close_id = UiId::owned(format!("terminal-tab-close-{tab_id}"));
        let close_node_id = close_id.clone();
        let select_node_id = UiId::owned(format!("terminal-tab-select-{tab_id}"));
        let mut tab_element = Element::new(move |cx| {
            let show_close = active
                || cx.context.interaction_flags(&cx.id).hovered
                || cx.context.interaction_flags(&select_node_id).hovered
                || cx.context.interaction_flags(&close_node_id).hovered;
            UiElement::group(cx.id, tab_rect).children(
                cx.children
                    .into_iter()
                    .filter(|child| show_close || child.node().id != close_node_id),
            )
        })
        .key(format!("terminal-tab-{tab_id}"))
        .event_policy(EventPolicy {
            hover: true,
            press: false,
            focus: false,
        })
        .on_event_capture(lgui::core::UiEventKind::PointerDown, move |cx, payload| {
            if let lgui::core::UiEventPayload::PointerDown {
                pointer,
                button: PointerButton::Right,
            } = payload
            {
                menu_state.run(Command::OpenMenu {
                    target: tab_id,
                    position: (pointer.point.x, pointer.point.y),
                    kind: super::state::TerminalTabMenuKind::Actions,
                });
                cx.stop_propagation();
            }
        })
        .child(visual);
        let select_rect = UiRect::new(
            tab_rect.left,
            tab_rect.top,
            (close_left - TAB_CONTENT_GAP).max(tab_rect.left),
            tab_rect.bottom,
        );
        // The pressed handle owns the cursor while dragging. As its own
        // component it follows the map's cursor without re-rendering the strip.
        tab_element = tab_element.child(
            component(select_rect, move |cx, &select_rect| {
                let press_point = cx
                    .state_with(|| Arc::new(std::sync::Mutex::new(None::<Point>)))
                    .get();
                let click_point = press_point.clone();
                let click_state = press_state.clone();
                let click_map = select_map.clone();
                let cursor = cx.use_observable(
                    select_map.observable(),
                    |s: &super::layout_binding::MapSignal| s.cursor,
                );
                let capture_map = select_map.clone();
                Element::new(move |cx| {
                    let id = UiId::owned(format!("terminal-tab-select-{tab_id}"));
                    if capture_map
                        .with(|d| d.is_some_and(|d| d.session == tab_id && d.release.is_none()))
                        && !cx.context.interaction_flags(&id).pressed
                    {
                        capture_map.cancel();
                    }
                    UiElement::panel(id, select_rect, VisualStyle::default()).children(cx.children)
                })
                .key(format!("terminal-tab-select-{tab_id}"))
                .event_policy(EventPolicy {
                    hover: true,
                    press: true,
                    focus: false,
                })
                .cursor(cursor)
                .on_pointer_down_with_button(move |cx, pointer, button| {
                    if button == PointerButton::Left {
                        *press_point.lock().unwrap() = Some(pointer.point);
                        if !commands::select(&press_state, &select_tabs, tab_id) {
                            return;
                        }
                        select_focus.focus();
                        select_map.run(MapCommand::Begin {
                            session: tab_id,
                            point: pointer.point,
                        });
                        cx.stop_propagation();
                    }
                })
                .on_click(move || {
                    let Some(point) = click_point.lock().unwrap().take() else {
                        return;
                    };
                    if click_state
                        .run(Command::TabClick {
                            target: tab_id,
                            point: (point.x, point.y),
                            menu_position: (tab_rect.left, tab_rect.bottom),
                        })
                        .applied
                    {
                        click_map.cancel();
                    }
                })
            })
            .key(format!("terminal-tab-select-component-{tab_id}")),
        );

        let close_tabs = terminal_tabs.clone();
        let close_state = state.clone();
        let close_terminal_focus = terminal_focus.clone();
        let close_editor_focus = editor_focus.clone();
        let close_application = application.clone();
        tab_element = tab_element.child(
            Element::new(move |cx| {
                UiElement::panel(close_id, close_rect, VisualStyle::default()).children(cx.children)
            })
            .key(format!("terminal-tab-close-{tab_id}"))
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(CursorIcon::Pointer)
            .on_click(move || {
                if !commands::close(
                    &close_state,
                    &close_tabs,
                    tab_id,
                    crate::features::terminal::actions::MenuAction::Close,
                    &close_application,
                ) {
                    return;
                }
                let closing_last_tab = close_tabs.get().is_empty();
                if closing_last_tab {
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
        left += tab_width + TAB_GAP;
    }

    strip = strip.child(crate::ui::components::divider::bottom(rect));
    if let Some(active) = active_rect {
        strip = strip.child(panel(
            UiRect::new(
                active.left,
                (active.bottom - 2.0).max(active.top),
                active.right,
                active.bottom,
            ),
            VisualStyle::filled(theme::c().accent),
        ));
    }
    clip(rect, 0.0, 0.0).child(strip)
}

pub(crate) fn render_screen(
    content: UiRect,
    snapshot: &crate::features::terminal::TerminalSnapshot,
    focused: bool,
    cursor_blink_visible: bool,
) -> Element {
    let mut screen = group(content);
    for (row, runs) in snapshot.lines.iter().enumerate() {
        let top = content.top + row as f32 * terminal_line_h();
        if top >= content.bottom {
            break;
        }
        for run in runs {
            if let Some(background) = run.style.background {
                let (left, right) = run_span(content, run);
                screen = screen.child(panel(
                    UiRect::new(left, top, right, top + terminal_line_h()),
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
                UiRect::new(left, top, right, top + terminal_line_h()),
                VisualStyle::filled(theme::c().selection),
            ));
        }
        for run in runs {
            let (left, right) = run_span(content, run);
            let run_rect = UiRect::new(left, top, right, top + terminal_line_h());
            if !run.text.is_empty() {
                let foreground = run.style.foreground.map(Color).unwrap_or(theme::c().text);
                let mut style = if run.style.bold {
                    fonts::terminal().bold(foreground)
                } else {
                    fonts::terminal().style(foreground)
                };
                if run.style.dim {
                    style = style.alpha(0x90);
                }
                // Keep a terminal run on one row even when shaping/rounding
                // makes its natural width slightly exceed the allocated cells.
                let measured = crate::ui::components::text::SingleLineText::new(
                    run.text.clone(),
                    UiRect::new(left, top, left + 100_000.0, top + terminal_line_h()),
                    style,
                );
                let text_rect = UiRect::new(
                    left,
                    top,
                    left + measured.width().max(right - left) + 2.0,
                    top + terminal_line_h(),
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
                            top + terminal_line_h() - 3.0,
                            right,
                            top + terminal_line_h() - 2.0,
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

    if let Some(message) = snapshot
        .status
        .message()
        .or_else(|| snapshot.notice.clone())
    {
        let top = (content.bottom - terminal_line_h()).max(content.top);
        screen = screen.child(text(
            UiRect::new(content.left, top, content.right, top + terminal_line_h()),
            message,
            theme::mono(theme::c().text_dim, theme::SMALL),
        ));
    }

    clip(content, 0.0, 0.0).child(screen)
}

pub(crate) fn run_span(content: UiRect, run: &TerminalRun) -> (f32, f32) {
    let cell_width = terminal_cell_width();
    let left = content.left + f32::from(run.start_col) * cell_width;
    let right = (left + f32::from(run.columns) * cell_width + 2.0).min(content.right + 2.0);
    (left, right)
}

/// Maps a pointer to the nearest cell boundary, clamped to the screen.
pub(crate) fn cell_at(content: UiRect, point: Point, rows: u16, cols: u16) -> (u16, u16) {
    let row = ((point.y - content.top) / terminal_line_h())
        .floor()
        .clamp(0.0, f32::from(rows.saturating_sub(1)));
    let col = ((point.x - content.left) / terminal_cell_width())
        .round()
        .clamp(0.0, f32::from(cols));
    (row as u16, col as u16)
}

pub(crate) fn mouse_cell_at(content: UiRect, point: Point, rows: u16, cols: u16) -> (u16, u16) {
    let row = ((point.y - content.top) / terminal_line_h())
        .floor()
        .clamp(0.0, f32::from(rows.saturating_sub(1)));
    let col = ((point.x - content.left) / terminal_cell_width())
        .floor()
        .clamp(0.0, f32::from(cols.saturating_sub(1)));
    (row as u16, col as u16)
}

pub(crate) fn cursor_is_drawn(pty_visible: bool, focused: bool, blink_visible: bool) -> bool {
    pty_visible && (!focused || blink_visible)
}

pub(crate) fn shell_menu(
    instance_rect: UiRect,
    bounds: UiRect,
    state: Binding,
    focus: UiFocusHandle,
) -> Element {
    let frame = state.get();
    let options = frame.shell_options;
    let space = (bounds.bottom - instance_rect.bottom).max(instance_rect.top - bounds.top);
    let visible = (((space - 12.0) / SHELL_MENU_ROW_H).floor() as usize)
        .clamp(1, 7)
        .min(options.len().max(1));
    let offset = state
        .get()
        .ui
        .terminal_shell_scroll
        .min(options.len().saturating_sub(visible));
    let height = 8.0 + SHELL_MENU_ROW_H * options.len().min(visible).max(1) as f32;
    let top = if instance_rect.bottom + 4.0 + height <= bounds.bottom {
        instance_rect.bottom + 4.0
    } else {
        (instance_rect.top - 4.0 - height).max(bounds.top)
    };
    let width = SHELL_MENU_W.min(bounds.width().max(0.0));
    let left =
        (instance_rect.right - width).clamp(bounds.left, (bounds.right - width).max(bounds.left));
    let menu_rect = UiRect::new(left, top, left + width, top + height);
    let mut menu = theme::bordered(menu_rect, theme::c().surface, theme::c().border, 4.0, 1.0);
    let scroll_state = state.clone();
    let max_offset = options.len().saturating_sub(visible);
    menu = menu
        .event_policy(EventPolicy::INTERACTIVE)
        .on_wheel(move |cx, delta| {
            let step = delta.y.abs().ceil().max(1.0) as usize;
            scroll_state.ui.update(|app| {
                app.terminal_shell_scroll = if delta.y > 0.0 {
                    offset.saturating_sub(step)
                } else {
                    offset.saturating_add(step).min(max_offset)
                }
            });
            cx.stop_propagation();
        });
    for (index, option) in options.iter().skip(offset).take(visible).enumerate() {
        let shell = option.kind;
        let top = menu_rect.top + 4.0 + index as f32 * SHELL_MENU_ROW_H;
        let row = UiRect::new(
            menu_rect.left + 4.0,
            top,
            menu_rect.right - 4.0,
            top + SHELL_MENU_ROW_H,
        );
        let row_state = state.clone();
        let row_focus = focus.clone();
        let style = if frame.ui.terminal_shell_selection == Some(shell) {
            VisualStyle::filled(theme::c().active_line)
        } else {
            VisualStyle::default()
        };
        let mut item = panel(row, style.radius(3.0)).key(format!("terminal-shell-{shell:?}"));
        if option.available() {
            item = item
                .event_policy(EventPolicy::INTERACTIVE)
                .cursor(CursorIcon::Pointer)
                .on_click(move || {
                    row_state.ui.update(|app| {
                        app.terminal_shell_selection = Some(shell);
                        app.terminal_shell_menu = false;
                    });
                    row_focus.focus();
                });
        }
        let item = item.child(text(
            UiRect::new(row.left + 8.0, row.top, row.right - 8.0, row.bottom),
            ellipsize(
                &format!(
                    "{}{}",
                    option.name,
                    if option.available() {
                        ""
                    } else {
                        " (unavailable)"
                    }
                ),
                (row.width() - 16.0).max(0.0),
                theme::UI_SIZE,
                400,
            ),
            theme::mono(
                if option.available() {
                    theme::c().text
                } else {
                    theme::c().text_dim
                },
                theme::UI_SIZE,
            ),
        ));
        menu = menu.child(item);
    }
    menu
}

pub(crate) fn terminal_cursor_rect(
    content: UiRect,
    cursor: (u16, u16),
    rows: u16,
    cols: u16,
) -> UiRect {
    let cell_width = terminal_cell_width();
    // vt100 keeps col == cols while a full line waits for the next character
    // to trigger wrapping. The visible cursor stays on the last cell meanwhile.
    let left = content.left + f32::from(cursor.1.min(cols.saturating_sub(1))) * cell_width;
    let top = content.top + f32::from(cursor.0.min(rows.saturating_sub(1))) * terminal_line_h();
    UiRect::new(
        left,
        top + 2.0,
        left + cell_width,
        top + terminal_line_h() - 2.0,
    )
}

pub(crate) fn resized_height(panel_bottom: f32, pointer_y: f32, max_height: f32) -> f32 {
    (panel_bottom - pointer_y).clamp(theme::TERMINAL_MIN_H, max_height)
}
