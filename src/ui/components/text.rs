//! Measured single-line text used by editable and read-only controls.

use std::ops::Range;

use lgui::core::TextStyle;
use lgui::prelude::{Element, UiRect, text};
use lgui::text::{self, TextLayout, TextLayoutRequest};
use unicode_width::UnicodeWidthStr;

/// A single-line text run whose caret and hit-testing geometry comes from the
/// active renderer rather than a fixed character-width approximation.
#[derive(Clone)]
pub struct SingleLineText {
    value: String,
    bounds: UiRect,
    style: TextStyle,
    layout: Option<TextLayout>,
}

impl SingleLineText {
    pub fn new(value: impl Into<String>, bounds: UiRect, style: TextStyle) -> Self {
        let value = value.into();
        let mut request = TextLayoutRequest::single_line(
            &value,
            bounds,
            style.height,
            style.weight,
        );
        request.font_slant = style.font_slant;
        request.font_families = style.font_families;
        request.tracking = style.tracking;
        request.align = style.align;
        request.vertical_align = style.vertical_align;
        let layout = text::layout(&request);
        Self {
            value,
            bounds,
            style,
            layout,
        }
    }

    pub fn width(&self) -> f32 {
        // Skia uses a negative sentinel for an empty paragraph's longest line.
        // Treat invalid backend metrics as unavailable so empty IME prefixes
        // contribute zero width instead of pulling the caret to the left edge.
        self.layout
            .as_ref()
            .map(|layout| layout.width)
            .filter(|width| width.is_finite() && *width >= 0.0)
            .unwrap_or_else(|| {
                UnicodeWidthStr::width(self.value.as_str()) as f32 * self.style.height * 0.6
            })
    }

    pub fn caret_x(&self, byte_offset: usize) -> f32 {
        let char_index = self.value[..self.byte_boundary(byte_offset)].chars().count();
        self.layout
            .as_ref()
            .and_then(|layout| layout.caret_rect(char_index))
            .map_or_else(
                || {
                    self.bounds.left
                        + UnicodeWidthStr::width(
                            &self.value[..self.byte_boundary(byte_offset)],
                        ) as f32
                            * self.style.height
                            * 0.6
                },
                |rect| rect.left,
            )
    }

    pub fn caret_offset(&self, byte_offset: usize) -> f32 {
        self.caret_x(byte_offset) - self.bounds.left
    }

    pub fn hit_byte_offset(&self, x: f32, y: f32) -> usize {
        let char_index = self.layout.as_ref().map_or_else(
            || {
                let local = ((x - self.bounds.left) / (self.style.height * 0.6))
                    .round()
                    .max(0.0) as usize;
                local.min(self.value.chars().count())
            },
            |layout| layout.hit_test(x, y).index.min(self.value.chars().count()),
        );
        char_to_byte(&self.value, char_index)
    }

    pub fn selection_rects(&self, range: Range<usize>) -> Vec<UiRect> {
        let start = self.value[..self.byte_boundary(range.start)].chars().count();
        let end = self.value[..self.byte_boundary(range.end)].chars().count();
        self.layout
            .as_ref()
            .map_or_else(Vec::new, |layout| layout.selection_rects(start..end))
    }

    pub fn element(&self) -> Element {
        text(self.bounds, self.value.clone(), self.style).into()
    }

    fn byte_boundary(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.value.len());
        while !self.value.is_char_boundary(offset) {
            offset = offset.saturating_sub(1);
        }
        offset
    }
}

fn char_to_byte(value: &str, char_index: usize) -> usize {
    value
        .char_indices()
        .nth(char_index)
        .map_or(value.len(), |(byte, _)| byte)
}
