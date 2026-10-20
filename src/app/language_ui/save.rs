//! Saving with the language servers' help: organise imports and format first,
//! when the language's settings ask for it, then write the file. A server that
//! is slow or silent never keeps the file from being saved.
use super::edit::{self, Request};
use super::refactor::single_document_edit;
use super::{AppContext, AppState, services, target};
use crate::app::ui_timer;
use crate::domain::document::DocumentStamp;
use crate::domain::identity::{FileId, PaneId};
use crate::features::language_service::reply;
use crate::features::language_service::request::Feature;
use crate::features::language_service::uri;
use crate::infrastructure::lsp::manager::{RequestHandle, RequestSpec};
use lgui::prelude::UiRect;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// How long each server step may take before the file is saved anyway.
const STEP: Duration = Duration::from_secs(1);

/// What to do once the steps are over (normally: write the file).
pub type Then = Arc<dyn Fn(&AppContext) + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Actions,
    Format,
}

#[derive(Clone)]
pub struct Run {
    id: u64,
    stage: Stage,
    pane: PaneId,
    document: FileId,
    stamp: DocumentStamp,
    handle: Option<Arc<RequestHandle>>,
    timer: Option<Arc<ui_timer::Timer>>,
    then: Then,
    rect: UiRect,
    kinds: Vec<String>,
    format: Option<String>,
    do_format: bool,
}

/// What the language's settings ask for before saving.
struct Wanted {
    kinds: Vec<String>,
    format: bool,
    formatter: Option<String>,
}

fn wanted(app: &AppState) -> Option<Wanted> {
    let pane = app.workspace.active_pane();
    let t = target(app, pane)?;
    let language = app.workspace.documents.get(&t.document)?.meta.lang;
    let name = crate::domain::language::Registry::builtin()
        .definition_of(language)
        .name
        .to_string();
    let setting = app.preferences.values().languages.get(&name)?;
    (setting.format_on_save || !setting.code_actions_on_save.is_empty()).then(|| Wanted {
        kinds: setting.code_actions_on_save.clone(),
        format: setting.format_on_save,
        formatter: setting.formatter.clone(),
    })
}

/// Saves the active document after the steps its language asks for.
pub fn save(state: &AppContext, rect: UiRect, then: Then) {
    let app = state.get();
    let Some(wanted) = wanted(&app) else {
        then(state);
        return;
    };
    let pane = app.workspace.active_pane();
    let Some(t) = target(&app, pane) else {
        then(state);
        return;
    };
    let Some(services) = services(state) else {
        then(state);
        return;
    };
    drop(app);
    let run = Run {
        id: 0,
        stage: if wanted.kinds.is_empty() {
            Stage::Format
        } else {
            Stage::Actions
        },
        pane,
        document: t.document,
        stamp: t.stamp,
        handle: None,
        timer: None,
        then: then.clone(),
        rect,
        kinds: wanted.kinds,
        format: wanted.formatter,
        do_format: wanted.format,
    };
    let _ = services;
    let mut started = false;
    state.update(|app| {
        app.editor.language.save_counter += 1;
        let mut run = run;
        run.id = app.editor.language.save_counter;
        started = begin_stage(state, app, run);
    });
    if !started {
        then(state);
    }
}

/// Sends the request of the run's stage. False when there is nothing to ask.
fn begin_stage(state: &AppContext, app: &mut AppState, mut run: Run) -> bool {
    let Some(t) = target(app, run.pane).filter(|t| t.stamp == run.stamp) else {
        return false;
    };
    let Some(services) = services(state) else {
        return false;
    };
    services.sync(app);
    let spec = match run.stage {
        Stage::Actions => RequestSpec::new(Feature::CodeAction, t.path.clone(), t.lsp_stamp(), 0)
            .range(t.text.len())
            .extra(json!({ "context": { "diagnostics": [], "only": run.kinds } })),
        Stage::Format => {
            let mut spec = RequestSpec::new(Feature::Formatting, t.path.clone(), t.lsp_stamp(), 0)
                .extra(json!({ "options": { "tabSize": 4, "insertSpaces": true } }));
            if let Some(name) = &run.format {
                spec = spec.server(name.clone());
            }
            spec
        }
    };
    run.handle = Some(Arc::new(services.request(spec.timeout(STEP))));
    let id = run.id;
    let timed = state.clone();
    if let Some(application) = state.services().and_then(|r| r.application()) {
        run.timer = Some(Arc::new(ui_timer::after(
            application,
            STEP + STEP / 2,
            move || expire(&timed, id),
        )));
    }
    app.editor.language.save = Some(run);
    true
}

/// A step took too long: save without it.
fn expire(state: &AppContext, id: u64) {
    let mut then = None;
    state.update(|app| {
        if app
            .editor
            .language
            .save
            .as_ref()
            .is_some_and(|r| r.id == id)
        {
            then = app.editor.language.save.take().map(|r| r.then);
        }
    });
    if let Some(then) = then {
        then(state);
    }
}

/// Applies a finished step and moves on. Returns what to run when the saving
/// can go ahead.
pub fn tick(state: &AppContext, app: &mut AppState) -> Option<Then> {
    let run = app.editor.language.save.clone()?;
    let progress = run.handle.as_ref()?.poll();
    if !progress.finished {
        return None;
    }
    app.editor.language.save = None;
    let current =
        target(app, run.pane).filter(|t| t.stamp == run.stamp && t.document == run.document);
    if let Some(t) = &current {
        let stamps = edit::stamps(app);
        let link = uri::file_uri(&t.path);
        let mut edits = Vec::new();
        match run.stage {
            Stage::Actions => {
                for part in &progress.parts {
                    for action in reply::code_actions(&part.value, part.encoding) {
                        let wanted = action.kind.as_deref().is_some_and(|kind| {
                            run.kinds
                                .iter()
                                .any(|k| kind == k || kind.starts_with(&format!("{k}.")))
                        });
                        if let (true, Some(edit)) = (wanted, action.edit) {
                            edits.push(edit);
                        }
                    }
                }
            }
            Stage::Format => {
                if let (Some(part), Some(link)) =
                    (progress.usable(Feature::Formatting).first(), link)
                {
                    let found = reply::text_edits(&part.value, part.encoding);
                    if !found.is_empty() {
                        edits.push(single_document_edit(link, found));
                    }
                }
            }
        }
        for edit in edits {
            let request = Request {
                edit,
                stamps: stamps.clone(),
                rect: run.rect,
                title: "Save".to_owned(),
                reply: None,
            };
            if let Err(error) = edit::apply_now(app, &request, HashMap::new()) {
                app.show_toast(format!("Save: {error}"));
            }
        }
    }
    if current.is_some() && run.stage == Stage::Actions && run.do_format {
        let next = Run {
            stage: Stage::Format,
            stamp: target(app, run.pane).map_or(run.stamp, |t| t.stamp),
            ..run.clone()
        };
        if begin_stage(state, app, next) {
            return None;
        }
    }
    Some(run.then)
}
