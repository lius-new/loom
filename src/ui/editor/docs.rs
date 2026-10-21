//! Documentation shown over the code: one set of rules for every card that
//! carries Markdown (hover and diagnostics, the completion list's side panel,
//! signature help), so they all wrap, scroll, select and open links alike.
//!
//! Prose wraps at the card's width; a code block wider than the card scrolls
//! sideways inside its box; text can be selected across the card and copied;
//! a link opens in the browser when clicked. Each card says only where it goes
//! and what the host does with a link or a selection.
use std::sync::Arc;

use crate::features::markdown::Block;
use crate::ui::markdown::{self, Interaction, Layout, Metrics};
use crate::ui::theme;
use lgui::prelude::{Element, UiRect};

/// Space between a card's edge and its text.
pub(crate) const PAD: f32 = 8.0;
/// Height of a line of documentation.
pub(crate) const LINE_H: f32 = 18.0;
const BLOCK_GAP: f32 = 6.0;
const INDENT: f32 = 16.0;

/// What the host does when the person acts on documentation.
#[derive(Clone)]
pub(crate) struct Actions {
    /// A link was clicked, with its URL.
    pub open_link: Arc<dyn Fn(String) + Send + Sync>,
    /// The selection changed: the selected text, or `None` once cleared.
    pub select: Arc<dyn Fn(Option<String>) + Send + Sync>,
}

impl Actions {
    /// Actions that do nothing, for drawing a card without a host.
    #[cfg(test)]
    pub fn ignored() -> Self {
        Self {
            open_link: Arc::new(|_| {}),
            select: Arc::new(|_| {}),
        }
    }

    pub fn interaction(&self) -> Interaction {
        let (open, select) = (self.open_link.clone(), self.select.clone());
        Interaction {
            on_link: Some(Arc::new(move |url| open(url))),
            on_select: Some(Arc::new(move |_, text| select(text))),
        }
    }
}

/// Lays `blocks` out within `width`.
pub(crate) fn layout(blocks: &[Block], width: f32) -> Layout {
    let size = theme::UI_SIZE;
    markdown::layout(
        blocks,
        width.max(40.0),
        &Metrics {
            measure: &|text, style| markdown::measure(text, style, size),
            line_h: LINE_H,
            gap: BLOCK_GAP,
            indent: INDENT,
        },
    )
}

/// Lays `blocks` out as narrow as they fit, between `min` and `max` wide (text
/// padding excluded from both), and returns the layout and the width it took.
pub(crate) fn fit(blocks: &[Block], min: f32, max: f32) -> (Layout, f32) {
    let natural = layout(blocks, max);
    let width = natural.width.clamp(min.min(max), max);
    if width >= max {
        (natural, max)
    } else {
        (layout(blocks, width), width)
    }
}

/// Draws a layout with its top-left corner at `(left, top)`.
pub(crate) fn render(
    layout: &Layout,
    left: f32,
    top: f32,
    interaction: Option<&Interaction>,
) -> Element {
    markdown::render(layout, left, top, theme::UI_SIZE, LINE_H, interaction)
}

/// The layers of a layout, for a card that puts its own text beside the docs.
pub(crate) fn parts(
    layout: &Layout,
    left: f32,
    top: f32,
    on_link: Option<&markdown::LinkHandler>,
) -> markdown::Parts {
    markdown::parts(layout, left, top, theme::UI_SIZE, LINE_H, on_link)
}

/// The rectangle a card's text takes inside `card`.
pub(crate) fn inset(card: UiRect) -> UiRect {
    UiRect::new(
        card.left + PAD,
        card.top + PAD,
        card.right - PAD,
        card.bottom - PAD,
    )
}
