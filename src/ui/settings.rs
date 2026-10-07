//! Settings page shown in its own editor tab.
//!
//! Every control writes straight to `AppState`; app-level effects persist the
//! values, so there is no separate apply/save step.

use lgui::core::{CursorIcon, EventPolicy, WheelUnit, clip};
use lgui::prelude::{Element, State, TextAlign, UiRect, VisualStyle, group, panel, text};

use crate::state::AppState;
use crate::terminal_session::ShellKind;
use crate::theme;
use crate::ui::tabs::{TEXT_MARGIN, measure};

const MAX_PAGE_W: f32 = 720.0;
const PAGE_PAD_X: f32 = 32.0;
const PAGE_PAD_TOP: f32 = 36.0;
const HEADER_H: f32 = 64.0;
const SECTION_TITLE_H: f32 = 28.0;
const SECTION_GAP: f32 = 24.0;
const ROW_H: f32 = 58.0;
const CONTROL_GAP: f32 = 24.0;
const SWITCH_W: f32 = 30.0;
const SWITCH_H: f32 = 16.0;
const KNOB: f32 = 12.0;
const SEGMENT_W: f32 = 64.0;
const SEGMENT_H: f32 = 24.0;
const SCROLL_STEP: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Setting {
    DefaultShell,
    TerminalCursorBlink,
    InlineBlame,
    SplitDiff,
    TreeView,
}

const SECTIONS: [(&str, &[Setting]); 2] = [
    (
        "TERMINAL",
        &[Setting::DefaultShell, Setting::TerminalCursorBlink],
    ),
    (
        "SOURCE CONTROL",
        &[Setting::InlineBlame, Setting::SplitDiff, Setting::TreeView],
    ),
];

impl Setting {
    fn label(self) -> &'static str {
        match self {
            Self::DefaultShell => "Default shell",
            Self::TerminalCursorBlink => "Cursor blinking",
            Self::InlineBlame => "Inline blame",
            Self::SplitDiff => "Side-by-side diff",
            Self::TreeView => "Tree view",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::DefaultShell => "Used when a new terminal opens without picking a shell.",
            Self::TerminalCursorBlink => "Blink the terminal cursor while it has focus.",
            Self::InlineBlame => "Show the last commit for the current line in the editor.",
            Self::SplitDiff => "Open diffs with the old and new versions in two columns.",
            Self::TreeView => "Group changed files by folder in Source Control.",
        }
    }

    /// Current value of an on/off setting; `None` for choice settings.
    fn enabled(self, app: &AppState) -> Option<bool> {
        match self {
            Self::DefaultShell => None,
            Self::TerminalCursorBlink => Some(app.terminal_cursor_blink),
            Self::InlineBlame => Some(app.git_inline_blame),
            Self::SplitDiff => Some(app.git_split_diff),
            Self::TreeView => Some(app.git_tree_view),
        }
    }

    fn toggle(self, app: &mut AppState) {
        match self {
            Self::DefaultShell => {}
            Self::TerminalCursorBlink => app.terminal_cursor_blink ^= true,
            Self::InlineBlame => app.git_inline_blame ^= true,
            Self::SplitDiff => app.git_split_diff ^= true,
            Self::TreeView => app.git_tree_view ^= true,
        }
    }
}

fn content_height() -> f32 {
    let rows: usize = SECTIONS.iter().map(|(_, settings)| settings.len()).sum();
    PAGE_PAD_TOP
        + HEADER_H
        + SECTIONS.len() as f32 * (SECTION_TITLE_H + SECTION_GAP)
        + rows as f32 * ROW_H
}

pub fn render(rect: UiRect, state: State<AppState>) -> Element {
    let app = state.get();
    let max_scroll = (content_height() - rect.height()).max(0.0);
    let scroll_y = app.workspace.active_scroll().1.clamp(0.0, max_scroll);

    let page_w = (rect.width() - PAGE_PAD_X * 2.0).clamp(0.0, MAX_PAGE_W);
    let page_left = rect.left + (rect.width() - page_w) / 2.0;
    let page_right = page_left + page_w;
    let mut top = rect.top + PAGE_PAD_TOP - scroll_y;

    let wheel_state = state.clone();
    let mut page = panel(rect, VisualStyle::filled(theme::c().bg))
        .event_policy(EventPolicy::INTERACTIVE)
        .on_wheel(move |cx, delta| {
            let step = match delta.unit {
                WheelUnit::Lines => delta.y * SCROLL_STEP * 3.0,
                WheelUnit::Pixels => delta.y,
            };
            wheel_state.update(move |app| {
                let (x, y) = app.workspace.active_scroll();
                app.workspace
                    .set_active_scroll(x, (y - step).clamp(0.0, max_scroll));
            });
            cx.stop_propagation();
        });

    let mut content = group(rect);
    let title_w = measure("Settings", 18.0, 700);
    content = content.child(text(
        UiRect::new(
            page_left,
            top,
            page_left + title_w + TEXT_MARGIN,
            top + 26.0,
        ),
        "Settings",
        theme::mono_bold(theme::c().text_bright, 18.0),
    ));
    content = content.child(text(
        UiRect::new(page_left, top + 28.0, page_right, top + 48.0),
        "Changes apply immediately and are saved automatically.",
        theme::sans(theme::c().text_dim, theme::UI_SIZE),
    ));
    top += HEADER_H;

    for (title, settings) in SECTIONS {
        content = content.child(text(
            UiRect::new(page_left, top, page_right, top + SECTION_TITLE_H),
            title,
            theme::mono(theme::c().text_dim, theme::SMALL).tracking(0.8),
        ));
        top += SECTION_TITLE_H;
        content = content.child(panel(
            UiRect::new(page_left, top - 1.0, page_right, top),
            VisualStyle::filled(theme::c().border),
        ));
        for &setting in settings {
            let row = UiRect::new(page_left, top, page_right, top + ROW_H);
            content = content.child(setting_row(row, setting, &app, state.clone()));
            top += ROW_H;
        }
        top += SECTION_GAP;
    }

    page = page.child(content);
    clip(rect, 0.0, 0.0).child(page)
}

fn setting_row(row: UiRect, setting: Setting, app: &AppState, state: State<AppState>) -> Element {
    let control_w = match setting.enabled(app) {
        Some(_) => SWITCH_W,
        None => SEGMENT_W * ShellKind::ALL.len() as f32,
    };
    let control_left = row.right - control_w;
    let text_right = (control_left - CONTROL_GAP).max(row.left);

    let mut element = panel(row, VisualStyle::default());
    element = element.child(text(
        UiRect::new(row.left, row.top + 11.0, text_right, row.top + 29.0),
        setting.label(),
        theme::sans(theme::c().text, theme::UI_SIZE),
    ));
    element = element.child(text(
        UiRect::new(row.left, row.top + 30.0, text_right, row.top + 48.0),
        setting.description(),
        theme::sans(theme::c().text_dim, theme::SMALL + 1.0),
    ));
    element = element.child(panel(
        UiRect::new(row.left, row.bottom - 1.0, row.right, row.bottom),
        VisualStyle::filled(theme::c().active_line),
    ));

    match setting.enabled(app) {
        Some(on) => {
            // The whole row is the hit target, like a native settings list.
            element = element
                .event_policy(EventPolicy::INTERACTIVE)
                .cursor(CursorIcon::Pointer)
                .on_click(move || state.update(move |app| setting.toggle(app)))
                .child(switch(
                    UiRect::new(
                        control_left,
                        row.top + (row.height() - SWITCH_H) / 2.0,
                        row.right,
                        row.top + (row.height() + SWITCH_H) / 2.0,
                    ),
                    on,
                ));
        }
        None => {
            element = element.child(shell_selector(
                UiRect::new(
                    control_left,
                    row.top + (row.height() - SEGMENT_H) / 2.0,
                    row.right,
                    row.top + (row.height() + SEGMENT_H) / 2.0,
                ),
                app.default_shell,
                state,
            ));
        }
    }
    element
}

fn switch(rect: UiRect, on: bool) -> Element {
    let track = if on {
        theme::c().accent
    } else {
        theme::c().text_ghost
    };
    let inset = (SWITCH_H - KNOB) / 2.0;
    let knob_left = if on {
        rect.right - inset - KNOB
    } else {
        rect.left + inset
    };
    panel(rect, VisualStyle::filled(track).radius(SWITCH_H / 2.0)).child(panel(
        UiRect::new(
            knob_left,
            rect.top + inset,
            knob_left + KNOB,
            rect.top + inset + KNOB,
        ),
        VisualStyle::filled(if on {
            theme::c().text_bright
        } else {
            theme::c().text_muted
        })
        .radius(KNOB / 2.0),
    ))
}

fn shell_selector(rect: UiRect, selected: ShellKind, state: State<AppState>) -> Element {
    let mut selector = theme::bordered(rect, theme::c().surface, theme::c().border, 4.0, 1.0);
    for (index, shell) in ShellKind::ALL.into_iter().enumerate() {
        let left = rect.left + index as f32 * SEGMENT_W;
        let segment = UiRect::new(left, rect.top, left + SEGMENT_W, rect.bottom);
        let active = shell == selected;
        let fill = if active {
            VisualStyle::filled(theme::c().selection).radius(3.0)
        } else {
            VisualStyle::default()
        };
        let mut label = theme::mono(
            if active {
                theme::c().text_bright
            } else {
                theme::c().text_muted
            },
            theme::UI_SIZE,
        );
        label.align = TextAlign::Center;
        let select_state = state.clone();
        selector = selector.child(
            panel(
                UiRect::new(
                    segment.left + 2.0,
                    segment.top + 2.0,
                    segment.right - 2.0,
                    segment.bottom - 2.0,
                ),
                fill,
            )
            .key(format!("settings-shell-{}", shell.short_label()))
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(CursorIcon::Pointer)
            .on_click(move || select_state.update(move |app| app.default_shell = shell))
            .child(text(segment, shell.short_label(), label)),
        );
    }
    selector
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_flip_their_backing_preference() {
        let mut app = AppState::new();
        for setting in [
            Setting::TerminalCursorBlink,
            Setting::InlineBlame,
            Setting::SplitDiff,
            Setting::TreeView,
        ] {
            let before = setting.enabled(&app).expect("on/off setting");
            setting.toggle(&mut app);
            assert_eq!(setting.enabled(&app), Some(!before), "{setting:?}");
        }
    }

    #[test]
    fn default_shell_is_a_choice_not_a_toggle() {
        let mut app = AppState::new();
        let before = app.default_shell;
        assert_eq!(Setting::DefaultShell.enabled(&app), None);
        Setting::DefaultShell.toggle(&mut app);
        assert_eq!(app.default_shell, before);
    }
}
