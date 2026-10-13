//! Convert UI rectangles at the application boundary.
use crate::domain::geometry::Rect;
use lgui::prelude::UiRect;
impl From<UiRect> for Rect {
    fn from(value: UiRect) -> Self {
        Self::new(value.left, value.top, value.right, value.bottom)
    }
}
impl From<Rect> for UiRect {
    fn from(value: Rect) -> Self {
        Self::new(value.left, value.top, value.right, value.bottom)
    }
}
