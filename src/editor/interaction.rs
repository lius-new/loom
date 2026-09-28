//! Transient pointer and IME state; selections and history live in each buffer.
use crate::model::document::FileId;
use lgui::core::KeyModifiers;
use std::{ops::Range, time::Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionUnit {
    Character,
    Word,
    Line,
}
#[derive(Clone)]
pub struct DragSelection {
    pub document: FileId,
    pub origin: Range<usize>,
    pub unit: SelectionUnit,
    pub point: (f32, f32),
}
#[derive(Clone, Default)]
pub struct EditorInteraction {
    pub modifiers: KeyModifiers,
    pub drag: Option<DragSelection>,
    pub last_click: Option<(Instant, FileId, f32, f32, u8)>,
    pub menu: Option<(f32, f32)>,
    pub menu_hover: Option<usize>,
    pub preedit: String,
    pub ime_pending: bool,
    pub preedit_cursor: Option<Range<usize>>,
}
impl EditorInteraction {
    pub fn click_count(&mut self, document: FileId, x: f32, y: f32) -> u8 {
        let now = Instant::now();
        let count = self
            .last_click
            .filter(|(t, id, px, py, _)| {
                *id == document
                    && now.duration_since(*t).as_millis() < 500
                    && (x - px).abs() < 5.0
                    && (y - py).abs() < 5.0
            })
            .map_or(1, |(_, _, _, _, n)| n % 3 + 1);
        self.last_click = Some((now, document, x, y, count));
        count
    }
}
