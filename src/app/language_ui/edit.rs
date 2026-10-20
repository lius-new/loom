//! Applying what a server asks to change: a `WorkspaceEdit` as one operation.
//!
//! Every text edit is checked against its document before any is made, so an
//! edit either happens completely or not at all. The edits of all documents are
//! one undo step each, tagged with a shared transaction; undoing from any of
//! them undoes them together, or says why it cannot.
use super::{AppContext, AppState};
use crate::domain::document::DocumentStamp;
use crate::domain::identity::FileId;
use crate::features::language_service::reply::{Change, DocumentEdit, WorkspaceEdit};
use crate::features::language_service::uri;
use crate::model::text::{Assoc, Edit, EditKind, Selection, TextBuffer};
use lgui::prelude::UiRect;
use std::collections::HashMap;
use std::path::PathBuf;

/// Files larger than this are not opened to be edited by a server.
const MAX_FILE: u64 = 16 * 1024 * 1024;

/// A workspace edit and the state of the open documents it was computed for.
#[derive(Clone)]
pub struct Request {
    pub edit: WorkspaceEdit,
    /// The documents that were open when the question was asked, with their state.
    pub stamps: Vec<(PathBuf, DocumentStamp)>,
    pub rect: UiRect,
    /// What the edit was for, for messages ("Rename", "Quick fix").
    pub title: String,
    /// A server waiting to hear whether the edit was made.
    pub reply: Option<crate::infrastructure::lsp::manager::Reply>,
}

/// The stamps of every open text document now.
pub fn stamps(app: &AppState) -> Vec<(PathBuf, DocumentStamp)> {
    app.workspace
        .documents
        .iter()
        .filter(|(_, d)| d.is_text())
        .map(|(id, d)| (d.meta.path.clone(), d.stamp(*id)))
        .collect()
}

/// One document's edits as bytes of its text.
struct Planned {
    path: PathBuf,
    edits: Vec<Edit>,
}

#[derive(Default)]
struct Plan {
    steps: Vec<Step>,
}

enum Step {
    Text(Planned),
    Create {
        path: PathBuf,
        overwrite: bool,
        ignore_if_exists: bool,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
        overwrite: bool,
        ignore_if_exists: bool,
    },
    Delete {
        path: PathBuf,
        recursive: bool,
        ignore_if_not_exists: bool,
    },
}

fn path_of(link: &str) -> Result<PathBuf, String> {
    uri::file_path(link).ok_or_else(|| format!("a file the server named cannot be used: {link}"))
}

/// Which files must be read from disk before the edit can be planned.
fn unopened(app: &AppState, edit: &WorkspaceEdit) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for change in &edit.changes {
        if let Change::Edit(DocumentEdit { uri, .. }) = change {
            let path = path_of(uri)?;
            if app.workspace.file_id_for_path(&path).is_none() && !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    Ok(paths)
}

/// Applies `request`, reading files that are not open first (off the UI thread).
pub fn apply(state: &AppContext, request: Request) {
    let app = state.get();
    let missing = match unopened(&app, &request.edit) {
        Ok(missing) => missing,
        Err(error) => {
            report(state, &request, Err(error));
            return;
        }
    };
    if missing.is_empty() {
        let mut result = Ok(0);
        state.update(|app| result = apply_now(app, &request, HashMap::new()));
        report(state, &request, result);
        return;
    }
    let Some(application) = state.services().and_then(|runtime| runtime.application()) else {
        return;
    };
    let worker_state = state.clone();
    let spawned = std::thread::Builder::new()
        .name("loom-edit-read".to_owned())
        .spawn(move || {
            let mut loaded = HashMap::new();
            let mut failure = None;
            for path in missing {
                let read = std::fs::metadata(&path)
                    .map_err(|error| error.to_string())
                    .and_then(|meta| {
                        if meta.len() > MAX_FILE {
                            Err("the file is too large".to_owned())
                        } else {
                            std::fs::read(&path).map_err(|error| error.to_string())
                        }
                    })
                    .and_then(|bytes| {
                        String::from_utf8(bytes).map_err(|_| "not UTF-8 text".to_owned())
                    });
                match read {
                    Ok(text) => {
                        loaded.insert(path, text);
                    }
                    Err(error) => {
                        failure = Some(format!("{}: {error}", path.display()));
                        break;
                    }
                }
            }
            application.post(move || match failure {
                Some(error) => {
                    worker_state.update(|app| app.show_error(format!("{}: {error}", request.title)))
                }
                None => {
                    let mut result = Ok(0);
                    worker_state.update(|app| result = apply_now(app, &request, loaded));
                    report(&worker_state, &request, result);
                }
            });
        });
    if spawned.is_err() {
        state.update(|app| app.show_error("Could not start reading the files"));
    }
}

fn report(state: &AppContext, request: &Request, result: Result<usize, String>) {
    if let Some(reply) = &request.reply {
        reply.send(Ok(match &result {
            Ok(_) => serde_json::json!({ "applied": true }),
            Err(error) => serde_json::json!({ "applied": false, "failureReason": error }),
        }));
    }
    let title = request.title.as_str();
    state.update(|app| match result {
        Ok(0) => app.show_toast(format!("{title}: nothing to change")),
        Ok(1) => {}
        Ok(files) => app.show_toast(format!("{title}: changed {files} files")),
        Err(error) => app.show_error(format!("{title}: {error}")),
    });
}

/// Plans and performs the edit. Either every text edit is made or none; returns
/// how many files it changed.
pub fn apply_now(
    app: &mut AppState,
    request: &Request,
    loaded: HashMap<PathBuf, String>,
) -> Result<usize, String> {
    let plan = plan(app, request, &loaded)?;
    // Everything that can fail on the text is tried on copies first.
    for step in &plan.steps {
        if let Step::Text(planned) = step {
            let copy = match app.workspace.file_id_for_path(&planned.path) {
                Some(id) => app
                    .workspace
                    .documents
                    .get(&id)
                    .map(|d| d.buffer.clone())
                    .ok_or("a document disappeared")?,
                None => TextBuffer::new(loaded.get(&planned.path).cloned().unwrap_or_default()),
            };
            copy.check_edits(planned.edits.clone())
                .map_err(|error| format!("the edits do not fit the file: {error}"))?;
        }
    }
    let active = app.workspace.active();
    let transaction = next_transaction(app);
    let mut documents: Vec<FileId> = Vec::new();
    let mut changed = 0;
    for step in plan.steps {
        match step {
            Step::Text(planned) => {
                let id = match app.workspace.file_id_for_path(&planned.path) {
                    Some(id) => id,
                    None => {
                        let text = loaded.get(&planned.path).cloned().unwrap_or_default();
                        app.workspace.open_path(planned.path.clone(), text)
                    }
                };
                let Some(pane) = app.workspace.pane_of(id) else {
                    return Err("a changed file has no view".to_owned());
                };
                let Some(mut editor) = app.workspace.view_editor_mut(pane, id) else {
                    return Err("a changed file cannot be edited".to_owned());
                };
                let cursor = editor.cursor();
                editor.begin_transaction(transaction);
                let done = editor.edit(EditKind::Other, planned.edits, |changes, _| {
                    Selection::caret(changes.map(cursor, Assoc::After))
                });
                editor.end_transaction();
                done.map_err(|error| error.to_string())?;
                if !documents.contains(&id) {
                    documents.push(id);
                }
                changed += 1;
            }
            Step::Create {
                path,
                overwrite,
                ignore_if_exists,
            } => {
                if path.exists() {
                    if ignore_if_exists || !overwrite {
                        continue;
                    }
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&path, "").map_err(|e| e.to_string())?;
                changed += 1;
            }
            Step::Rename {
                from,
                to,
                overwrite,
                ignore_if_exists,
            } => {
                if to.exists() && (ignore_if_exists || !overwrite) {
                    continue;
                }
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::rename(&from, &to).map_err(|e| e.to_string())?;
                app.workspace.move_open_path(&from, to.clone());
                changed += 1;
            }
            Step::Delete {
                path,
                recursive,
                ignore_if_not_exists,
            } => {
                if !path.exists() {
                    if ignore_if_not_exists {
                        continue;
                    }
                    return Err(format!("{} does not exist", path.display()));
                }
                app.workspace.close_paths_under(&path);
                if path.is_dir() {
                    if recursive {
                        std::fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
                    } else {
                        std::fs::remove_dir(&path).map_err(|e| e.to_string())?;
                    }
                } else {
                    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                }
                changed += 1;
            }
        }
    }
    if let Some(active) = active {
        app.workspace.reveal(active);
    }
    if documents.len() > 1 {
        app.editor
            .language
            .transactions
            .insert(transaction, documents);
    }
    Ok(changed)
}

fn next_transaction(app: &mut AppState) -> u64 {
    app.editor.language.next_transaction += 1;
    app.editor.language.next_transaction
}

fn plan(
    app: &AppState,
    request: &Request,
    loaded: &HashMap<PathBuf, String>,
) -> Result<Plan, String> {
    let mut plan = Plan::default();
    let mut edited: Vec<PathBuf> = Vec::new();
    for change in &request.edit.changes {
        match change {
            Change::Edit(document) => {
                let path = path_of(&document.uri)?;
                if edited.contains(&path) {
                    return Err("the server edits one file twice".to_owned());
                }
                let text = match app.workspace.file_id_for_path(&path) {
                    Some(id) => {
                        let open = app
                            .workspace
                            .documents
                            .get(&id)
                            .ok_or("a document disappeared")?;
                        // The server computed the edit for the text it was shown.
                        if let Some((_, before)) = request.stamps.iter().find(|(p, _)| *p == path)
                            && *before != open.stamp(id)
                        {
                            return Err(format!(
                                "{} changed while the language server was working",
                                path.display()
                            ));
                        }
                        open.buffer.text_snapshot()
                    }
                    None => loaded
                        .get(&path)
                        .map(|text| std::sync::Arc::new(text.clone()))
                        .ok_or("a file was not read")?,
                };
                let edits = document
                    .edits
                    .iter()
                    .map(|e| Edit::replace(e.span.bytes(&text), e.new_text.clone()))
                    .collect();
                edited.push(path.clone());
                plan.steps.push(Step::Text(Planned { path, edits }));
            }
            Change::Create {
                uri,
                overwrite,
                ignore_if_exists,
            } => plan.steps.push(Step::Create {
                path: path_of(uri)?,
                overwrite: *overwrite,
                ignore_if_exists: *ignore_if_exists,
            }),
            Change::Rename {
                from,
                to,
                overwrite,
                ignore_if_exists,
            } => plan.steps.push(Step::Rename {
                from: path_of(from)?,
                to: path_of(to)?,
                overwrite: *overwrite,
                ignore_if_exists: *ignore_if_exists,
            }),
            Change::Delete {
                uri,
                recursive,
                ignore_if_not_exists,
            } => plan.steps.push(Step::Delete {
                path: path_of(uri)?,
                recursive: *recursive,
                ignore_if_not_exists: *ignore_if_not_exists,
            }),
        }
    }
    Ok(plan)
}

/// Undoes or redoes the change spanning several documents that the active one
/// is part of. `None` when the active document's next step is not one.
pub fn undo_redo(app: &mut AppState, undo: bool) -> Option<Result<(), String>> {
    let top = |app: &AppState, id: FileId| {
        app.workspace.documents.get(&id).and_then(|d| {
            if undo {
                d.buffer.undo_transaction()
            } else {
                d.buffer.redo_transaction()
            }
        })
    };
    let active = app.workspace.active()?;
    let transaction = top(app, active)?;
    let documents = app.editor.language.transactions.get(&transaction)?.clone();
    for id in &documents {
        if top(app, *id) != Some(transaction) {
            return Some(Err(
                "that change spans several files and some were edited since".to_owned(),
            ));
        }
    }
    for id in documents {
        let pane = app.workspace.pane_of(id)?;
        let mut editor = app.workspace.view_editor_mut(pane, id)?;
        if undo {
            editor.undo();
        } else {
            editor.redo();
        }
    }
    Some(Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::language_service::capabilities::PositionEncoding;
    use crate::features::language_service::reply::{TextEdit, workspace_edit};
    use serde_json::json;

    fn uri_of(path: &std::path::Path) -> String {
        uri::file_uri(path).unwrap()
    }

    fn app_with(files: &[(&str, &str)]) -> (AppState, Vec<PathBuf>) {
        let mut app = AppState::new();
        let mut paths = Vec::new();
        for (name, text) in files {
            let path =
                std::env::temp_dir().join(format!("loom-edit-{}-{name}", std::process::id()));
            app.workspace.open_path(path.clone(), (*text).into());
            paths.push(path);
        }
        (app, paths)
    }

    type Replace<'a> = (&'a std::path::Path, (u32, u32), (u32, u32), &'a str);

    fn edit_json(entries: &[Replace]) -> WorkspaceEdit {
        let changes: Vec<_> = entries
            .iter()
            .map(|(path, from, to, text)| {
                json!({
                    "textDocument": { "uri": uri_of(path), "version": 1 },
                    "edits": [{ "range": {
                        "start": { "line": from.0, "character": from.1 },
                        "end": { "line": to.0, "character": to.1 } }, "newText": text }]
                })
            })
            .collect();
        workspace_edit(
            &json!({ "documentChanges": changes }),
            PositionEncoding::Utf16,
        )
        .unwrap()
    }

    fn request(app: &AppState, edit: WorkspaceEdit) -> Request {
        Request {
            edit,
            stamps: stamps(app),
            rect: UiRect::new(0.0, 0.0, 400.0, 300.0),
            title: "Test".into(),
            reply: None,
        }
    }

    fn text_of(app: &AppState, path: &std::path::Path) -> String {
        let id = app.workspace.file_id_for_path(path).unwrap();
        app.workspace.documents[&id].buffer.text().to_owned()
    }

    #[test]
    fn edits_in_several_open_files_apply_together_and_undo_together() {
        let (mut app, paths) = app_with(&[("a.rs", "one two"), ("b.rs", "three four")]);
        let edit = edit_json(&[
            (&paths[0], (0, 0), (0, 3), "ONE"),
            (&paths[1], (0, 6), (0, 10), "FOUR"),
        ]);
        let request = request(&app, edit);
        assert_eq!(apply_now(&mut app, &request, HashMap::new()), Ok(2));
        assert_eq!(text_of(&app, &paths[0]), "ONE two");
        assert_eq!(text_of(&app, &paths[1]), "three FOUR");
        // Undo from the active file takes the other file back too.
        assert_eq!(undo_redo(&mut app, true), Some(Ok(())));
        assert_eq!(text_of(&app, &paths[0]), "one two");
        assert_eq!(text_of(&app, &paths[1]), "three four");
        assert_eq!(undo_redo(&mut app, false), Some(Ok(())));
        assert_eq!(text_of(&app, &paths[0]), "ONE two");
        assert_eq!(text_of(&app, &paths[1]), "three FOUR");
    }

    #[test]
    fn one_bad_edit_changes_nothing_anywhere() {
        let (mut app, paths) = app_with(&[("c.rs", "alpha"), ("d.rs", "beta")]);
        // The second edit overlaps another edit in the same file.
        let mut edit = edit_json(&[(&paths[0], (0, 0), (0, 2), "xx")]);
        let bad = workspace_edit(
            &json!({ "documentChanges": [{
                "textDocument": { "uri": uri_of(&paths[1]), "version": 1 },
                "edits": [
                    { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 3 } }, "newText": "a" },
                    { "range": { "start": { "line": 0, "character": 2 }, "end": { "line": 0, "character": 4 } }, "newText": "b" }
                ] }] }),
            PositionEncoding::Utf16,
        )
        .unwrap();
        edit.changes.extend(bad.changes);
        let request = request(&app, edit);
        let result = apply_now(&mut app, &request, HashMap::new());
        assert!(result.unwrap_err().contains("do not fit"));
        assert_eq!(text_of(&app, &paths[0]), "alpha");
        assert_eq!(text_of(&app, &paths[1]), "beta");
    }

    #[test]
    fn an_edit_for_an_older_state_of_a_file_is_refused() {
        let (mut app, paths) = app_with(&[("e.rs", "text")]);
        let edit = edit_json(&[(&paths[0], (0, 0), (0, 1), "T")]);
        let request = request(&app, edit);
        // The person typed after the question was asked.
        app.workspace.active_editor_mut().unwrap().set_cursor(4);
        {
            let mut editor = app.workspace.active_editor_mut().unwrap();
            loom_editor_core::normal::insert(&mut editor, "!");
        }
        let result = apply_now(&mut app, &request, HashMap::new());
        assert!(result.unwrap_err().contains("changed while"));
        assert_eq!(text_of(&app, &paths[0]), "text!");
    }

    #[test]
    fn a_file_that_is_not_open_is_opened_with_the_edit_unsaved() {
        let (mut app, _) = app_with(&[("open.rs", "x")]);
        let closed =
            std::env::temp_dir().join(format!("loom-edit-{}-closed.rs", std::process::id()));
        let edit = edit_json(&[(&closed, (0, 0), (0, 0), "// ")]);
        let request = request(&app, edit);
        let loaded = HashMap::from([(closed.clone(), "code".to_owned())]);
        assert_eq!(apply_now(&mut app, &request, loaded), Ok(1));
        assert_eq!(text_of(&app, &closed), "// code");
        let id = app.workspace.file_id_for_path(&closed).unwrap();
        assert!(app.workspace.is_dirty(id));
        // The file that was active before is active again.
        assert_eq!(
            app.workspace
                .active_path()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy(),
            format!("loom-edit-{}-open.rs", std::process::id())
        );
    }

    #[test]
    fn undo_is_refused_when_one_of_the_files_was_edited_since() {
        let (mut app, paths) = app_with(&[("f.rs", "aa"), ("g.rs", "bb")]);
        let edit = edit_json(&[
            (&paths[0], (0, 0), (0, 1), "X"),
            (&paths[1], (0, 0), (0, 1), "Y"),
        ]);
        let request = request(&app, edit);
        apply_now(&mut app, &request, HashMap::new()).unwrap();
        // Edit the first file after the transaction.
        let first = app.workspace.file_id_for_path(&paths[0]).unwrap();
        app.workspace.reveal(first);
        {
            let mut editor = app.workspace.active_editor_mut().unwrap();
            editor.set_cursor(2);
            loom_editor_core::normal::insert(&mut editor, "!");
        }
        // The active document's next undo is its own typing, not the transaction.
        assert_eq!(undo_redo(&mut app, true), None);
        // From the other file the transaction is blocked.
        let second = app.workspace.file_id_for_path(&paths[1]).unwrap();
        app.workspace.reveal(second);
        let refused = undo_redo(&mut app, true).unwrap();
        assert!(refused.unwrap_err().contains("edited since"));
        assert_eq!(text_of(&app, &paths[1]), "Yb");
    }

    #[test]
    fn text_edit_spans_follow_the_servers_encoding() {
        let (mut app, paths) = app_with(&[("h.rs", "a😀b")]);
        // UTF-16 columns: the emoji is two units wide, so 'b' is at 3.
        let edit = edit_json(&[(&paths[0], (0, 3), (0, 4), "B")]);
        let request = request(&app, edit);
        apply_now(&mut app, &request, HashMap::new()).unwrap();
        assert_eq!(text_of(&app, &paths[0]), "a😀B");
        let _ = TextEdit {
            span: crate::features::language_service::reply::Span::of(
                "x",
                0..1,
                PositionEncoding::Utf8,
            ),
            new_text: String::new(),
        };
    }
}
