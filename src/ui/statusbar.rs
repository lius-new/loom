//! Status bar: cursor position, indent mode, encoding, and language badge,
//! grouped at the right edge. The terminal and Source Control toggles sit on
//! the left; repository controls live in the title bar.
//!
//! Items are laid out from measured text widths so every gap is uniform, and
//! each text rect carries a small right margin so glyphs are never clipped by
//! the rect bounds.
//!
//! With no active file there is no cursor position or language to report, so
//! only the universal editor settings (indent mode, encoding) remain.

use lgui::core::{
    EventPolicy, IconStyle, SemanticRole, Semantics, UiElement, UiFocusHandle, UiId, precompiled,
};
use lgui::prelude::{Element, State, TextStyle, UiRect, VisualStyle, panel, text};

use crate::input::action::Action;
use crate::state::AppState;
use crate::theme;

/// Padding between the bar's right edge and the last item.
const EDGE_PAD: f32 = 12.0;
/// Uniform gap between adjacent items.
const GAP: f32 = 10.0;
/// Extra right margin inside each text rect so the last glyph is not clipped.
const TEXT_MARGIN: f32 = 6.0;

/// Natural width of `s` in the monospace status bar font at `weight`,
/// measured with the renderer's text system so it matches the rendered
/// pixels. Measuring with the default sans font underestimated monospace
/// labels, and a too-narrow rect wraps text such as `feat/keymap` at the `/`.
fn measure(s: &str, weight: i32) -> f32 {
    super::tabs::measure(s, theme::SMALL, weight)
}

pub fn render(
    rect: UiRect,
    state: State<AppState>,
    editor_focus: UiFocusHandle,
    terminal_focus: UiFocusHandle,
    terminal_tabs: crate::app::terminal_binding::TerminalStore,
) -> Element {
    let s = state.get();

    let mut bar = panel(rect, VisualStyle::filled(theme::c().sidebar));

    let st = state.clone();
    let terminal_sessions = terminal_tabs;
    let editor_target = editor_focus;
    let terminal_target = terminal_focus;
    let top = rect.top + (rect.height() - theme::ICON_SIZE) / 2.0;
    let icon_r = UiRect::new(
        rect.left + 10.0,
        top,
        rect.left + 10.0 + theme::ICON_SIZE,
        top + theme::ICON_SIZE,
    );
    let color = if s.default_terminal_open() {
        theme::c().accent
    } else {
        theme::c().text_muted
    };
    bar = bar.child(
        panel(
            UiRect::new(rect.left, rect.top, rect.left + 34.0, rect.bottom),
            VisualStyle::default(),
        )
        .event_policy(EventPolicy {
            hover: true,
            press: true,
            focus: false,
        })
        .on_click(move || {
            crate::app::terminal::toggle_panel(
                &st,
                &terminal_sessions,
                &editor_target,
                &terminal_target,
            );
        })
        .child(precompiled(
            UiElement::icon(UiId::new("statusbar.terminal"), icon_r, "terminal")
                .icon_style(IconStyle::new(color)),
        )),
    );
    // Right-aligned items. Without an active file, only the editor-wide
    // settings are shown (no Ln/Col position, no language badge).
    let mut items: Vec<(String, TextStyle)> = Vec::new();
    // A key sequence waiting for its next keystroke, e.g. `Ctrl+K …`.
    if !s.pending_keystrokes.is_empty() {
        items.push((
            format!(
                "{} …",
                crate::input::keystroke::Keystroke::sequence_label(&s.pending_keystrokes)
            ),
            theme::mono_bold(theme::c().accent, theme::SMALL),
        ));
    }
    if let Some(id) = s.workspace.active() {
        let m = s
            .workspace
            .meta(id)
            .expect("active document metadata exists");
        if s.workspace.is_missing_on_disk(id) {
            items.push((
                "Deleted on Disk — Save Recreates".to_string(),
                theme::mono_bold(theme::c().error, theme::SMALL),
            ));
        } else if s.workspace.has_disk_conflict(id) {
            items.push((
                "Changed on Disk".to_string(),
                theme::mono_bold(theme::c().warning, theme::SMALL),
            ));
        }
        if let Some(buffer) = s.workspace.active_editor() {
            let (line, col) = buffer.line_col();
            items.push((
                format!("Ln {}, Col {}", line + 1, col + 1),
                theme::mono(theme::c().text_muted, theme::SMALL),
            ));
            items.push((
                "Spaces: 2".to_string(),
                theme::mono(theme::c().text_dim, theme::SMALL),
            ));
            items.push((
                "UTF-8".to_string(),
                theme::mono(theme::c().text_dim, theme::SMALL),
            ));
        } else if s.workspace.is_diff(id) {
            items.push((
                "Text Diff".to_string(),
                theme::mono(theme::c().text_dim, theme::SMALL),
            ));
        }
        if s.workspace.is_file(id) || s.workspace.is_diff(id) {
            items.push((
                m.lang.badge().to_string(),
                theme::mono_bold(m.lang.badge_color(), theme::SMALL),
            ));
        }
    } else {
        items.push((
            "Spaces: 2".to_string(),
            theme::mono(theme::c().text_dim, theme::SMALL),
        ));
        items.push((
            "UTF-8".to_string(),
            theme::mono(theme::c().text_dim, theme::SMALL),
        ));
    }

    // Source Control panel toggle, beside the terminal toggle.
    let scm_slot = UiRect::new(rect.left + 34.0, rect.top, rect.left + 62.0, rect.bottom);
    let scm_state = state.clone();
    bar = bar.child(
        panel(scm_slot, VisualStyle::default())
            .event_policy(EventPolicy::INTERACTIVE)
            .semantics(Semantics::new(SemanticRole::Button).name("Toggle Source Control"))
            .on_click(move || {
                scm_state.update(|app| app.apply(Action::ToggleSourceControl));
            })
            .child(precompiled(
                UiElement::icon(
                    UiId::new("statusbar.source-control"),
                    UiRect::new(
                        scm_slot.left + 6.0,
                        top,
                        scm_slot.left + 6.0 + theme::ICON_SIZE,
                        top + theme::ICON_SIZE,
                    ),
                    "source-control",
                )
                .icon_style(IconStyle::new(if s.source_control.visible {
                    theme::c().accent
                } else {
                    theme::c().text_muted
                })),
            )),
    );

    let mut left = scm_slot.right + GAP;
    if s.git.model.diff_loading().is_some() {
        bar = bar.child(text(
            UiRect::new(left, rect.top, left + 140.0, rect.bottom),
            "Loading diff…",
            theme::mono(theme::c().text_dim, theme::SMALL),
        ));
        left += 140.0 + GAP;
    }
    if crate::app::editor_vim::is_active(&s) {
        for (label, style) in vim_items(&s.editor.vim) {
            let width = measure(&label, style.weight) + TEXT_MARGIN;
            bar = bar.child(text(
                UiRect::new(left, rect.top, left + width, rect.bottom),
                label,
                style,
            ));
            left += width + GAP;
        }
    }

    let total = items
        .iter()
        .map(|(s, style)| measure(s, style.weight))
        .sum::<f32>()
        + GAP * (items.len().saturating_sub(1)) as f32;
    let mut x = rect.right - EDGE_PAD - total;
    for (label, style) in items {
        let w = measure(&label, style.weight);
        bar = bar.child(text(
            UiRect::new(x, rect.top, x + w + TEXT_MARGIN, rect.bottom),
            label,
            style,
        ));
        x += w + GAP;
    }

    // Hairline top border (matches the title bar's bottom border).
    bar = bar.child(panel(
        UiRect::new(rect.left, rect.top, rect.right, rect.top + 1.0),
        VisualStyle::filled(theme::c().border),
    ));

    bar
}

/// Vim's mode, then its command line, message or pending keys, and macro
/// recording, left to right.
fn vim_items(vim: &crate::vim::Vim) -> Vec<(String, TextStyle)> {
    use crate::vim::Mode;
    let status = vim.status();
    let mode_color = match vim.mode() {
        Mode::Normal => theme::c().text_muted,
        Mode::Insert => theme::c().accent,
        Mode::Replace => theme::c().error,
        Mode::Visual(_) => theme::c().warning,
    };
    let mut items = vec![(
        format!("-- {} --", status.mode),
        theme::mono_bold(mode_color, theme::SMALL),
    )];
    if let Some(line) = status.command_line {
        items.push((line, theme::mono(theme::c().text_bright, theme::SMALL)));
    } else if let Some(message) = status.message {
        let color = if message.error {
            theme::c().error
        } else {
            theme::c().text_soft
        };
        items.push((message.text, theme::mono(color, theme::SMALL)));
    }
    if !status.pending.is_empty() {
        items.push((
            status.pending,
            theme::mono_bold(theme::c().accent, theme::SMALL),
        ));
    }
    if let Some(register) = status.recording {
        items.push((
            format!("recording @{register}"),
            theme::mono(theme::c().warning, theme::SMALL),
        ));
    }
    items
}
