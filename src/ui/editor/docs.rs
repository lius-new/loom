//! Compact memory-document host of the shared async Markdown reader.
use crate::ui::markdown::card::{self, Document, Interaction, Layout};
use lgui::prelude::{Element, UiRect};
use std::sync::Arc;
pub(crate) const PAD: f32 = 12.0;
#[derive(Clone)]
pub(crate) struct Actions {
    pub open_link: Arc<dyn Fn(String) + Send + Sync>,
    pub select: Arc<dyn Fn(Option<String>) + Send + Sync>,
}
impl Actions {
    #[cfg(test)]
    pub fn ignored() -> Self {
        Self {
            open_link: Arc::new(|_| {}),
            select: Arc::new(|_| {}),
        }
    }
    pub fn interaction(&self) -> Interaction {
        Interaction {
            on_link: Some(self.open_link.clone()),
            on_select: Some(self.select.clone()),
        }
    }
}
pub(crate) fn layout(document: &Document, width: f32) -> Layout {
    card::layout(document, width.max(40.0))
}
pub(crate) fn fit(document: &Document, min: f32, max: f32) -> (Layout, f32) {
    let layout = layout(document, max);
    let width = layout.width.clamp(min.min(max), max);
    (layout, width)
}
pub(crate) fn render(
    layout: &Layout,
    left: f32,
    top: f32,
    interaction: Option<&Interaction>,
    height: f32,
) -> Element {
    card::render(layout, left, top, height, interaction)
}
pub(crate) fn inset(card: UiRect) -> UiRect {
    UiRect::new(
        card.left + PAD,
        card.top + PAD,
        card.right - PAD,
        card.bottom - PAD,
    )
}
