//! Bottom terminal panel chrome.
//!
//! The terminal process is intentionally still a later concern. This module
//! establishes the panel hierarchy and visual language used by desktop code
//! editors without pretending that a separate input box is a shell prompt.

use lgui::core::{
    CursorIcon, EventPolicy, IconStyle, PointerButton, UiElement, UiFocusHandle, UiId, precompiled,
};
use lgui::prelude::{Color, Element, State, UiRect, VisualStyle, panel, text};

use crate::state::AppState;
use crate::theme;

const HEADER_H: f32 = 34.0;
const HEADER_PAD: f32 = 12.0;
const TOOL_BUTTON_W: f32 = 28.0;
const TOOL_ICON: f32 = 14.0;
const INSTANCE_W: f32 = 112.0;
const BODY_PAD_X: f32 = 14.0;
const BODY_PAD_TOP: f32 = 10.0;
const TERMINAL_LINE_H: f32 = 20.0;
const RESIZE_HIT_H: f32 = 8.0;

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

pub fn render(
    rect: UiRect,
    state: State<AppState>,
    editor_focus: UiFocusHandle,
    max_height: f32,
) -> Element {
    let resizing = state.get().resizing_terminal;
    let mut terminal = panel(rect, VisualStyle::filled(theme::BG));
    let header = UiRect::new(rect.left, rect.top + 1.0, rect.right, rect.top + HEADER_H);

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

    // Active panel tab. The small accent underline makes the selected surface
    // legible without the heavy boxed header used by the previous mockup.
    let title_left = rect.left + HEADER_PAD;
    terminal = terminal.child(text(
        UiRect::new(title_left, header.top, title_left + 70.0, header.bottom),
        "TERMINAL",
        theme::sans_semibold(theme::ZINC_200, theme::SMALL),
    ));
    terminal = terminal.child(panel(
        UiRect::new(title_left, header.bottom - 2.0, title_left + 52.0, header.bottom),
        VisualStyle::filled(theme::ACCENT),
    ));

    // Right-aligned terminal instance selector and editor-style icon actions.
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

    terminal = terminal.child(
        panel(instance_rect, VisualStyle::filled(theme::SURFACE).radius(3.0))
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
                "pwsh",
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
    terminal = terminal.child(icon_button(
        "terminal.new",
        "plus",
        plus_rect,
        theme::ZINC_400,
    ));
    terminal = terminal.child(icon_button(
        "terminal.split",
        "split-terminal",
        split_rect,
        theme::ZINC_400,
    ));
    terminal = terminal.child(icon_button(
        "terminal.trash",
        "trash",
        trash_rect,
        theme::ZINC_400,
    ));
    terminal = terminal.child(icon_button(
        "terminal.maximize",
        "maximize",
        maximize_rect,
        theme::ZINC_400,
    ));

    let st_close = state.clone();
    let close_focus = editor_focus;
    terminal = terminal.child(
        icon_button(
            "terminal.close",
            "close",
            close_rect,
            theme::ZINC_500,
        )
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move || {
            st_close.update(|app| app.show_terminal = false);
            close_focus.focus();
        }),
    );

    // Terminal output uses one continuous shell flow. There is no artificial
    // bottom input field; the final row is simply the next shell prompt.
    let body_top = header.bottom + BODY_PAD_TOP;
    let content_right = rect.right - BODY_PAD_X;
    let prompt_w = 14.0;
    terminal = terminal.child(text(
        UiRect::new(
            rect.left + BODY_PAD_X,
            body_top,
            rect.left + BODY_PAD_X + prompt_w,
            body_top + TERMINAL_LINE_H,
        ),
        "❯",
        theme::mono_bold(theme::ACCENT, theme::SMALL),
    ));
    terminal = terminal.child(text(
        UiRect::new(
            rect.left + BODY_PAD_X + prompt_w + 4.0,
            body_top,
            content_right,
            body_top + TERMINAL_LINE_H,
        ),
        "cargo run -p leditor",
        theme::mono(theme::ZINC_200, theme::SMALL),
    ));

    let output = [
        ("   Compiling leditor v0.1.0", theme::ZINC_500),
        ("    Finished `dev` profile", theme::EMERALD_500),
        ("     Running `target\\debug\\leditor.exe`", theme::ZINC_400),
    ];
    for (index, (line, color)) in output.into_iter().enumerate() {
        let top = body_top + (index as f32 + 1.0) * TERMINAL_LINE_H;
        terminal = terminal.child(text(
            UiRect::new(
                rect.left + BODY_PAD_X,
                top,
                content_right,
                top + TERMINAL_LINE_H,
            ),
            line,
            theme::mono(color, theme::SMALL),
        ));
    }

    let prompt_top = body_top + 4.5 * TERMINAL_LINE_H;
    terminal = terminal.child(text(
        UiRect::new(
            rect.left + BODY_PAD_X,
            prompt_top,
            rect.left + BODY_PAD_X + prompt_w,
            prompt_top + TERMINAL_LINE_H,
        ),
        "❯",
        theme::mono_bold(theme::ACCENT, theme::SMALL),
    ));
    terminal = terminal.child(panel(
        UiRect::new(
            rect.left + BODY_PAD_X + prompt_w + 4.0,
            prompt_top + 5.0,
            rect.left + BODY_PAD_X + prompt_w + 11.0,
            prompt_top + 16.0,
        ),
        VisualStyle::filled(theme::ZINC_300).alpha(0xc0),
    ));

    // The invisible resize target straddles the panel boundary so the divider
    // is easy to acquire from either the editor or terminal side. It is added
    // last to stay above terminal header controls during a drag.
    let handle = UiRect::new(
        rect.left,
        rect.top - RESIZE_HIT_H / 2.0,
        rect.right,
        rect.top + RESIZE_HIT_H / 2.0,
    );
    let panel_bottom = rect.bottom;
    let st_down = state.clone();
    let st_drag = state.clone();
    let st_up = state;
    terminal = terminal.child(
        panel(handle, VisualStyle::default())
            .key("terminal-resize-handle")
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(CursorIcon::ResizeVertical)
            .on_pointer_down_with_button(move |_cx, _pointer, button| {
                if button == PointerButton::Left {
                    st_down.update(|app| app.resizing_terminal = true);
                }
            })
            .on_pointer_drag(move |_cx, pointer| {
                st_drag.update(move |app| {
                    if app.resizing_terminal {
                        app.terminal_h = resized_height(panel_bottom, pointer.point.y, max_height);
                    }
                });
            })
            .on_pointer_up(move |_cx, _pointer| {
                st_up.update(|app| app.resizing_terminal = false);
            }),
    );

    terminal
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
}
