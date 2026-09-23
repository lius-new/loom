//! The line-number gutter with clickable breakpoint dots.

use std::ops::Range;

use lgui::core::ellipse;
use lgui::prelude::{group, text, Color, Element, UiRect, VisualStyle};

use crate::theme;

/// Render only `visible_lines` while preserving their absolute document rows.
/// `breakpoint_line` is 1-based
/// and draws a rose dot next to that number.
pub fn render(
    rect: UiRect,
    visible_lines: Range<usize>,
    breakpoint_line: Option<usize>,
) -> Element {
    let mut g = group(rect);

    for i in visible_lines {
        let y = rect.top + i as f32 * theme::LINE_H;
        let has_bp = breakpoint_line == Some(i + 1);

        let color: Color = if has_bp { theme::ROSE_500 } else { theme::ZINC_600 };
        let num_rect = UiRect::new(rect.left, y, rect.right - 10.0, y + theme::LINE_H);
        g = g.child(text(num_rect, (i + 1).to_string(), theme::mono_right(color, theme::CODE_SIZE)));

        if has_bp {
            let dot = UiRect::new(rect.right - 11.0, y + 8.0, rect.right - 5.0, y + 14.0);
            g = g.child(ellipse(dot, VisualStyle::filled(theme::ROSE_500)));
        }
    }

    g
}
