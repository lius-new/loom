//! The line-number gutter with clickable breakpoint dots.

use std::collections::BTreeMap;
use std::ops::Range;

use lgui::core::ellipse;
use lgui::prelude::{Color, Element, UiRect, VisualStyle, group, text};

use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GutterDecoration {
    Breakpoint,
    GitAdded,
    GitModified,
    GitDeleted,
    Diagnostic,
}

/// Render only `visible_lines` while preserving their absolute document rows.
pub fn render(
    rect: UiRect,
    visible_lines: Range<usize>,
    decorations: &BTreeMap<usize, Vec<GutterDecoration>>,
) -> Element {
    let mut g = group(rect);

    for i in visible_lines {
        let y = rect.top + i as f32 * theme::LINE_H;
        let line_decorations = decorations.get(&(i + 1));
        let has_bp =
            line_decorations.is_some_and(|values| values.contains(&GutterDecoration::Breakpoint));

        let color: Color = if has_bp {
            theme::ROSE_500
        } else {
            theme::ZINC_600
        };
        let num_rect = UiRect::new(rect.left, y, rect.right - 10.0, y + theme::LINE_H);
        g = g.child(text(
            num_rect,
            (i + 1).to_string(),
            theme::mono_right(color, theme::CODE_SIZE),
        ));

        if has_bp {
            let dot = UiRect::new(rect.right - 11.0, y + 8.0, rect.right - 5.0, y + 14.0);
            g = g.child(ellipse(dot, VisualStyle::filled(theme::ROSE_500)));
        }
        if let Some(change) = line_decorations.and_then(|values| {
            values.iter().find_map(|value| match value {
                GutterDecoration::GitAdded => Some(theme::DIFF_ADD_BORDER),
                GutterDecoration::GitModified => Some(theme::BLUE_400),
                GutterDecoration::GitDeleted => Some(theme::DIFF_DEL_BORDER),
                _ => None,
            })
        }) {
            g = g.child(lgui::prelude::panel(
                UiRect::new(
                    rect.right - 3.0,
                    y + 2.0,
                    rect.right,
                    y + theme::LINE_H - 2.0,
                ),
                VisualStyle::filled(change),
            ));
        }
    }

    g
}
