use crate::features::git::diff_selection::DiffSide;
use crate::features::git::diff_selection_tests::overview;
use crate::ui::diff_editor::{code_left, row_h};
use crate::ui::diff_selection::{Geometry, point_at};
use crate::ui::editor::view::display_columns;
use crate::ui::{fonts, theme};
use lgui::prelude::UiRect;

#[test]
fn pointer_positions_resolve_to_files_rows_and_characters() {
    let document = overview();
    let body = UiRect::new(0.0, 34.0, 800.0, 600.0);
    let geometry = Geometry {
        body,
        split: false,
        ratio: 0.5,
        scroll_x: 0.0,
        scroll_y: 0.0,
    };
    let layout = document.changes_layout(false, &Default::default());
    let text_left = code_left(body, false) + theme::CODE_PAD;
    let row_y = |row: usize| body.top + (row as f32 + 0.5) * row_h();
    let first_body = layout.sections[0].body_top;
    let (side, point, inside) = point_at(
        &document,
        Some(&layout),
        geometry,
        None,
        (
            text_left + 3.2 * fonts::editor().char_w,
            row_y(first_body + 1),
        ),
    )
    .unwrap();
    assert_eq!((side, inside), (DiffSide::New, true));
    assert_eq!((point.section, point.row, point.column), (0, 1, 3));
    // A header row pulls in to the start of its file's body.
    let (_, header, inside) = point_at(
        &document,
        Some(&layout),
        geometry,
        None,
        (text_left, row_y(0)),
    )
    .unwrap();
    assert!(!inside && (header.section, header.row, header.column) == (0, 0, 0));
    // Below everything pulls in to the end of the last row.
    let (_, tail, inside) =
        point_at(&document, Some(&layout), geometry, None, (text_left, 590.0)).unwrap();
    assert!(!inside && (tail.section, tail.row, tail.column) == (1, 1, 6));
}

#[test]
fn tabs_and_wide_characters_advance_the_highlight() {
    assert_eq!(display_columns("\tab", 1), 4);
    assert_eq!(display_columns("a\tb", 2), 4);
    assert_eq!(display_columns("你a", 1), 2);
}
