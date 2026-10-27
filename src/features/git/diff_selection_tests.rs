use super::diff_document::DiffDocument;
use super::diff_selection::*;
use crate::domain::identity::PaneId;
use crate::features::git::{
    ChangeDiff, ChangeGroup, DiffContent, DiffRequest, DiffTarget, diff::parse_unified,
};
use std::collections::BTreeSet;

/// Two files: `a.rs` (hunk, context, deletion, addition, context) and an
/// untracked `b.txt` with two lines.
pub(crate) fn overview() -> DiffDocument {
    let patch = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,3 +1,3 @@\n fn main() {\n-    old();\n+    new_call(1);\n }\n";
    DiffDocument::from_content(
        DiffRequest {
            repository_root: "repo".into(),
            path: Default::default(),
            target: DiffTarget::Changes,
        },
        DiffContent::Changes(vec![
            ChangeDiff {
                path: "a.rs".into(),
                group: ChangeGroup::Unstaged,
                target: DiffTarget::IndexToWorktree,
                content: Ok(DiffContent::Unified(parse_unified(patch).unwrap())),
            },
            ChangeDiff {
                path: "b.txt".into(),
                group: ChangeGroup::Untracked,
                target: DiffTarget::IndexToWorktree,
                content: Ok(DiffContent::Untracked("first\nsecond\n".into())),
            },
        ]),
    )
}

fn pane() -> PaneId {
    PaneId::new(1)
}

fn at(section: usize, row: usize, column: usize) -> DiffPoint {
    DiffPoint {
        section,
        row,
        column,
    }
}

fn select(anchor: DiffPoint, head: DiffPoint) -> DiffSelection {
    DiffSelection {
        pane: pane(),
        side: DiffSide::New,
        anchor,
        head,
        unit: SelectionUnit::Character,
        origin: (anchor, anchor),
        dragging: false,
    }
}

#[test]
fn a_selection_across_files_copies_code_without_markers_or_hunk_headers() {
    let document = overview();
    let none = BTreeSet::new();
    // Inline rows of a.rs: 0 context, 1 deletion, 2 addition, 3 context.
    let selection = select(at(0, 0, 3), at(1, 0, 3));
    assert_eq!(
        selection.text(&document, false, &none).unwrap(),
        "main() {\n    old();\n    new_call(1);\n}\nfir"
    );
    let reversed = select(at(1, 0, 3), at(0, 0, 3));
    assert_eq!(
        reversed.text(&document, false, &none),
        selection.text(&document, false, &none)
    );
    assert_eq!(
        select(at(0, 1, 2), at(0, 1, 2)).text(&document, false, &none),
        None
    );
}

#[test]
fn collapsed_files_are_left_out_of_a_copy() {
    let document = overview();
    let hidden = BTreeSet::from([document.sections.as_ref().unwrap()[0].request()]);
    let selection = select(at(0, 1, 0), at(1, 1, 6));
    assert_eq!(
        selection.text(&document, false, &hidden).unwrap(),
        "first\nsecond"
    );
}

#[test]
fn split_selection_stays_on_the_side_it_started_on() {
    let document = overview();
    let none = BTreeSet::new();
    let on = |side| DiffSelection {
        side,
        ..select(at(0, 0, 0), at(0, 4, 99))
    };
    let new = on(DiffSide::New).text(&document, true, &none).unwrap();
    let old = on(DiffSide::Old).text(&document, true, &none).unwrap();
    assert!(new.contains("new_call(1)") && !new.contains("old()"));
    assert!(old.contains("old()") && !old.contains("new_call"));
}

#[test]
fn word_and_line_presses_select_whole_units_and_grow_by_them() {
    let document = overview();
    let none = BTreeSet::new();
    let point = at(0, 2, 6); // inside `new_call`
    let press =
        |unit| DiffSelection::start(&document, false, pane(), DiffSide::New, point, unit, None);
    let word = press(SelectionUnit::Word);
    assert_eq!(word.text(&document, false, &none).unwrap(), "new_call");
    let mut grown = word;
    grown.drag_to(&document, false, at(1, 0, 1));
    assert_eq!(
        grown.text(&document, false, &none).unwrap(),
        "new_call(1);\n}\nfirst"
    );
    assert_eq!(
        press(SelectionUnit::Line)
            .text(&document, false, &none)
            .unwrap(),
        "    new_call(1);"
    );
}

#[test]
fn shift_press_keeps_the_anchor() {
    let document = overview();
    let first = select(at(0, 1, 2), at(0, 1, 2));
    let extended = DiffSelection::start(
        &document,
        false,
        pane(),
        DiffSide::New,
        at(0, 2, 4),
        SelectionUnit::Character,
        Some(&first),
    );
    assert_eq!((extended.anchor, extended.head), (at(0, 1, 2), at(0, 2, 4)));
}

#[test]
fn select_all_covers_every_expanded_file() {
    let document = overview();
    let none = BTreeSet::new();
    let all = DiffSelection::all(&document, false, pane(), &none).unwrap();
    assert_eq!(
        all.text(&document, false, &none).unwrap(),
        "fn main() {\n    old();\n    new_call(1);\n}\nfirst\nsecond"
    );
}
