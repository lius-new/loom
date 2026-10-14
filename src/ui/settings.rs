//! Settings page shown in its own editor tab.
//!
//! Every control writes straight to `AppState`; app-level effects persist the
//! values, so there is no separate apply/save step.

use lgui::core::{CursorIcon, EventPolicy, TextStyle, WheelUnit, clip};
use lgui::prelude::{
    Color, Element, State, TextAlign, UiRect, VisualStyle, component, group, panel, text,
};

use crate::fonts::{self, FontSettings};
use crate::input::action::Action;
use crate::state::AppState;
use crate::theme::{self, ThemeId};
use crate::ui::components::input::{self, InputBinding, InputOptions, InputState, InputStyle};
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
const FAMILY_INPUT_W: f32 = 220.0;
const FAMILY_INPUT_H: f32 = 26.0;
const PRESET_H: f32 = 24.0;
const PRESET_GAP: f32 = 8.0;
const FAMILY_ROW_H: f32 = ROW_H + PRESET_H + 12.0;
const STEP_BUTTON_W: f32 = 26.0;
const STEP_VALUE_W: f32 = 44.0;
const STEPPER_W: f32 = STEP_BUTTON_W * 2.0 + STEP_VALUE_W;
const STEPPER_H: f32 = 26.0;

/// Families offered as one-click choices under the font family field. They
/// are only suggestions: one that is not installed falls back to the
/// built-in monospace stack.
const FONT_PRESETS: [&str; 5] = [
    "Cascadia Code",
    "Consolas",
    "JetBrains Mono",
    "Fira Code",
    "Source Code Pro",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Setting {
    Theme,
    EditorFontFamily,
    EditorFontSize,
    EditorLineHeight,
    TerminalFontFamily,
    TerminalFontSize,
    TerminalLineHeight,
    DefaultShell,
    TerminalCursorBlink,
    SmoothCaret,
    InlineBlame,
    SplitDiff,
    TreeView,
    VimMode,
    VimSystemClipboard,
}

const SECTIONS: [(&str, &[Setting]); 4] = [
    ("APPEARANCE", &[Setting::Theme]),
    (
        "EDITOR",
        &[
            Setting::EditorFontFamily,
            Setting::EditorFontSize,
            Setting::EditorLineHeight,
            Setting::SmoothCaret,
            Setting::VimMode,
            Setting::VimSystemClipboard,
        ],
    ),
    (
        "TERMINAL",
        &[
            Setting::TerminalFontFamily,
            Setting::TerminalFontSize,
            Setting::TerminalLineHeight,
            Setting::DefaultShell,
            Setting::TerminalCursorBlink,
        ],
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
            Self::EditorFontFamily | Self::TerminalFontFamily => "Font family",
            Self::EditorFontSize | Self::TerminalFontSize => "Font size",
            Self::EditorLineHeight | Self::TerminalLineHeight => "Line height",
            Self::DefaultShell => "Default shell",
            Self::TerminalCursorBlink => "Cursor blinking",
            Self::SmoothCaret => "Smooth caret movement",
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
            Self::EditorFontFamily => {
                "Font for code, diffs and change previews. Leave empty for the default."
            }
            Self::EditorFontSize => "Size of code text in pixels.",
            Self::EditorLineHeight => "Height of each code line, as a multiple of the font size.",
            Self::TerminalFontFamily => {
                "Font for terminal text. Leave empty to use the editor font."
            }
            Self::TerminalFontSize => "Size of terminal text in pixels.",
            Self::TerminalLineHeight => {
                "Height of each terminal row, as a multiple of the font size."
            }
            Self::DefaultShell => "Used when a new terminal opens without picking a shell.",
            Self::TerminalCursorBlink => "Blink the terminal cursor while it has focus.",
            Self::SmoothCaret => "Animate the editor caret when typing, navigating or clicking.",
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
            Self::EditorFontFamily
            | Self::EditorFontSize
            | Self::EditorLineHeight
            | Self::TerminalFontFamily
            | Self::TerminalFontSize
            | Self::TerminalLineHeight => None,
            Self::TerminalCursorBlink => Some(app.terminal_cursor_blink),
            Self::SmoothCaret => Some(app.smooth_caret),
            Self::InlineBlame => Some(app.git.ui.inline_blame),
            Self::SplitDiff => Some(app.git.ui.split_diff),
            Self::TreeView => Some(app.git.ui.tree_view),
            Self::VimMode => Some(app.editor.vim.options.enabled),
            Self::VimSystemClipboard => Some(app.editor.vim.options.system_clipboard),
        }
    }

    fn toggle(self, app: &mut AppState) {
        match self {
            Self::Theme | Self::DefaultShell => {}
            Self::EditorFontFamily
            | Self::EditorFontSize
            | Self::EditorLineHeight
            | Self::TerminalFontFamily
            | Self::TerminalFontSize
            | Self::TerminalLineHeight => {}
            Self::TerminalCursorBlink => app.terminal_cursor_blink ^= true,
            Self::SmoothCaret => app.smooth_caret ^= true,
            Self::InlineBlame => app.git.ui.inline_blame ^= true,
            Self::SplitDiff => app.git.ui.split_diff ^= true,
            Self::TreeView => app.git.ui.tree_view ^= true,
            Self::VimMode => {
                let enabled = !app.editor.vim.options.enabled;
                crate::app::editor_vim::set_enabled(app, enabled);
            }
            Self::VimSystemClipboard => app.editor.vim.options.system_clipboard ^= true,
        }
    }

    fn height(self, app: &AppState) -> f32 {
        match self {
            Self::Theme => THEME_ROW_H,
            Self::EditorFontFamily | Self::TerminalFontFamily => FAMILY_ROW_H,
            Self::DefaultShell => crate::ui::terminal::settings::height(
                &crate::app::terminal_profiles::settings_frame(app),
                &app.terminal.ui,
            ),
            _ => ROW_H,
        }
    }

    /// The font a font setting edits and the part of it this row controls.
    fn font(self) -> Option<(FontTarget, FontField)> {
        Some(match self {
            Self::EditorFontFamily => (FontTarget::Editor, FontField::Family),
            Self::EditorFontSize => (FontTarget::Editor, FontField::Size),
            Self::EditorLineHeight => (FontTarget::Editor, FontField::LineHeight),
            Self::TerminalFontFamily => (FontTarget::Terminal, FontField::Family),
            Self::TerminalFontSize => (FontTarget::Terminal, FontField::Size),
            Self::TerminalLineHeight => (FontTarget::Terminal, FontField::LineHeight),
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FontTarget {
    Editor,
    Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FontField {
    Family,
    Size,
    LineHeight,
}

impl FontTarget {
    fn get(self, app: &AppState) -> &FontSettings {
        match self {
            Self::Editor => &app.editor_font,
            Self::Terminal => &app.terminal_font,
        }
    }

    fn set(self, app: &mut AppState, font: FontSettings) {
        match self {
            Self::Editor => app.set_editor_font(font),
            Self::Terminal => app.set_terminal_font(font),
        }
    }

    fn set_family(self, app: &mut AppState, family: &str) {
        let font = FontSettings {
            family: family.trim().to_owned(),
            ..self.get(app).clone()
        };
        self.set(app, font);
        // Normalize the field even when the stored family did not change.
        let family = self.get(app).family.clone();
        self.input_mut(app).set_text(family);
    }

    fn input(self, app: &AppState) -> &InputState {
        match self {
            Self::Editor => &app.editor_font_input,
            Self::Terminal => &app.terminal_font_input,
        }
    }

    fn input_mut(self, app: &mut AppState) -> &mut InputState {
        match self {
            Self::Editor => &mut app.editor_font_input,
            Self::Terminal => &mut app.terminal_font_input,
        }
    }

    fn commit_input(self, app: &mut AppState) {
        let family = self.input(app).text().to_owned();
        self.set_family(app, &family);
    }

    fn revert_input(self, app: &mut AppState) {
        let family = self.get(app).family.clone();
        self.input_mut(app).set_text(family);
    }
}

/// A line-height percentage as a multiplier, e.g. 160 → "1.6", 155 → "1.55".
fn line_height_label(percent: u16) -> String {
    let (whole, hundredths) = (percent / 100, percent % 100);
    if hundredths % 10 == 0 {
        format!("{whole}.{}", hundredths / 10)
    } else {
        format!("{whole}.{hundredths:02}")
    }
}

impl FontField {
    /// Displayed value and whether it can step down / up.
    fn value(self, font: &FontSettings) -> (String, bool, bool) {
        match self {
            Self::Family => (font.family.clone(), false, false),
            Self::Size => (
                font.size.to_string(),
                font.size > fonts::MIN_SIZE,
                font.size < fonts::MAX_SIZE,
            ),
            Self::LineHeight => (
                line_height_label(font.line_height),
                font.line_height > fonts::MIN_LINE_HEIGHT,
                font.line_height < fonts::MAX_LINE_HEIGHT,
            ),
        }
    }

    fn step(self, font: &FontSettings, up: bool) -> FontSettings {
        let mut font = font.clone();
        let step = |value: u16, by: u16| {
            if up {
                value.saturating_add(by)
            } else {
                value.saturating_sub(by)
            }
        };
        match self {
            Self::Family => {}
            Self::Size => font.size = step(font.size, 1),
            Self::LineHeight => {
                // Snap to the step grid so a hand-edited value lines up again.
                let snapped = font.line_height / fonts::LINE_HEIGHT_STEP * fonts::LINE_HEIGHT_STEP;
                font.line_height = if up || snapped == font.line_height {
                    step(snapped, fonts::LINE_HEIGHT_STEP)
                } else {
                    snapped
                };
            }
        }
        font.clamped()
    }
}

fn content_height(app: &AppState) -> f32 {
    let rows: f32 = SECTIONS
        .iter()
        .flat_map(|(_, settings)| settings.iter())
        .map(|setting| setting.height(app))
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
    let max_scroll = (content_height(&app) - rect.height()).max(0.0);
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
    content = content.child(settings_file_link(
        UiRect::new(page_left, top, page_right, top + 26.0),
        state.clone(),
    ));
    content = content.child(text(
        UiRect::new(page_left, top + 28.0, page_right, top + 48.0),
        "Changes apply immediately and are saved to settings.json.",
        theme::sans(theme::c().text_dim, theme::UI_SIZE),
    ));
    top += HEADER_H;

    for (title, settings) in SECTIONS {
        content = content.child(section_title(page_left, page_right, top, title));
        top += SECTION_TITLE_H;
        for &setting in settings {
            let row = UiRect::new(page_left, top, page_right, top + setting.height(&app));
            content = content.child(setting_row(row, setting, &app, state.clone()));
            top += setting.height(&app);
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

/// "Edit in settings.json", right-aligned on the title row.
fn settings_file_link(row: UiRect, state: State<AppState>) -> Element {
    let label = "Edit in settings.json";
    let width = measure(label, theme::UI_SIZE, 400) + TEXT_MARGIN;
    let link = UiRect::new(
        (row.right - width).max(row.left),
        row.top,
        row.right,
        row.bottom,
    );
    let mut style = theme::sans(theme::c().accent, theme::UI_SIZE);
    style.align = TextAlign::Right;
    panel(link, VisualStyle::default())
        .key("settings-open-settings-file")
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            crate::key_actions::open_user_settings(&state);
        })
        .child(text(link, label, style))
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
    if setting == Setting::DefaultShell {
        return crate::ui::terminal::settings::render(
            row,
            crate::app::terminal_profiles::binding(&state),
        );
    }
    let control_w = match setting.font() {
        Some((_, FontField::Family)) => FAMILY_INPUT_W,
        Some(_) => STEPPER_W,
        // The theme cards sit below the label, so the text spans the row.
        None if setting == Setting::Theme => 0.0,
        None => SWITCH_W,
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
            let Some((target, field)) = setting.font() else {
                return element;
            };
            let control_top = row.top + (ROW_H - STEPPER_H) / 2.0;
            if field == FontField::Family {
                element = element
                    .child(family_input(
                        UiRect::new(
                            control_left,
                            control_top,
                            row.right,
                            control_top + FAMILY_INPUT_H,
                        ),
                        target,
                        state.clone(),
                    ))
                    .child(font_presets(
                        UiRect::new(
                            row.left,
                            row.top + ROW_H - 6.0,
                            row.right,
                            row.top + ROW_H - 6.0 + PRESET_H,
                        ),
                        target,
                        &target.get(app).family,
                        state,
                    ));
            } else {
                element = element.child(stepper(
                    UiRect::new(
                        control_left,
                        control_top,
                        row.right,
                        control_top + STEPPER_H,
                    ),
                    target,
                    field,
                    target.get(app),
                    state,
                ));
            }
        }
    }
    element
}

/// Free-form family name, committed on Enter or when focus leaves.
fn family_input(frame: UiRect, target: FontTarget, state: State<AppState>) -> Element {
    type Read = fn(&AppState) -> &InputState;
    type Write = fn(&mut AppState) -> &mut InputState;
    type Action = fn(&State<AppState>);
    let (read, write, commit, cancel, placeholder): (Read, Write, Action, Action, &str) =
        match target {
            FontTarget::Editor => (
                |app| &app.editor_font_input,
                |app| &mut app.editor_font_input,
                |state| state.update(|app| FontTarget::Editor.commit_input(app)),
                |state| state.update(|app| FontTarget::Editor.revert_input(app)),
                "Default (Cascadia Mono)",
            ),
            FontTarget::Terminal => (
                |app| &app.terminal_font_input,
                |app| &mut app.terminal_font_input,
                |state| state.update(|app| FontTarget::Terminal.commit_input(app)),
                |state| state.update(|app| FontTarget::Terminal.revert_input(app)),
                "Same as editor",
            ),
        };
    let input_state = state;
    theme::bordered(frame, theme::c().surface, theme::c().border, 4.0, 1.0).child(
        component(
            UiRect::new(
                frame.left + 4.0,
                frame.top + 1.0,
                (frame.right - 4.0).max(frame.left + 4.0),
                frame.bottom - 1.0,
            ),
            move |cx, &rect| {
                let id = cx.use_stable_id();
                let focus = cx.focus_handle(id.clone());
                let focus_state = input_state.clone();
                input::render(
                    rect,
                    id,
                    focus,
                    InputBinding::new(input_state.clone(), read, write),
                    InputOptions {
                        label: "Font family",
                        placeholder,
                        style: InputStyle {
                            text: theme::mono(theme::c().text, theme::UI_SIZE),
                            placeholder: theme::mono(theme::c().text_faint, theme::UI_SIZE),
                            caret: theme::c().text,
                            selection: theme::c().selection,
                        },
                        on_submit: Some(commit),
                        on_cancel: Some(cancel),
                        on_blur: Some(commit),
                    },
                )
                .on_focus(move |_| {
                    focus_state.update(|app| app.terminal.ui.terminal_focused = false)
                })
            },
        )
        .key(match target {
            FontTarget::Editor => "settings-editor-font-family",
            FontTarget::Terminal => "settings-terminal-font-family",
        }),
    )
}

/// One-click family choices, each drawn in its own font. The first chip
/// clears the field back to the inherited family.
fn font_presets(
    rect: UiRect,
    target: FontTarget,
    current: &str,
    state: State<AppState>,
) -> Element {
    let default_label = match target {
        FontTarget::Editor => "Default",
        FontTarget::Terminal => "Same as editor",
    };
    let mut row = group(rect);
    let mut left = rect.left;
    for (family, label) in std::iter::once(("", default_label))
        .chain(FONT_PRESETS.iter().map(|family| (*family, *family)))
    {
        let width = measure(label, theme::SMALL + 1.0, 400) + 20.0;
        if left + width > rect.right {
            break;
        }
        let chip = UiRect::new(left, rect.top, left + width, rect.bottom);
        let active = current.eq_ignore_ascii_case(family);
        let (fill, border, color) = if active {
            (
                theme::c().selection,
                theme::c().accent,
                theme::c().text_bright,
            )
        } else {
            (theme::c().surface, theme::c().border, theme::c().text_muted)
        };
        let mut text_style =
            TextStyle::new(color, theme::SMALL + 1.0, 400).font_families(fonts::families(family));
        text_style.align = TextAlign::Center;
        let select_state = state.clone();
        row = row.child(
            theme::bordered(chip, fill, border, 4.0, 1.0)
                .key(format!("settings-font-preset-{target:?}-{label}"))
                .event_policy(EventPolicy::INTERACTIVE)
                .cursor(CursorIcon::Pointer)
                .on_click(move || select_state.update(move |app| target.set_family(app, family)))
                .child(text(chip, label, text_style)),
        );
        left += width + PRESET_GAP;
    }
    row
}

/// `[−] value [+]` for a numeric font setting.
fn stepper(
    rect: UiRect,
    target: FontTarget,
    field: FontField,
    font: &FontSettings,
    state: State<AppState>,
) -> Element {
    let (value, can_down, can_up) = field.value(font);
    let mut value_style = theme::mono(theme::c().text, theme::UI_SIZE);
    value_style.align = TextAlign::Center;
    let mut element =
        theme::bordered(rect, theme::c().surface, theme::c().border, 4.0, 1.0).child(text(
            UiRect::new(
                rect.left + STEP_BUTTON_W,
                rect.top,
                rect.right - STEP_BUTTON_W,
                rect.bottom,
            ),
            value,
            value_style,
        ));
    for (up, enabled) in [(false, can_down), (true, can_up)] {
        let button = if up {
            UiRect::new(
                rect.right - STEP_BUTTON_W,
                rect.top,
                rect.right,
                rect.bottom,
            )
        } else {
            UiRect::new(rect.left, rect.top, rect.left + STEP_BUTTON_W, rect.bottom)
        };
        let mut glyph_style = theme::mono(
            if enabled {
                theme::c().text
            } else {
                theme::c().text_ghost
            },
            theme::UI_SIZE + 2.0,
        );
        glyph_style.align = TextAlign::Center;
        let mut control = panel(button, VisualStyle::default())
            .key(format!(
                "settings-font-{target:?}-{field:?}-{}",
                if up { "up" } else { "down" }
            ))
            .child(text(button, if up { "+" } else { "\u{2212}" }, glyph_style));
        if enabled {
            let step_state = state.clone();
            control = control
                .event_policy(EventPolicy::INTERACTIVE)
                .cursor(CursorIcon::Pointer)
                .on_click(move || {
                    step_state.update(move |app| {
                        let font = field.step(target.get(app), up);
                        target.set(app, font);
                    })
                });
        }
        element = element.child(control);
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
            Setting::SmoothCaret,
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

    #[test]
    fn font_steppers_move_within_their_range() {
        let font = FontSettings::EDITOR;
        assert_eq!(FontField::Size.step(&font, true).size, 13);
        assert_eq!(FontField::Size.step(&font, false).size, 11);
        assert_eq!(FontField::LineHeight.step(&font, true).line_height, 210);

        let smallest = FontSettings {
            size: fonts::MIN_SIZE,
            line_height: fonts::MIN_LINE_HEIGHT,
            ..font.clone()
        };
        assert_eq!(FontField::Size.step(&smallest, false).size, fonts::MIN_SIZE);
        let (_, can_down, can_up) = FontField::Size.value(&smallest);
        assert!(!can_down && can_up);
        assert_eq!(
            FontField::LineHeight.step(&smallest, false).line_height,
            fonts::MIN_LINE_HEIGHT
        );

        let off_grid = FontSettings {
            line_height: 155,
            ..font.clone()
        };
        assert_eq!(
            FontField::LineHeight.step(&off_grid, false).line_height,
            150
        );
        assert_eq!(FontField::LineHeight.step(&off_grid, true).line_height, 160);
        assert_eq!(FontField::LineHeight.value(&off_grid).0, "1.55");
        assert_eq!(FontField::LineHeight.value(&font).0, "2.0");
    }

    #[test]
    fn family_input_commits_trimmed_and_reverts_on_cancel() {
        let mut app = AppState::new();
        app.editor_font_input.set_text("  Fira Code ");
        FontTarget::Editor.commit_input(&mut app);
        assert_eq!(app.editor_font.family, "Fira Code");
        assert_eq!(app.editor_font_input.text(), "Fira Code");

        app.editor_font_input.set_text("Typo");
        FontTarget::Editor.revert_input(&mut app);
        assert_eq!(app.editor_font_input.text(), "Fira Code");

        FontTarget::Terminal.set_family(&mut app, "Consolas");
        assert_eq!(app.terminal_font.family, "Consolas");
        assert_eq!(app.terminal_font_input.text(), "Consolas");
        assert_eq!(app.editor_font.family, "Fira Code");
    }

    #[test]
    fn font_settings_are_choices_not_toggles() {
        let mut app = AppState::new();
        for setting in [
            Setting::EditorFontFamily,
            Setting::EditorFontSize,
            Setting::EditorLineHeight,
            Setting::TerminalFontFamily,
            Setting::TerminalFontSize,
            Setting::TerminalLineHeight,
        ] {
            assert_eq!(setting.enabled(&app), None, "{setting:?}");
            assert!(setting.font().is_some(), "{setting:?}");
            setting.toggle(&mut app);
        }
        assert_eq!(app.editor_font, FontSettings::EDITOR);
        assert_eq!(app.terminal_font, FontSettings::TERMINAL);
    }

    #[test]
    fn font_size_actions_zoom_and_keep_the_top_line() {
        let mut app = AppState::new();
        app.workspace.open_path(
            "zoom.rs".into(),
            "fn main() {}
"
            .repeat(100),
        );
        app.workspace.set_active_scroll(0.0, 240.0);
        app.apply(Action::IncreaseFontSize);
        assert_eq!(app.editor_font.size, 13);
        // 13px at 200% rounds to 26px lines: line 10 stays at the top.
        assert_eq!(app.workspace.active_scroll().1, 260.0);
        app.apply(Action::ResetFontSize);
        assert_eq!(app.editor_font, FontSettings::EDITOR);
        assert_eq!(app.workspace.active_scroll().1, 240.0);
        for _ in 0..40 {
            app.apply(Action::DecreaseFontSize);
        }
        assert_eq!(app.editor_font.size, fonts::MIN_SIZE);
    }

    #[test]
    fn settings_file_values_apply_and_round_trip() {
        let mut app = AppState::new();
        assert!(!app.apply_settings(app.settings()));

        let file = crate::settings_persistence::parse(
            r#"{ "smooth_caret": false, "vim": { "enabled": true }, "editor_font": { "family": "Consolas", "size": 99 } }"#,
        )
        .unwrap();
        assert!(app.apply_settings(file.settings));
        assert!(!app.smooth_caret);
        assert!(app.editor.vim.options.enabled);
        assert_eq!(app.editor_font.family, "Consolas");
        assert_eq!(app.editor_font_input.text(), "Consolas");
        // Out-of-range values are clamped, so the next save normalizes them.
        assert_eq!(app.editor_font.size, fonts::MAX_SIZE);
        assert_eq!(app.settings().editor_font.size, fonts::MAX_SIZE);
    }
}
