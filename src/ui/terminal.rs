//! Bottom terminal panel backed by a real platform PTY.

use std::path::PathBuf;
use std::sync::Arc;

use lgui::ApplicationHandle;
use lgui::core::{
    CursorIcon, EventPolicy, IconStyle, KeyState, KeyboardEvent, LogicalKey, NamedKey,
    PointerButton, SemanticRole, Semantics, UiElement, UiFocusHandle, UiId, WheelUnit, clip,
    precompiled,
};
use lgui::prelude::{Color, Element, State, UiRect, VisualStyle, group, panel, text};

use crate::state::AppState;
use crate::terminal_session::{ShellKind, TerminalController, TerminalSize};
use crate::theme;

const HEADER_H: f32 = 34.0;
const HEADER_PAD: f32 = 12.0;
const TOOL_BUTTON_W: f32 = 28.0;
const TOOL_ICON: f32 = 14.0;
const INSTANCE_W: f32 = 132.0;
const BODY_PAD_X: f32 = 12.0;
const BODY_PAD_Y: f32 = 6.0;
const TERMINAL_FONT_SIZE: f32 = 12.0;
const TERMINAL_CHAR_W: f32 = 7.2;
const TERMINAL_LINE_H: f32 = 19.0;
const RESIZE_HIT_H: f32 = 8.0;
const SHELL_MENU_W: f32 = 172.0;
const SHELL_MENU_ROW_H: f32 = 28.0;

fn icon(id: &'static str, key: &'static str, rect: UiRect, color: Color) -> Element {
    precompiled(UiElement::icon(UiId::new(id), rect, key).icon_style(IconStyle::new(color)))
}

fn icon_button(id: &'static str, key: &'static str, rect: UiRect, color: Color) -> Element {
    let icon_left = rect.left + (rect.width() - TOOL_ICON) / 2.0;
    let icon_top = rect.top + (rect.height() - TOOL_ICON) / 2.0;
    panel(rect, VisualStyle::default()).child(icon(
        id,
        key,
        UiRect::new(
            icon_left,
            icon_top,
            icon_left + TOOL_ICON,
            icon_top + TOOL_ICON,
        ),
        color,
    ))
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
    application: Arc<ApplicationHandle>,
    cwd: Option<PathBuf>,
    size: TerminalSize,
    max_height: f32,
) -> Element {
    let app_state = state.get();
    let resizing = app_state.resizing_terminal;
    let terminal_focused = app_state.terminal_focused;
    let shell_menu_open = app_state.terminal_shell_menu;
    let snapshot = controller.snapshot();
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
        UiElement::panel(terminal_id, rect, VisualStyle::filled(theme::BG))
            .ime_cursor_rect(cursor_rect)
            .children(cx.children)
    })
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(semantics)
    .cursor(CursorIcon::Text);

    let focus_on_click = terminal_focus.clone();
    terminal = terminal.on_click(move || focus_on_click.focus());

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
    terminal = terminal.on_key_down(move |cx, event| {
        if let Some(bytes) = key_bytes(event) {
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
            theme::ACCENT
        } else {
            theme::BORDER
        }),
    ));
    terminal = terminal.child(panel(
        UiRect::new(rect.left, header.bottom - 1.0, rect.right, header.bottom),
        VisualStyle::filled(theme::BORDER),
    ));

    let title_left = rect.left + HEADER_PAD;
    terminal = terminal.child(text(
        UiRect::new(title_left, header.top, title_left + 70.0, header.bottom),
        "TERMINAL",
        theme::sans_semibold(theme::ZINC_200, theme::SMALL),
    ));
    terminal = terminal.child(panel(
        UiRect::new(
            title_left,
            header.bottom - 2.0,
            title_left + 52.0,
            header.bottom,
        ),
        VisualStyle::filled(theme::ACCENT),
    ));

    let mut tool_right = rect.right - 6.0;
    let close_rect = UiRect::new(
        tool_right - TOOL_BUTTON_W,
        header.top,
        tool_right,
        header.bottom,
    );
    tool_right = close_rect.left;
    let maximize_rect = UiRect::new(
        tool_right - TOOL_BUTTON_W,
        header.top,
        tool_right,
        header.bottom,
    );
    tool_right = maximize_rect.left;
    let trash_rect = UiRect::new(
        tool_right - TOOL_BUTTON_W,
        header.top,
        tool_right,
        header.bottom,
    );
    tool_right = trash_rect.left;
    let split_rect = UiRect::new(
        tool_right - TOOL_BUTTON_W,
        header.top,
        tool_right,
        header.bottom,
    );
    tool_right = split_rect.left;
    let plus_rect = UiRect::new(
        tool_right - TOOL_BUTTON_W,
        header.top,
        tool_right,
        header.bottom,
    );
    tool_right = plus_rect.left - 4.0;
    let instance_rect = UiRect::new(
        tool_right - INSTANCE_W,
        header.top + 5.0,
        tool_right,
        header.bottom - 5.0,
    );

    let menu_state = state.clone();
    let menu_focus = terminal_focus.clone();
    terminal = terminal.child(
        panel(
            instance_rect,
            VisualStyle::filled(theme::SURFACE).radius(3.0),
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
            theme::ZINC_400,
        ))
        .child(text(
            UiRect::new(
                instance_rect.left + 25.0,
                instance_rect.top,
                instance_rect.right - 22.0,
                instance_rect.bottom,
            ),
            snapshot.shell.short_label(),
            theme::mono(theme::ZINC_300, theme::SMALL),
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
            theme::ZINC_500,
        )),
    );

    let restart_controller = controller.clone();
    let restart_application = application.clone();
    let restart_cwd = cwd.clone();
    let restart_focus = terminal_focus.clone();
    terminal = terminal.child(
        icon_button("terminal.new", "plus", plus_rect, theme::ZINC_400)
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(CursorIcon::Pointer)
            .on_click(move || {
                restart_controller.restart(
                    restart_controller.shell(),
                    restart_cwd.as_deref(),
                    size,
                    restart_application.clone(),
                );
                restart_focus.focus();
            }),
    );
    terminal = terminal.child(icon_button(
        "terminal.split",
        "split-terminal",
        split_rect,
        theme::ZINC_600,
    ));

    let stop_controller = controller.clone();
    let stop_application = application.clone();
    let stop_focus = terminal_focus.clone();
    terminal = terminal.child(
        icon_button("terminal.trash", "trash", trash_rect, theme::ZINC_400)
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(CursorIcon::Pointer)
            .on_click(move || {
                stop_controller.terminate(&stop_application);
                stop_focus.focus();
            }),
    );

    let maximize_state = state.clone();
    terminal = terminal.child(
        icon_button(
            "terminal.maximize",
            "maximize",
            maximize_rect,
            theme::ZINC_400,
        )
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            maximize_state.update(move |app| {
                app.terminal_h = if (app.terminal_h - max_height).abs() < 1.0 {
                    theme::TERMINAL_H.min(max_height)
                } else {
                    max_height
                };
            });
        }),
    );

    let close_state = state.clone();
    let close_focus = editor_focus;
    terminal = terminal.child(
        icon_button("terminal.close", "close", close_rect, theme::ZINC_500)
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(CursorIcon::Pointer)
            .on_click(move || {
                close_state.update(|app| {
                    app.show_terminal = false;
                    app.terminal_shell_menu = false;
                    app.terminal_focused = false;
                });
                close_focus.focus();
            }),
    );

    terminal = terminal.child(render_screen(content, &snapshot, terminal_focused));

    if shell_menu_open {
        terminal = terminal.child(shell_menu(
            instance_rect,
            snapshot.shell,
            state.clone(),
            terminal_focus.clone(),
            controller.clone(),
            application.clone(),
            cwd,
            size,
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

fn render_screen(
    content: UiRect,
    snapshot: &crate::terminal_session::TerminalSnapshot,
    focused: bool,
) -> Element {
    let mut screen = group(content);
    for (row, runs) in snapshot.lines.iter().enumerate() {
        let top = content.top + row as f32 * TERMINAL_LINE_H;
        if top >= content.bottom {
            break;
        }
        for run in runs {
            let left = content.left + f32::from(run.start_col) * TERMINAL_CHAR_W;
            let right =
                (left + f32::from(run.columns) * TERMINAL_CHAR_W + 2.0).min(content.right + 2.0);
            let run_rect = UiRect::new(left, top, right, top + TERMINAL_LINE_H);
            if let Some(background) = run.style.background {
                screen = screen.child(panel(
                    UiRect::new(left, top, right, top + TERMINAL_LINE_H),
                    VisualStyle::filled(Color(background)),
                ));
            }
            if !run.text.is_empty() {
                let foreground = run.style.foreground.map(Color).unwrap_or(theme::ZINC_200);
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

    if snapshot.cursor_visible {
        let cursor = terminal_cursor_rect(content, snapshot.cursor);
        let cursor_style = if focused {
            VisualStyle::filled(theme::ZINC_300).alpha(0xc8)
        } else {
            VisualStyle::default().stroked(theme::hairline(theme::ZINC_500))
        };
        screen = screen.child(panel(cursor, cursor_style));
    }

    if let Some(message) = snapshot.status.message() {
        let top = (content.bottom - TERMINAL_LINE_H).max(content.top);
        screen = screen.child(text(
            UiRect::new(content.left, top, content.right, top + TERMINAL_LINE_H),
            message,
            theme::mono(theme::ZINC_500, theme::SMALL),
        ));
    }

    clip(content, 0.0, 0.0).child(screen)
}

#[allow(clippy::too_many_arguments)]
fn shell_menu(
    instance_rect: UiRect,
    active: ShellKind,
    state: State<AppState>,
    focus: UiFocusHandle,
    controller: TerminalController,
    application: Arc<ApplicationHandle>,
    cwd: Option<PathBuf>,
    size: TerminalSize,
) -> Element {
    let menu_rect = UiRect::new(
        instance_rect.right - SHELL_MENU_W,
        instance_rect.bottom + 4.0,
        instance_rect.right,
        instance_rect.bottom + 4.0 + SHELL_MENU_ROW_H * ShellKind::ALL.len() as f32 + 8.0,
    );
    let mut menu = theme::bordered(menu_rect, theme::SURFACE, theme::BORDER, 4.0, 1.0);
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
        let row_controller = controller.clone();
        let row_application = application.clone();
        let row_cwd = cwd.clone();
        let mut item = panel(
            row,
            if shell == active {
                VisualStyle::filled(theme::ACTIVE_LINE).radius(3.0)
            } else {
                VisualStyle::default().radius(3.0)
            },
        )
        .key(format!("terminal-shell-{}", shell.short_label()))
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            row_controller.restart(shell, row_cwd.as_deref(), size, row_application.clone());
            row_state.update(|app| app.terminal_shell_menu = false);
            row_focus.focus();
        });
        if shell == active {
            item = item.child(text(
                UiRect::new(row.left + 8.0, row.top, row.left + 22.0, row.bottom),
                "•",
                theme::mono_bold(theme::ACCENT, theme::UI_SIZE),
            ));
        }
        item = item.child(text(
            UiRect::new(row.left + 24.0, row.top, row.right - 8.0, row.bottom),
            shell.label(),
            theme::mono(theme::ZINC_200, theme::UI_SIZE),
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

fn key_bytes(event: &KeyboardEvent) -> Option<Vec<u8>> {
    if event.state != KeyState::Down {
        return None;
    }

    if event.modifiers.ctrl()
        && let LogicalKey::Character(value) = &event.key
        && let Some(character) = value.chars().next()
        && value.chars().count() == 1
    {
        let lower = character.to_ascii_lowercase();
        // These remain editor-wide shortcuts while the terminal is focused.
        if matches!(lower, 'p' | 'b' | '`') {
            return None;
        }
        if lower.is_ascii_lowercase() {
            return Some(vec![(lower as u8) & 0x1f]);
        }
    }

    let bytes: &[u8] = match &event.key {
        LogicalKey::Named(NamedKey::Enter) => b"\r",
        LogicalKey::Named(NamedKey::Backspace) => b"\x7f",
        LogicalKey::Named(NamedKey::Delete) => b"\x1b[3~",
        LogicalKey::Named(NamedKey::Tab) if event.modifiers.shift() => b"\x1b[Z",
        LogicalKey::Named(NamedKey::Tab) => b"\t",
        LogicalKey::Named(NamedKey::Escape) => b"\x1b",
        LogicalKey::Named(NamedKey::ArrowUp) => b"\x1b[A",
        LogicalKey::Named(NamedKey::ArrowDown) => b"\x1b[B",
        LogicalKey::Named(NamedKey::ArrowRight) => b"\x1b[C",
        LogicalKey::Named(NamedKey::ArrowLeft) => b"\x1b[D",
        LogicalKey::Named(NamedKey::Home) => b"\x1b[H",
        LogicalKey::Named(NamedKey::End) => b"\x1b[F",
        LogicalKey::Named(NamedKey::PageUp) => b"\x1b[5~",
        LogicalKey::Named(NamedKey::PageDown) => b"\x1b[6~",
        LogicalKey::Named(NamedKey::Insert) => b"\x1b[2~",
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
