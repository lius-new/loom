//! Status bar: cursor position, indent mode, encoding, and language badge,
//! grouped at the right edge. The left side is intentionally empty.
//!
//! Items are laid out from measured text widths so every gap is uniform, and
//! each text rect carries a small right margin so glyphs are never clipped by
//! the rect bounds.

use lgui::prelude::{panel, text, Element, State, UiRect, VisualStyle};
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
    let id = s.workspace.active();
    let m = meta(id);
    let (line, col) = s.workspace.active_buffer().line_col();

    let mut bar = panel(rect, VisualStyle::filled(theme::SIDEBAR));

    let ln = format!("Ln {}, Col {}", line + 1, col + 1);
    let spaces = "Spaces: 2";
    let enc = "UTF-8";
    let badge = m.lang.badge();

    let widths = [measure(&ln), measure(spaces), measure(enc), measure(badge)];
    let total = widths.iter().sum::<f32>() + GAP * 3.0;

    let mut x = rect.right - EDGE_PAD - total;

    let ln_w = widths[0];
    bar = bar.child(text(
        UiRect::new(x, rect.top, x + ln_w + TEXT_MARGIN, rect.bottom),
        ln,
        theme::mono(theme::ZINC_400, theme::SMALL),
    ));
    x += ln_w + GAP;

    let spaces_w = widths[1];
    bar = bar.child(text(
        UiRect::new(x, rect.top, x + spaces_w + TEXT_MARGIN, rect.bottom),
        spaces,
        theme::mono(theme::ZINC_500, theme::SMALL),
    ));
    x += spaces_w + GAP;

    let enc_w = widths[2];
    bar = bar.child(text(
        UiRect::new(x, rect.top, x + enc_w + TEXT_MARGIN, rect.bottom),
        enc,
        theme::mono(theme::ZINC_500, theme::SMALL),
    ));
    x += enc_w + GAP;

    let badge_w = widths[3];
    bar = bar.child(text(
        UiRect::new(x, rect.top, x + badge_w + TEXT_MARGIN, rect.bottom),
        badge,
        theme::mono_bold(m.lang.badge_color(), theme::SMALL),
    ));

    // Hairline top border (matches the title bar's bottom border).
    bar = bar.child(panel(
        UiRect::new(rect.left, rect.top, rect.right, rect.top + 1.0),
        VisualStyle::filled(theme::BORDER),
    ));

    bar
}
