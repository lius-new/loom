//! A snippet being filled in: Tab moves between its tab stops, typing in one
//! updates its mirrors, and leaving it ends the session.
use super::AppState;
use crate::domain::identity::{FileId, PaneId};
use crate::features::language_service::snippet::Snippet;
use crate::model::text::{Edit, EditKind, Selection};
use std::ops::Range;

#[derive(Clone)]
pub struct Stop {
    pub index: u32,
    /// The first is the stop itself; the others mirror it.
    pub ranges: Vec<Range<usize>>,
}

#[derive(Clone)]
pub struct Session {
    pub pane: PaneId,
    pub document: FileId,
    stops: Vec<Stop>,
    /// Which stop is active.
    current: usize,
    /// The text state the ranges describe.
    version: u64,
    identity: usize,
}

impl Session {
    pub fn current_index(&self) -> usize {
        self.current
    }

    pub fn stop_count(&self) -> usize {
        self.stops.len()
    }

    fn contains(&self, offset: usize) -> bool {
        self.stops
            .iter()
            .flat_map(|stop| &stop.ranges)
            .any(|range| range.start <= offset && offset <= range.end)
    }
}

/// Starts filling in `snippet`, inserted at `origin`, with the document at
/// `version`. The first stop is selected.
pub fn begin(app: &mut AppState, pane: PaneId, origin: usize, snippet: &Snippet, version: u64) {
    let Some(id) = app.workspace.pane(pane).and_then(|p| p.active()) else {
        return;
    };
    let stops: Vec<Stop> = snippet
        .stops
        .iter()
        .map(|stop| Stop {
            index: stop.index,
            ranges: stop
                .ranges
                .iter()
                .map(|r| origin + r.start..origin + r.end)
                .collect(),
        })
        .collect();
    let Some(identity) = app
        .workspace
        .documents
        .get(&id)
        .map(|d| d.buffer.snapshot_identity())
    else {
        return;
    };
    // Only the final stop: the caret goes there and there is nothing to visit.
    let single = stops.len() == 1;
    let mut session = Session {
        pane,
        document: id,
        stops,
        current: 0,
        version,
        identity,
    };
    select(app, &session);
    if single {
        return;
    }
    session.version = app
        .workspace
        .documents
        .get(&id)
        .map_or(version, |d| d.buffer.version());
    app.editor.language.snippet = Some(session);
}

fn select(app: &mut AppState, session: &Session) {
    let Some(range) = session
        .stops
        .get(session.current)
        .and_then(|s| s.ranges.first().cloned())
    else {
        return;
    };
    if let Some(mut editor) = app
        .workspace
        .view_editor_mut(session.pane, session.document)
    {
        if range.is_empty() {
            editor.set_cursor(range.start);
        } else {
            editor.select_range(range);
        }
    }
}

/// Moves to the next (or previous) stop. After the last one the session ends and
/// the caret is where the snippet said. Returns false when there is no session.
pub fn step(app: &mut AppState, forward: bool) -> bool {
    let Some(mut session) = app.editor.language.snippet.take() else {
        return false;
    };
    follow(app, &mut session);
    let last = session.stops.len() - 1;
    let target = if forward {
        session.current + 1
    } else {
        session.current.saturating_sub(1)
    };
    session.current = target.min(last);
    select(app, &session);
    // Landing on the final stop ends the session.
    if session.current < last {
        app.editor.language.snippet = Some(session);
    }
    true
}

pub fn end(app: &mut AppState) {
    app.editor.language.snippet = None;
}

/// Brings the ranges up to date with edits made since they were recorded, and
/// copies what was typed into a stop to its mirrors. Ends the session when the
/// caret has left it or the edits cannot be followed.
pub fn observe(app: &mut AppState) {
    let Some(mut session) = app.editor.language.snippet.take() else {
        return;
    };
    let Some(version) = app
        .workspace
        .documents
        .get(&session.document)
        .map(|document| document.buffer.version())
    else {
        return;
    };
    let cursor = app
        .workspace
        .view_editor_mut(session.pane, session.document)
        .map(|editor| editor.cursor());
    if version == session.version {
        // Only the caret moved: leaving every stop ends the session.
        if cursor.is_some_and(|c| session.contains(c)) {
            app.editor.language.snippet = Some(session);
        }
        return;
    }
    if !follow(app, &mut session) {
        return;
    }
    mirror(app, &mut session);
    let cursor = app
        .workspace
        .view_editor_mut(session.pane, session.document)
        .map(|editor| editor.cursor());
    if cursor.is_some_and(|c| session.contains(c)) {
        app.editor.language.snippet = Some(session);
    }
}

/// Maps every range across the revisions since the session last looked.
fn follow(app: &AppState, session: &mut Session) -> bool {
    let Some(document) = app.workspace.documents.get(&session.document) else {
        return false;
    };
    if document.buffer.version() == session.version {
        return true;
    }
    let Some(revisions) = document
        .buffer
        .revisions_since(session.version, session.identity)
    else {
        return false;
    };
    for stop in &mut session.stops {
        for range in &mut stop.ranges {
            let mut start = range.start;
            let mut end = range.end;
            for revision in &revisions {
                start = revision.map(start, false);
                end = revision.map(end, true);
            }
            *range = start..end.max(start);
        }
    }
    session.version = document.buffer.version();
    session.identity = document.buffer.snapshot_identity();
    true
}

/// Makes each mirror show what the stop shows.
fn mirror(app: &mut AppState, session: &mut Session) {
    let Some(text) = app
        .workspace
        .documents
        .get(&session.document)
        .map(|d| d.buffer.text_snapshot())
    else {
        return;
    };
    let mut edits = Vec::new();
    for stop in &session.stops {
        let Some(primary) = stop.ranges.first() else {
            continue;
        };
        if primary.end > text.len() || !text.is_char_boundary(primary.start) {
            continue;
        }
        let shown = &text[primary.clone()];
        for range in stop.ranges.iter().skip(1) {
            if range.end <= text.len() && &text[range.clone()] != shown {
                edits.push(Edit::replace(range.clone(), shown.to_owned()));
            }
        }
    }
    if edits.is_empty() {
        return;
    }
    let Some(mut editor) = app
        .workspace
        .view_editor_mut(session.pane, session.document)
    else {
        return;
    };
    let cursor = editor.cursor();
    // The caret stays in the stop being typed into.
    let applied = editor.edit(EditKind::Typing, edits, |changes, _| {
        Selection::caret(changes.map(cursor, crate::model::text::Assoc::After))
    });
    drop(editor);
    if applied.is_ok() {
        follow(app, session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::language_service::snippet::parse;

    fn app_with(text: &str) -> (AppState, PaneId) {
        let mut app = AppState::new();
        app.workspace.open_path("s.rs".into(), text.into());
        let pane = app.workspace.active_pane();
        (app, pane)
    }

    /// Inserts `snippet` at the cursor, as accepting a completion does.
    fn insert(app: &mut AppState, pane: PaneId, source: &str) {
        let snippet = parse(source, &|_| None);
        let origin = app.workspace.active_editor().unwrap().cursor();
        let mut editor = app.workspace.editor_mut(pane).unwrap();
        editor
            .edit(
                EditKind::Other,
                vec![Edit::insert(origin, snippet.text.clone())],
                |_, _| Selection::caret(origin),
            )
            .unwrap();
        let version = editor.version();
        drop(editor);
        begin(app, pane, origin, &snippet, version);
    }

    fn type_text(app: &mut AppState, text: &str) {
        let mut editor = app.workspace.active_editor_mut().unwrap();
        loom_editor_core::normal::insert(&mut editor, text);
        drop(editor);
        observe(app);
    }

    fn text(app: &AppState) -> String {
        app.workspace.active_editor().unwrap().text().to_owned()
    }

    #[test]
    fn the_first_stop_is_selected_and_tab_visits_the_rest_in_order() {
        let (mut app, pane) = app_with("");
        insert(&mut app, pane, "fn ${1:name}(${2:args}) {\n\t$0\n}");
        assert_eq!(text(&app), "fn name(args) {\n\t\n}");
        let editor = app.workspace.active_editor().unwrap();
        assert_eq!(editor.selection(), Some(3..7));
        assert!(step(&mut app, true));
        assert_eq!(
            app.workspace.active_editor().unwrap().selection(),
            Some(8..12)
        );
        assert!(step(&mut app, true));
        assert_eq!(app.workspace.active_editor().unwrap().cursor(), 17);
        assert!(
            app.editor.language.snippet.is_none(),
            "the last stop ends it"
        );
    }

    #[test]
    fn shift_tab_goes_back_and_stops_at_the_first() {
        let (mut app, pane) = app_with("");
        insert(&mut app, pane, "${1:a} ${2:b} $0");
        step(&mut app, true);
        assert_eq!(
            app.workspace.active_editor().unwrap().selection(),
            Some(2..3)
        );
        step(&mut app, false);
        assert_eq!(
            app.workspace.active_editor().unwrap().selection(),
            Some(0..1)
        );
        step(&mut app, false);
        assert_eq!(
            app.workspace.active_editor().unwrap().selection(),
            Some(0..1)
        );
    }

    #[test]
    fn typing_into_a_stop_updates_its_mirrors() {
        let (mut app, pane) = app_with("");
        insert(&mut app, pane, "let ${1:x} = ${2:0}; // $1");
        assert_eq!(text(&app), "let x = 0; // x");
        type_text(&mut app, "count");
        assert_eq!(text(&app), "let count = 0; // count");
        type_text(&mut app, "er");
        assert_eq!(text(&app), "let counter = 0; // counter");
        assert!(app.editor.language.snippet.is_some());
    }

    #[test]
    fn leaving_the_snippet_ends_the_session() {
        let (mut app, pane) = app_with("");
        insert(&mut app, pane, "a ${1:b} ${2:c}");
        app.workspace.active_editor_mut().unwrap().set_cursor(0);
        observe(&mut app);
        assert!(app.editor.language.snippet.is_none());
    }

    #[test]
    fn a_snippet_with_only_the_final_stop_leaves_nothing_to_visit() {
        let (mut app, pane) = app_with("");
        insert(&mut app, pane, "println!($0)");
        assert!(app.editor.language.snippet.is_none());
        assert_eq!(app.workspace.active_editor().unwrap().cursor(), 9);
    }
}
