//! Transient pointer and IME state; selections and history live in each buffer.
use crate::model::document::FileId;
use crate::model::pane_layout::PaneId;
use lgui::core::KeyModifiers;
use std::{
    ops::Range,
    path::{Path, PathBuf},
    time::Instant,
};

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
    /// A press can place the caret smoothly; only a real drag must follow immediately.
    pub caret_dragging: bool,
    pub last_click: Option<(Instant, FileId, f32, f32, u8)>,
    pub last_tree_click: Option<(Instant, PathBuf, f32, f32, u8)>,
    pub last_tab_click: Option<(Instant, FileId, f32, f32, u8)>,
    pub last_terminal_tab_click: Option<(Instant, u64, f32, f32, u8)>,
    pub menu: Option<(f32, f32)>,
    pub menu_hover: Option<usize>,
    /// The input method composition in progress, and the view it belongs to.
    pub ime: Option<ImeSession>,
}

/// An input method composition. It belongs to the view where it started, so a
/// commit that arrives after the focus moved still lands in that view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImeSession {
    pub pane: PaneId,
    pub document: FileId,
    /// Composition text; it is not part of the document until committed.
    pub text: String,
    pub cursor: Option<Range<usize>>,
}

impl EditorInteraction {
    /// The composition text to draw in `pane` showing `document`, if any.
    pub fn preedit_for(&self, pane: PaneId, document: FileId) -> Option<&ImeSession> {
        self.ime.as_ref().filter(|session| {
            session.pane == pane && session.document == document && !session.text.is_empty()
        })
    }

    /// Whether a composition is showing text, which owns every key.
    pub fn is_composing(&self) -> bool {
        self.ime
            .as_ref()
            .is_some_and(|session| !session.text.is_empty())
    }
}
impl EditorInteraction {
    pub fn click_count(&mut self, document: FileId, x: f32, y: f32) -> u8 {
        next_click_count(&mut self.last_click, document, x, y)
    }

    pub fn tree_click_count(&mut self, path: &Path, x: f32, y: f32) -> u8 {
        next_click_count(&mut self.last_tree_click, path.to_path_buf(), x, y)
    }

    pub fn tab_click_count(&mut self, document: FileId, x: f32, y: f32) -> u8 {
        next_click_count(&mut self.last_tab_click, document, x, y)
    }

    pub fn terminal_tab_click_count(&mut self, terminal: u64, x: f32, y: f32) -> u8 {
        next_click_count(&mut self.last_terminal_tab_click, terminal, x, y)
    }
}

fn next_click_count<K: PartialEq>(
    previous: &mut Option<(Instant, K, f32, f32, u8)>,
    key: K,
    x: f32,
    y: f32,
) -> u8 {
    let now = Instant::now();
    let count = previous
        .as_ref()
        .filter(|(time, previous_key, previous_x, previous_y, _)| {
            previous_key == &key
                && now.duration_since(*time).as_millis() < 500
                && (x - *previous_x).abs() < 5.0
                && (y - *previous_y).abs() < 5.0
        })
        .map_or(1, |(_, _, _, _, count)| count % 3 + 1);
    *previous = Some((now, key, x, y, count));
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_tree_and_tab_click_sequences_are_independent() {
        let mut interaction = EditorInteraction::default();
        let document = FileId::new(1);
        let path = Path::new("src/main.rs");

        assert_eq!(interaction.tree_click_count(path, 10.0, 20.0), 1);
        assert_eq!(interaction.tree_click_count(path, 10.0, 20.0), 2);
        assert_eq!(interaction.tab_click_count(document, 10.0, 20.0), 1);
        assert_eq!(interaction.click_count(document, 10.0, 20.0), 1);
        assert_eq!(interaction.tab_click_count(document, 10.0, 20.0), 2);
    }

    #[test]
    fn changing_the_click_target_starts_a_new_sequence() {
        let mut interaction = EditorInteraction::default();

        assert_eq!(
            interaction.tree_click_count(Path::new("first.rs"), 10.0, 20.0),
            1
        );
        assert_eq!(
            interaction.tree_click_count(Path::new("second.rs"), 10.0, 20.0),
            1
        );
    }
}
