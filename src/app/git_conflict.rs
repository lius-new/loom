//! Moving between the conflicts Git reports in the open file.

use lgui::prelude::{State, UiRect};

use super::language_ui::target;
use crate::app::context::AppContext;
use crate::features::git::GitStoreSnapshot;
use crate::features::git::conflict::ConflictRegion;

/// The conflict that starts after (or, going backwards, before) one-based
/// `line`, wrapping around.
pub(crate) fn next_region(
    regions: &[ConflictRegion],
    line: usize,
    forward: bool,
) -> Option<&ConflictRegion> {
    if forward {
        regions
            .iter()
            .find(|region| region.start > line)
            .or_else(|| regions.first())
    } else {
        regions
            .iter()
            .rev()
            .find(|region| region.start < line)
            .or_else(|| regions.last())
    }
}

/// Moves the cursor to the next or previous conflict of the open file.
pub(crate) fn go_to(
    state: &AppContext,
    store: &State<GitStoreSnapshot>,
    forward: bool,
    rect: UiRect,
) -> bool {
    let app = state.get();
    let Some(target) = target(&app, app.workspace.active_pane()) else {
        return false;
    };
    let regions = store.get().conflicts.get(&target.path).cloned();
    let Some(regions) = regions.filter(|regions| !regions.is_empty()) else {
        state.update(|app| app.show_toast("No conflicts in this file"));
        return true;
    };
    let Some(document) = app.workspace.documents.get(&target.document) else {
        return false;
    };
    let line = document.buffer.line_of(target.cursor) + 1;
    let Some(region) = next_region(&regions, line, forward) else {
        return false;
    };
    let offset = document.buffer.line_start(region.start - 1);
    state.update(|app| {
        if let Some(mut editor) = app.workspace.active_editor_mut() {
            editor.set_cursor(offset);
        }
        crate::app::editor::reveal_cursor(app, rect);
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(start: usize) -> ConflictRegion {
        ConflictRegion {
            start,
            current: start + 1..start + 2,
            separator: start + 2,
            incoming: start + 3..start + 4,
            end: start + 4,
        }
    }

    #[test]
    fn the_next_conflict_is_after_the_cursor_and_wraps() {
        let regions = [region(3), region(20)];
        assert_eq!(next_region(&regions, 1, true).map(|r| r.start), Some(3));
        assert_eq!(next_region(&regions, 3, true).map(|r| r.start), Some(20));
        assert_eq!(next_region(&regions, 20, true).map(|r| r.start), Some(3));
    }

    #[test]
    fn the_previous_conflict_is_before_the_cursor_and_wraps() {
        let regions = [region(3), region(20)];
        assert_eq!(next_region(&regions, 25, false).map(|r| r.start), Some(20));
        assert_eq!(next_region(&regions, 20, false).map(|r| r.start), Some(3));
        assert_eq!(next_region(&regions, 3, false).map(|r| r.start), Some(20));
    }
}
