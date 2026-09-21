//! Status bar: cursor position, indent mode, encoding, and language badge,
//! grouped at the right edge. The left side is intentionally empty.
//!
//! Items are laid out from measured text widths so every gap is uniform, and
//! each text rect carries a small right margin so glyphs are never clipped by
//! the rect bounds.
//!
//! With no active file there is no cursor position or language to report, so
//! only the universal editor settings (indent mode, encoding) remain.

use lgui::prelude::{panel, text, Element, State, TextStyle, UiRect, VisualStyle};
use lgui::text::measure_width;

use crate::model::document::meta;
use crate::state::AppState;
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

pub fn render(rect: UiRect, state: State<AppState>) -> Element {
    let s = state.get();

    let mut bar = panel(rect, VisualStyle::filled(theme::SIDEBAR));

    // Right-aligned items. Without an active file, only the editor-wide
    // settings are shown (no Ln/Col position, no language badge).
    let mut items: Vec<(String, TextStyle)> = Vec::new();
    if let Some(id) = s.workspace.active() {
        let m = meta(id);
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
