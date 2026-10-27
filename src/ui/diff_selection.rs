//! Pointer hit-testing and highlight for text selected in a diff view.
use lgui::core::clip;
use lgui::prelude::{Element, UiRect, VisualStyle, panel};

use crate::features::git::diff_document::{ChangesLayout, DiffDocument};
use crate::features::git::diff_selection::{
    DiffPoint, DiffSelection, DiffSide, row_count, row_text,
};
use crate::ui::diff_editor::{NUMBER_W, code_left, row_h, split_middle};
use crate::ui::editor::view::{display_columns, fallback_hit_column};
use crate::ui::{fonts, theme};

/// Where the diff body sits and how far it is scrolled.
#[derive(Clone, Copy)]
pub struct Geometry {
    pub body: UiRect,
    pub split: bool,
    pub ratio: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
}

/// Which way a position outside the rows was pulled in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Clamp {
    Inside,
    Before,
    After,
}

impl Geometry {
    fn side_at(&self, x: f32) -> DiffSide {
        if self.split && x < split_middle(self.body, self.ratio) {
            DiffSide::Old
        } else {
            DiffSide::New
        }
    }

    /// Left edge of a column's code area, before scrolling.
    fn code_area(&self, side: DiffSide) -> (f32, f32) {
        if !self.split {
            return (code_left(self.body, false), self.body.right);
        }
        let middle = split_middle(self.body, self.ratio);
        match side {
            DiffSide::Old => (self.body.left + NUMBER_W, middle),
            DiffSide::New => (middle + NUMBER_W, self.body.right),
        }
    }

    /// Where the first character of a row is drawn.
    fn text_origin(&self, side: DiffSide) -> f32 {
        self.code_area(side).0 + theme::CODE_PAD - self.scroll_x
    }
}

fn locate(layout: &ChangesLayout, row: usize) -> Option<(usize, usize, Clamp)> {
    let mut last = None;
    for item in layout.sections.iter().filter(|item| item.body_rows > 0) {
        if row < item.body_top {
            return Some((item.index, 0, Clamp::Before));
        }
        if row < item.body_top + item.body_rows {
            return Some((item.index, row - item.body_top, Clamp::Inside));
        }
        last = Some((item.index, item.body_rows - 1, Clamp::After));
    }
    last
}

/// The character under a pointer. A drag passes the side its press began on;
/// a press picks the side under the pointer. Also reports whether the pointer
/// really was over a code row.
pub fn point_at(
    document: &DiffDocument,
    layout: Option<&ChangesLayout>,
    geometry: Geometry,
    side: Option<DiffSide>,
    (x, y): (f32, f32),
) -> Option<(DiffSide, DiffPoint, bool)> {
    let row = ((y - geometry.body.top + geometry.scroll_y) / row_h())
        .floor()
        .max(0.0) as usize;
    let (section, row, clamp) = match layout {
        Some(layout) => locate(layout, row)?,
        None => {
            let count = row_count(document, geometry.split, 0);
            if count == 0 {
                return None;
            }
            let clamp = if row >= count {
                Clamp::After
            } else {
                Clamp::Inside
            };
            (0, row.min(count - 1), clamp)
        }
    };
    let side = if geometry.split {
        side.unwrap_or_else(|| geometry.side_at(x))
    } else {
        DiffSide::New
    };
    let text = row_text(document, geometry.split, section, row, side);
    let column = match (clamp, text) {
        (_, None) | (Clamp::Before, _) => 0,
        (Clamp::After, Some(text)) => text.chars().count(),
        (Clamp::Inside, Some(text)) => fallback_hit_column(text, x - geometry.text_origin(side)),
    };
    let point = DiffPoint {
        section,
        row,
        column,
    };
    Some((side, point, clamp == Clamp::Inside))
}

/// Highlight for the selected characters of every visible row.
pub fn layer(
    document: &DiffDocument,
    layout: Option<&ChangesLayout>,
    geometry: Geometry,
    selection: &DiffSelection,
) -> Element {
    let height = row_h();
    let body = geometry.body;
    let first = (geometry.scroll_y / height).floor() as usize;
    let last = ((geometry.scroll_y + body.height()) / height).ceil() as usize;
    let (area_left, area_right) = geometry.code_area(selection.side);
    let origin = geometry.text_origin(selection.side);
    let mut rows = Vec::new();
    match layout {
        Some(layout) => {
            for item in &layout.sections {
                let from = item.body_top.max(first);
                let to = (item.body_top + item.body_rows).min(last + 1);
                rows.extend((from..to).map(|row| (item.index, row - item.body_top, row)));
            }
        }
        None => {
            let to = (last + 1).min(row_count(document, geometry.split, 0));
            rows.extend((first..to).map(|row| (0, row, row)));
        }
    }
    let mut out = clip(body, 0.0, 0.0);
    for (section, local, row) in rows {
        let Some(range) = selection.span(document, geometry.split, section, local) else {
            continue;
        };
        let Some(text) = row_text(document, geometry.split, section, local, selection.side) else {
            continue;
        };
        let top = body.top + row as f32 * height - geometry.scroll_y;
        let char_w = fonts::editor().char_w;
        let left = (origin + display_columns(text, range.start) as f32 * char_w).max(area_left);
        let right = (origin + display_columns(text, range.end) as f32 * char_w).min(area_right);
        if right > left {
            out = out.child(panel(
                UiRect::new(left, top, right, top + height),
                VisualStyle::filled(theme::c().accent).alpha(55),
            ));
        }
    }
    out
}
