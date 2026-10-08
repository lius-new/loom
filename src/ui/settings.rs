//! Settings page shown in its own editor tab.
//!
//! Every control writes straight to `AppState`; app-level effects persist the
//! values, so there is no separate apply/save step.

use lgui::core::{CursorIcon, EventPolicy, WheelUnit, clip};
use lgui::prelude::{Color, Element, State, TextAlign, UiRect, VisualStyle, group, panel, text};

use crate::input::action::Action;
use crate::state::AppState;
use crate::terminal_session::ShellKind;
use crate::theme::{self, ThemeId};
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
const THEME_CARD_H: f32 = 72.0;
const THEME_CARD_MAX_W: f32 = 168.0;
const THEME_CARD_GAP: f32 = 12.0;
const THEME_NAME_H: f32 = 24.0;
const THEME_ROW_H: f32 = ROW_H + THEME_CARD_H + 18.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Setting {
    Theme,
    DefaultShell,
    TerminalCursorBlink,
    InlineBlame,
    SplitDiff,
    TreeView,
    VimMode,
    VimSystemClipboard,
}

const SECTIONS: [(&str, &[Setting]); 4] = [
    ("APPEARANCE", &[Setting::Theme]),
    ("EDITOR", &[Setting::VimMode, Setting::VimSystemClipboard]),
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
            Self::Theme => "Color theme",
            Self::DefaultShell => "Default shell",
            Self::TerminalCursorBlink => "Cursor blinking",
            Self::InlineBlame => "Inline blame",
            Self::SplitDiff => "Side-by-side diff",
            Self::TreeView => "Tree view",
            Self::VimMode => "Vim mode",
            Self::VimSystemClipboard => "Vim: use the system clipboard",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Theme => "Colors used across the editor, panels and terminal.",
            Self::DefaultShell => "Used when a new terminal opens without picking a shell.",
            Self::TerminalCursorBlink => "Blink the terminal cursor while it has focus.",
            Self::InlineBlame => "Show the last commit for the current line in the editor.",
            Self::SplitDiff => "Open diffs with the old and new versions in two columns.",
            Self::TreeView => "Group changed files by folder in Source Control.",
            Self::VimMode => "Edit with Vim modes, operators and motions.",
            Self::VimSystemClipboard => {
                "Yank, delete and put without a register name through the clipboard."
            }
        }
    }

    /// Current value of an on/off setting; `None` for choice settings.
    fn enabled(self, app: &AppState) -> Option<bool> {
        match self {
            Self::Theme | Self::DefaultShell => None,
            Self::TerminalCursorBlink => Some(app.terminal_cursor_blink),
            Self::InlineBlame => Some(app.git_inline_blame),
            Self::SplitDiff => Some(app.git_split_diff),
            Self::TreeView => Some(app.git_tree_view),
            Self::VimMode => Some(app.vim.options.enabled),
            Self::VimSystemClipboard => Some(app.vim.options.system_clipboard),
        }
    }

    fn toggle(self, app: &mut AppState) {
        match self {
            Self::Theme | Self::DefaultShell => {}
            Self::TerminalCursorBlink => app.terminal_cursor_blink ^= true,
            Self::InlineBlame => app.git_inline_blame ^= true,
            Self::SplitDiff => app.git_split_diff ^= true,
            Self::TreeView => app.git_tree_view ^= true,
            Self::VimMode => {
                let enabled = !app.vim.options.enabled;
                crate::editor::vim_input::set_enabled(app, enabled);
            }
            Self::VimSystemClipboard => app.vim.options.system_clipboard ^= true,
        }
    }

    fn height(self) -> f32 {
        match self {
            Self::Theme => THEME_ROW_H,
            _ => ROW_H,
        }
    }
}

fn content_height() -> f32 {
    let rows: f32 = SECTIONS
        .iter()
        .flat_map(|(_, settings)| settings.iter())
        .map(|setting| setting.height())
        .sum();
    // The trailing KEYBOARD section holds a single link row.
    PAGE_PAD_TOP
        + HEADER_H
        + (SECTIONS.len() + 1) as f32 * (SECTION_TITLE_H + SECTION_GAP)
        + rows
        + ROW_H
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
        content = content.child(section_title(page_left, page_right, top, title));
        top += SECTION_TITLE_H;
        for &setting in settings {
            let row = UiRect::new(page_left, top, page_right, top + setting.height());
            content = content.child(setting_row(row, setting, &app, state.clone()));
            top += setting.height();
        }
        top += SECTION_GAP;
    }

    content = content.child(section_title(page_left, page_right, top, "KEYBOARD"));
    top += SECTION_TITLE_H;
    content = content.child(keymap_link_row(
        UiRect::new(page_left, top, page_right, top + ROW_H),
        state.clone(),
    ));

    page = page.child(content);
    clip(rect, 0.0, 0.0).child(page)
}

fn section_title(left: f32, right: f32, top: f32, title: &'static str) -> Element {
    group(UiRect::new(left, top - 1.0, right, top + SECTION_TITLE_H))
        .child(text(
            UiRect::new(left, top, right, top + SECTION_TITLE_H),
            title,
            theme::mono(theme::c().text_dim, theme::SMALL).tracking(0.8),
        ))
        .child(panel(
            UiRect::new(
                left,
                top + SECTION_TITLE_H - 1.0,
                right,
                top + SECTION_TITLE_H,
            ),
            VisualStyle::filled(theme::c().border),
        ))
}

/// Key bindings live on their own Keymap page; Settings links to it.
fn keymap_link_row(row: UiRect, state: State<AppState>) -> Element {
    let link = "Open Keymap";
    let link_w = measure(link, theme::UI_SIZE, 400) + TEXT_MARGIN;
    let text_right = (row.right - link_w - CONTROL_GAP).max(row.left);
    let mut link_style = theme::sans(theme::c().accent, theme::UI_SIZE);
    link_style.align = TextAlign::Right;
    panel(row, VisualStyle::default())
        .key("settings-open-keymap")
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_click(move || state.update(|app| app.apply(Action::OpenKeymap)))
        .child(text(
            UiRect::new(row.left, row.top + 11.0, text_right, row.top + 29.0),
            "Keyboard shortcuts",
            theme::sans(theme::c().text, theme::UI_SIZE),
        ))
        .child(text(
            UiRect::new(row.left, row.top + 30.0, text_right, row.top + 48.0),
            "View every action and its key binding, and customize them in keymap.json.",
            theme::sans(theme::c().text_dim, theme::SMALL + 1.0),
        ))
        .child(text(
            UiRect::new(
                row.right - link_w,
                row.top + 11.0,
                row.right,
                row.top + 29.0,
            ),
            link,
            link_style,
        ))
        .child(panel(
            UiRect::new(row.left, row.bottom - 1.0, row.right, row.bottom),
            VisualStyle::filled(theme::c().active_line),
        ))
}

fn setting_row(row: UiRect, setting: Setting, app: &AppState, state: State<AppState>) -> Element {
    let control_w = match setting {
        // The theme cards sit below the label, so the text spans the row.
        Setting::Theme => 0.0,
        Setting::DefaultShell => SEGMENT_W * ShellKind::ALL.len() as f32,
        _ => SWITCH_W,
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
        None if setting == Setting::Theme => {
            let cards_top = row.top + ROW_H - 4.0;
            element = element.child(theme_picker(
                UiRect::new(row.left, cards_top, row.right, cards_top + THEME_CARD_H),
                app.theme,
                state,
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
            theme::c().on_accent
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

/// One card per built-in theme, each drawn in its own palette with a tiny
/// code preview so the choice is visible before it is applied.
fn theme_picker(rect: UiRect, selected: ThemeId, state: State<AppState>) -> Element {
    let count = ThemeId::all().count() as f32;
    let card_w =
        ((rect.width() - THEME_CARD_GAP * (count - 1.0)) / count).clamp(0.0, THEME_CARD_MAX_W);
    let mut picker = group(rect);
    for (index, id) in ThemeId::all().enumerate() {
        let entry = id.theme();
        let p = &entry.palette;
        let active = id == selected;
        let left = rect.left + index as f32 * (card_w + THEME_CARD_GAP);
        let card = UiRect::new(left, rect.top, left + card_w, rect.bottom);
        let (ring, ring_w) = if active {
            (theme::c().accent, 2.0)
        } else {
            (theme::c().border, 1.0)
        };

        // Mini code lines: (indent, [(width, color)]).
        let lines: [(f32, &[(f32, Color)]); 3] = [
            (0.0, &[(18.0, p.syntax.keyword), (30.0, p.syntax.function)]),
            (10.0, &[(22.0, p.syntax.property), (36.0, p.syntax.string)]),
            (10.0, &[(44.0, p.syntax.comment)]),
        ];
        let mut element = theme::bordered(card, p.bg, ring, 6.0, ring_w);
        for (row, (indent, bars)) in lines.into_iter().enumerate() {
            let y = card.top + 12.0 + row as f32 * 9.0;
            let mut x = card.left + 12.0 + indent;
            for &(width, color) in bars {
                let right = (x + width).min(card.right - 12.0);
                if right > x {
                    element = element.child(panel(
                        UiRect::new(x, y, right, y + 4.0),
                        VisualStyle::filled(color).radius(2.0),
                    ));
                }
                x += width + 5.0;
            }
        }
        let name_top = card.bottom - THEME_NAME_H;
        element = element
            .child(panel(
                UiRect::new(
                    card.left + ring_w,
                    name_top,
                    card.right - ring_w,
                    name_top + 1.0,
                ),
                VisualStyle::filled(p.border),
            ))
            .child(text(
                UiRect::new(
                    card.left + 10.0,
                    name_top + 4.0,
                    card.right - 22.0,
                    card.bottom - 2.0,
                ),
                entry.name,
                theme::sans(
                    if active { p.text_bright } else { p.text_muted },
                    theme::SMALL + 1.0,
                ),
            ))
            .child(panel(
                UiRect::new(
                    card.right - 16.0,
                    name_top + 8.0,
                    card.right - 8.0,
                    name_top + 16.0,
                ),
                VisualStyle::filled(p.accent).radius(4.0),
            ));

        let select_state = state.clone();
        picker = picker.child(
            element
                .key(format!("settings-theme-{}", entry.id))
                .event_policy(EventPolicy::INTERACTIVE)
                .cursor(CursorIcon::Pointer)
                .on_click(move || select_state.update(move |app| app.theme = id)),
        );
    }
    picker
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
            Setting::VimMode,
            Setting::VimSystemClipboard,
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

    #[test]
    fn theme_is_a_choice_not_a_toggle() {
        let mut app = AppState::new();
        let before = app.theme;
        assert_eq!(Setting::Theme.enabled(&app), None);
        Setting::Theme.toggle(&mut app);
        assert_eq!(app.theme, before);
    }
}
