//! Status bar: cursor position, indent mode, encoding, and language badge,
//! grouped at the right edge. The terminal toggle sits on the left.
//!
//! Items are laid out from measured text widths so every gap is uniform, and
//! each text rect carries a small right margin so glyphs are never clipped by
//! the rect bounds.
//!
//! With no active file there is no cursor position or language to report, so
//! only the universal editor settings (indent mode, encoding) remain.

use lgui::core::{EventPolicy, IconStyle, UiElement, UiFocusHandle, UiId, precompiled};
use lgui::prelude::{Element, State, TextStyle, UiRect, VisualStyle, panel, text};
use lgui::text::measure_width;

use crate::state::AppState;
use crate::terminal_session::{ShellKind, TerminalTabs};
use crate::theme;

/// Padding between the bar's right edge and the last item.
const EDGE_PAD: f32 = 12.0;
/// Uniform gap between adjacent items.
const GAP: f32 = 10.0;
/// Extra right margin inside each text rect so the last glyph is not clipped.
const TEXT_MARGIN: f32 = 6.0;

/// Natural width of `s` at the status bar text size, measured with the
/// renderer's own text system so it matches the rendered pixels. Falls back
/// to a per-character estimate when no text system is installed.
fn measure(s: &str) -> f32 {
    let bounds = UiRect::new(0.0, 0.0, 10_000.0, theme::SMALL);
    measure_width(s, bounds, theme::SMALL, 400).unwrap_or_else(|| {
        s.chars().count() as f32 * theme::CHAR_W * (theme::SMALL / theme::CODE_SIZE)
    })
}

pub fn render(
    rect: UiRect,
    state: State<AppState>,
    editor_focus: UiFocusHandle,
    terminal_focus: UiFocusHandle,
    terminal_tabs: State<TerminalTabs>,
) -> Element {
    let s = state.get();

    let mut bar = panel(rect, VisualStyle::filled(theme::SIDEBAR));

    let st = state.clone();
    let terminal_sessions = terminal_tabs;
    let editor_target = editor_focus;
    let terminal_target = terminal_focus;
    let top = rect.top + (rect.height() - 14.0) / 2.0;
    let icon_r = UiRect::new(rect.left + 10.0, top, rect.left + 24.0, top + 14.0);
    let color = if s.show_terminal {
        theme::ACCENT
    } else {
        theme::ZINC_400
    };
    bar = bar.child(
        panel(
            UiRect::new(rect.left, rect.top, rect.left + 34.0, rect.bottom),
            VisualStyle::default(),
        )
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move || {
            let opening = !st.get().show_terminal;
            if opening && terminal_sessions.get().is_empty() {
                terminal_sessions.update(|tabs| {
                    tabs.add(ShellKind::default());
                });
            }
            st.update(|app| app.show_terminal = !app.show_terminal);
            if opening {
                terminal_target.focus();
            } else {
                editor_target.focus();
            }
        })
        .child(precompiled(
            UiElement::icon(UiId::new("statusbar.terminal"), icon_r, "terminal")
                .icon_style(IconStyle::new(color)),
        )),
    );

    // Right-aligned items. Without an active file, only the editor-wide
    // settings are shown (no Ln/Col position, no language badge).
    let mut items: Vec<(String, TextStyle)> = Vec::new();
    if let Some(id) = s.workspace.active() {
        let m = s
            .workspace
            .meta(id)
            .expect("active document metadata exists");
        let (line, col) = s
            .workspace
            .active_buffer()
            .expect("active buffer exists")
            .line_col();
        items.push((
            format!("Ln {}, Col {}", line + 1, col + 1),
            theme::mono(theme::ZINC_400, theme::SMALL),
        ));
        items.push((
            "Spaces: 2".to_string(),
            theme::mono(theme::ZINC_500, theme::SMALL),
        ));
        items.push((
            "UTF-8".to_string(),
            theme::mono(theme::ZINC_500, theme::SMALL),
        ));
        items.push((
            m.lang.badge().to_string(),
            theme::mono_bold(m.lang.badge_color(), theme::SMALL),
        ));
    } else {
        items.push((
            "Spaces: 2".to_string(),
            theme::mono(theme::ZINC_500, theme::SMALL),
        ));
        items.push((
            "UTF-8".to_string(),
            theme::mono(theme::ZINC_500, theme::SMALL),
        ));
    }

    let total = items.iter().map(|(s, _)| measure(s)).sum::<f32>()
        + GAP * (items.len().saturating_sub(1)) as f32;
    let mut x = rect.right - EDGE_PAD - total;
    for (label, style) in items {
        let w = measure(&label);
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
        VisualStyle::filled(theme::BORDER),
    ));

    bar
}
