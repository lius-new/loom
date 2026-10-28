//! Completion: asking the servers as the person types, keeping the list while
//! typing continues, and turning the chosen item into an edit.
use super::snippet_session;
use super::{AppContext, AppState, Target, services, target};
use crate::app::ui_timer;
use crate::domain::identity::{FileId, PaneId, ViewId};
use crate::features::language_service::completion::{
    Completion, Entry, Shown, commits, filter, insertion, word_start,
};
use crate::features::language_service::reply;
use crate::features::language_service::request::Feature;
use crate::features::language_service::snippet;
use crate::infrastructure::lsp::manager::{RequestHandle, RequestSpec};
use crate::model::text::{Assoc, Edit, EditKind, Selection};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// Typing waits this long before the servers are asked, so a burst is one request.
const DEBOUNCE: Duration = Duration::from_millis(30);

#[derive(Clone)]
pub struct Session {
    pub pane: PaneId,
    pub document: FileId,
    pub view: ViewId,
    pub input_generation: u64,
    pub incarnation: u64,
    /// The text the servers were asked about, to read their ranges in.
    pub requested: Arc<String>,
    pub manual: bool,
    /// Started by a trigger character (so an empty word is expected).
    triggered: bool,
    handle: Option<Arc<RequestHandle>>,
    pub data: Completion,
    pub shown: Vec<Shown>,
    pub selected: usize,
    pub scroll: usize,
    resolving: Option<(usize, Arc<RequestHandle>)>,
    last_cursor: usize,
    last_version: u64,
}

impl Session {
    pub fn set_selected(&mut self, index: usize) {
        if !self.shown.is_empty() {
            self.selected = index.min(self.shown.len() - 1);
            keep_visible(self);
        }
    }

    pub fn scroll_by(&mut self, rows: isize) {
        let max = self.shown.len().saturating_sub(super::popup::VISIBLE) as isize;
        self.scroll = (self.scroll as isize + rows).clamp(0, max) as usize;
    }

    pub fn visible(&self) -> bool {
        !self.shown.is_empty()
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.shown
            .get(self.selected)
            .and_then(|shown| self.data.entries.get(shown.entry))
    }
}

/// Whether the character continues an identifier.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A key was typed into the document (not pasted, not an undo).
pub fn note_typed(app: &mut AppState, input: &str) {
    app.editor.language.typed = input.chars().last();
}

/// Looks at what changed in the document since the last update and starts,
/// follows or ends a completion accordingly. Runs after every state update, so
/// it is cheap when nothing relevant changed.
pub fn observe(ctx: &AppContext, app: &mut AppState, typed: Option<char>) {
    if app
        .workspace
        .active_editor()
        .is_some_and(|e| !e.secondary().is_empty())
    {
        app.editor.language.completion = None;
        app.editor.language.completion_timer = None;
        return;
    }
    let pane = app.workspace.active_pane();
    let Some(t) = target(app, pane) else {
        app.editor.language.completion = None;
        return;
    };
    let mut ended = false;
    if let Some(session) = &app.editor.language.completion {
        if session.document != t.document
            || session.view != t.view
            || session.input_generation != t.input_generation
            || session.incarnation != t.stamp.incarnation
        {
            ended = true;
        } else if session.last_version != t.stamp.version || session.last_cursor != t.cursor {
            ended = !follow(app, &t);
        }
    }
    if ended {
        app.editor.language.completion = None;
    }
    let Some(c) = typed else { return };
    super::refactor::on_type(ctx, app, c);
    // A commit character accepts the highlighted item and is then kept.
    let commit = app
        .editor
        .language
        .completion
        .as_ref()
        .and_then(|s| s.selected_entry())
        .is_some_and(|entry| commits(entry, c));
    if commit {
        accept(app, Some(c));
        return;
    }
    if !is_word(c) && app.editor.language.completion.is_some() {
        app.editor.language.completion = None;
    }
    let triggers = trigger_characters(ctx);
    let is_trigger = triggers.iter().any(|trigger| trigger.chars().eq([c]));
    if (is_word(c) || is_trigger) && app.preferences.values().lsp.completion.auto {
        schedule(ctx, app, is_trigger.then_some(c));
    }
}

/// The characters any running server wants completion after.
fn trigger_characters(ctx: &AppContext) -> Vec<String> {
    services(ctx)
        .map(|s| {
            s.infos()
                .iter()
                .filter_map(|info| info.providers.as_ref()?.completion.as_ref())
                .flat_map(|options| options.trigger_characters.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn offers_completion(ctx: &AppContext) -> bool {
    services(ctx).is_some_and(|s| {
        s.infos().iter().any(|info| {
            info.providers
                .as_ref()
                .is_some_and(|p| p.completion.is_some())
        })
    })
}

/// Asks for completions soon, unless more typing comes first.
fn schedule(ctx: &AppContext, app: &mut AppState, trigger: Option<char>) {
    if !offers_completion(ctx) {
        if app.preferences.values().lsp.completion.words_fallback && trigger.is_none() {
            words(app);
        }
        return;
    }
    let Some(application) = ctx.services().and_then(|r| r.application()) else {
        return;
    };
    let timed = ctx.clone();
    app.editor.language.completion_timer = Some(Arc::new(ui_timer::after(
        application,
        DEBOUNCE,
        move || {
            timed.update(|app| request(&timed, app, false, trigger));
        },
    )));
}

/// The words of the document, as completions, for when no server offers any.
pub fn words(app: &mut AppState) {
    crate::app::multicursor::primary_only(app);
    let pane = app.workspace.active_pane();
    let Some(t) = target(app, pane) else { return };
    let start = word_start(&t.text, t.cursor);
    let mut seen = std::collections::BTreeSet::new();
    let skip = start..crate::features::language_service::completion::word_end(&t.text, t.cursor);
    for (at, word) in words_of(&t.text) {
        if word.chars().count() >= 3 && !skip.contains(&at) {
            seen.insert(word);
        }
        if seen.len() >= 5000 {
            break;
        }
    }
    let list = reply::CompletionList {
        items: seen
            .into_iter()
            .map(|word| reply::CompletionItem {
                label: word,
                label_detail: None,
                kind: None,
                detail: None,
                documentation: None,
                deprecated: false,
                preselect: false,
                sort_text: None,
                filter_text: None,
                insert_text: None,
                snippet: false,
                as_is: false,
                edit: None,
                additional_edits: Vec::new(),
                commit_characters: Vec::new(),
                command: None,
                raw: serde_json::Value::Null,
            })
            .collect(),
        incomplete: false,
    };
    let data = Completion::merge(start, t.cursor, vec![(0, usize::MAX, list)]);
    app.editor.language.completion = Some(Session {
        pane,
        document: t.document,
        view: t.view,
        input_generation: t.input_generation,
        incarnation: t.stamp.incarnation,
        requested: t.text.clone(),
        manual: false,
        triggered: false,
        handle: None,
        data,
        shown: Vec::new(),
        selected: 0,
        scroll: 0,
        resolving: None,
        last_cursor: t.cursor,
        last_version: t.stamp.version,
    });
    let typed = t.text[start..t.cursor].to_owned();
    if let Some(session) = app.editor.language.completion.as_mut() {
        refilter(session, &typed);
        if session.shown.is_empty() {
            app.editor.language.completion = None;
        }
    }
}

/// Identifiers in `text` with where each starts.
pub fn words_of(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut start = None;
    for (at, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        match (is_word(c), start) {
            (true, None) => start = Some(at),
            (false, Some(from)) => {
                let word = &text[from..at];
                if !word.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    out.push((from, word.to_owned()));
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// Asks the servers now. `manual` is the person pressing the key.
pub fn request(ctx: &AppContext, app: &mut AppState, manual: bool, trigger: Option<char>) {
    if manual {
        crate::app::multicursor::primary_only(app);
    } else if app
        .workspace
        .active_editor()
        .is_some_and(|e| !e.secondary().is_empty())
    {
        return;
    }
    app.editor.language.completion_timer = None;
    if !manual && snippet_session::choice_visible(app) {
        return;
    }
    let pane = app.workspace.active_pane();
    let Some(t) = target(app, pane) else { return };
    let Some(services) = services(ctx) else {
        return;
    };
    // The servers must have the text this question is about.
    services.sync(app);
    let start = if trigger.is_some() {
        t.cursor
    } else {
        word_start(&t.text, t.cursor)
    };
    let context = match trigger {
        Some(c) => json!({ "triggerKind": 2, "triggerCharacter": c.to_string() }),
        None if manual => json!({ "triggerKind": 1 }),
        None => json!({ "triggerKind": 1 }),
    };
    let handle = Arc::new(
        services.request(
            RequestSpec::new(Feature::Completion, t.path.clone(), t.lsp_stamp(), t.cursor)
                .extra(json!({ "context": context })),
        ),
    );
    let same = app.editor.language.completion.as_ref().is_some_and(|s| {
        s.document == t.document
            && s.view == t.view
            && s.input_generation == t.input_generation
            && s.data.word_start == start
    });
    if same {
        if let Some(session) = app.editor.language.completion.as_mut() {
            session.resolving = None;
            session.handle = Some(handle);
            session.requested = t.text.clone();
            session.data.cursor = t.cursor;
        }
        return;
    }
    app.editor.language.completion = Some(Session {
        pane,
        document: t.document,
        view: t.view,
        input_generation: t.input_generation,
        incarnation: t.stamp.incarnation,
        requested: t.text.clone(),
        manual,
        triggered: trigger.is_some(),
        handle: Some(handle),
        data: Completion {
            word_start: start,
            cursor: t.cursor,
            ..Completion::default()
        },
        shown: Vec::new(),
        selected: 0,
        scroll: 0,
        resolving: None,
        last_cursor: t.cursor,
        last_version: t.stamp.version,
    });
}

/// Keeps the list in step with the text. Returns false when the list should go.
fn follow(app: &mut AppState, t: &Target) -> bool {
    let Some(session) = app.editor.language.completion.as_mut() else {
        return false;
    };
    let start = session.data.word_start;
    // The text before the word is what the list was asked about; the cursor must
    // still be in the word.
    let prefix_same = t.text.len() >= start
        && session.requested.len() >= start
        && t.text.is_char_boundary(start)
        && t.text.as_bytes()[..start] == session.requested.as_bytes()[..start];
    if !prefix_same || t.cursor < start || !t.text.is_char_boundary(t.cursor) {
        return false;
    }
    let typed = &t.text[start..t.cursor];
    if !typed.chars().all(is_word) {
        return false;
    }
    if typed.is_empty() && !session.manual && !session.triggered {
        return false;
    }
    session.last_cursor = t.cursor;
    let version_changed = session.last_version != t.stamp.version;
    session.last_version = t.stamp.version;
    refilter(session, typed);
    // Typing past the start of a list the servers called incomplete asks again.
    if version_changed && session.data.incomplete && !typed.is_empty() {
        session.data.cursor = t.cursor;
    }
    !(session.shown.is_empty() && session.handle.is_none() && !session.data.incomplete)
}

fn refilter(session: &mut Session, typed: &str) {
    let before = session
        .selected_entry()
        .map(|entry| (entry.item.label.clone(), entry.item.kind));
    session.shown = filter(&session.data, typed);
    session.selected = before
        .and_then(|(label, kind)| {
            session.shown.iter().position(|s| {
                let item = &session.data.entries[s.entry].item;
                item.label == label && item.kind == kind
            })
        })
        .unwrap_or(0);
    session.scroll = 0;
    keep_visible(session);
}

fn keep_visible(session: &mut Session) {
    let rows = super::popup::VISIBLE;
    if session.selected < session.scroll {
        session.scroll = session.selected;
    } else if session.selected >= session.scroll + rows {
        session.scroll = session.selected + 1 - rows;
    }
}

/// Replace candidates and their display indices together. Old indices must
/// never be used to read a shorter or reordered response.
fn replace_candidates(session: &mut Session, mut data: Completion, typed: &str) {
    let selected = session
        .selected_entry()
        .map(|entry| (entry.item.label.clone(), entry.item.kind));
    let resolved: std::collections::HashMap<_, _> = session
        .data
        .entries
        .iter()
        .filter(|entry| entry.resolved)
        .map(|entry| {
            (
                (entry.instance, entry.item.label.clone(), entry.item.kind),
                &entry.item,
            )
        })
        .collect();
    let resolving = session.resolving.take().and_then(|(index, handle)| {
        let old = session.data.entries.get(index)?;
        let index = data
            .entries
            .iter()
            .position(|entry| entry.instance == old.instance && entry.item.raw == old.item.raw)?;
        Some((index, handle))
    });
    for entry in &mut data.entries {
        if let Some(item) =
            resolved.get(&(entry.instance, entry.item.label.clone(), entry.item.kind))
            && item.raw == entry.item.raw
        {
            entry.item = (*item).clone();
            entry.documentation =
                crate::features::language_service::completion::item_documentation(&entry.item);
            entry.resolved = true;
        }
    }
    session.shown.clear();
    session.data = data;
    session.resolving = resolving;
    refilter(session, typed);
    if let Some((label, kind)) = selected
        && let Some(at) = session.shown.iter().position(|shown| {
            let item = &session.data.entries[shown.entry].item;
            item.label == label && item.kind == kind
        })
    {
        session.selected = at;
        keep_visible(session);
    }
}

/// Folds in what the servers answered so far.
pub fn tick(app: &mut AppState) -> bool {
    let text = app_text(app);
    let Some(session) = app.editor.language.completion.as_mut() else {
        return false;
    };
    let mut changed = false;
    if let Some(handle) = session.handle.clone() {
        let progress = handle.poll();
        let sources: Vec<(u64, usize, reply::CompletionList)> = progress
            .parts
            .iter()
            .map(|part| {
                (
                    part.instance,
                    part.rank,
                    reply::completion(&part.value, part.encoding),
                )
            })
            .collect();
        if !sources.is_empty() || progress.finished {
            let word_start = session.data.word_start;
            let cursor = session.data.cursor;
            let merged = Completion::merge(word_start, cursor, sources);
            if progress.finished {
                session.handle = None;
            }
            let typed = typed_prefix(text, session);
            replace_candidates(session, merged, &typed);
            if session.shown.is_empty() && session.handle.is_none() && !session.data.incomplete {
                app.editor.language.completion = None;
            }
            changed = true;
        }
    }
    changed |= tick_resolve(app);
    changed
}

fn app_text(app: &AppState) -> Option<(Arc<String>, usize)> {
    let pane = app.workspace.active_pane();
    target(app, pane).map(|t| (t.text, t.cursor))
}

fn typed_prefix(text: Option<(Arc<String>, usize)>, session: &Session) -> String {
    text.and_then(|(text, cursor)| {
        let start = session.data.word_start;
        (start <= cursor && text.is_char_boundary(start) && text.is_char_boundary(cursor))
            .then(|| text[start..cursor].to_owned())
    })
    .unwrap_or_default()
}

fn tick_resolve(app: &mut AppState) -> bool {
    let Some(session) = app.editor.language.completion.as_mut() else {
        return false;
    };
    let Some((index, handle)) = session.resolving.clone() else {
        return false;
    };
    let progress = handle.poll();
    if !progress.finished {
        return false;
    }
    session.resolving = None;
    if let Some(part) = progress.parts.first()
        && let Some(mut item) = reply::completion_item(&part.value, part.encoding)
        && let Some(entry) = session.data.entries.get_mut(index)
    {
        // Keep the unresolved source identity for matching later snapshots.
        item.raw = entry.item.raw.clone();
        entry.item = item;
        entry.documentation =
            crate::features::language_service::completion::item_documentation(&entry.item);
        entry.resolved = true;
    }
    true
}

/// Moves the highlight by `delta` rows, wrapping.
pub fn move_selection(app: &mut AppState, delta: isize) {
    if let Some(session) = app.editor.language.completion.as_mut() {
        let len = session.shown.len() as isize;
        if len > 0 {
            session.selected = (session.selected as isize + delta).rem_euclid(len) as usize;
            keep_visible(session);
        }
    }
}

pub fn page(app: &mut AppState, delta: isize) {
    if let Some(session) = app.editor.language.completion.as_mut() {
        let len = session.shown.len() as isize;
        if len > 0 {
            session.selected = (session.selected as isize + delta).clamp(0, len - 1) as usize;
            keep_visible(session);
        }
    }
}

/// Asks the server of the highlighted item for its documentation, once.
pub fn resolve_selected(ctx: &AppContext, app: &mut AppState) {
    let pane = app.workspace.active_pane();
    let Some(t) = target(app, pane) else { return };
    let Some(session) = app.editor.language.completion.as_mut() else {
        return;
    };
    if session.resolving.is_some() {
        return;
    }
    let Some(shown) = session.shown.get(session.selected) else {
        return;
    };
    let index = shown.entry;
    let Some(entry) = session.data.entries.get(index) else {
        return;
    };
    if entry.resolved || entry.item.documentation.is_some() {
        return;
    }
    let Some(services) = services(ctx) else {
        return;
    };
    let handle = Arc::new(
        services.request(
            RequestSpec::new(Feature::CompletionResolve, t.path.clone(), t.lsp_stamp(), 0)
                .extra(entry.item.raw.clone())
                .target(entry.instance),
        ),
    );
    session.resolving = Some((index, handle));
}

/// How the list is drawn.
pub fn view(session: &Session) -> crate::ui::editor::popup::View {
    use crate::ui::editor::popup::{Row, View};
    let rows: Vec<Row> = session
        .shown
        .iter()
        .map(|shown| {
            let item = &session.data.entries[shown.entry].item;
            Row {
                label: item.label.clone(),
                detail: item
                    .label_detail
                    .clone()
                    .or_else(|| item.detail.clone())
                    .unwrap_or_default()
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
                badge: kind_badge(item.kind),
                matches: shown.positions.clone(),
                faint: item.deprecated,
            }
        })
        .collect();
    let docs = session
        .selected_entry()
        .and_then(|entry| entry.documentation.clone())
        .map(Into::into);
    View {
        panel: false,
        rows: rows.into(),
        selected: session.selected,
        scroll: session.scroll,
        visible: super::popup::VISIBLE,
        query: None,
        title: None,
        empty: String::new(),
        anchor: Some(session.data.word_start),
        docs,
    }
}

/// A short name for a `CompletionItemKind`.
pub fn kind_badge(kind: Option<u32>) -> &'static str {
    match kind {
        Some(1) => "t",
        Some(2) => "m",
        Some(3) => "fn",
        Some(4) => "c",
        Some(5) => "f",
        Some(6) => "v",
        Some(7) => "C",
        Some(8) => "I",
        Some(9) => "{}",
        Some(10) => "p",
        Some(11) => "u",
        Some(12) => "V",
        Some(13) => "E",
        Some(14) => "k",
        Some(15) => "sn",
        Some(17) => "F",
        Some(19) => "D",
        Some(20) => "e",
        Some(21) => "k",
        Some(22) => "S",
        Some(23) => "ev",
        Some(24) => "op",
        Some(25) => "T",
        _ => "",
    }
}

/// Accepts the highlighted item. `then` is a character typed to commit it; it is
/// inserted after the item.
pub fn accept(app: &mut AppState, then: Option<char>) -> bool {
    let Some(session) = app.editor.language.completion.take() else {
        return false;
    };
    let Some(entry) = session.selected_entry().cloned() else {
        return false;
    };
    let pane = session.pane;
    let Some(t) = target(app, pane) else {
        return false;
    };
    if t.view != session.view || t.input_generation != session.input_generation {
        return false;
    }
    // A committing character is already in the text; the item replaces the word
    // before it and the character goes back after.
    let cursor = match then {
        Some(c) => t.cursor.saturating_sub(c.len_utf8()),
        None => t.cursor,
    };
    let plan = insertion(
        &session.data,
        &entry,
        &session.requested,
        &t.text,
        cursor,
        false,
    );
    let ending = app
        .workspace
        .editor(session.pane)
        .map(|e| e.line_ending())
        .unwrap_or("\n");
    let indent = line_indent(&t.text, plan.range.start);
    let mut snippet = plan.snippet.then(|| {
        let resolve = snippet::insertion_variables(
            &t.path,
            t.selection
                .as_ref()
                .map_or("", |range| &t.text[range.clone()]),
            &t.text,
            cursor,
        );
        let mut parsed = snippet::parse(&plan.text, &resolve);
        if !entry.item.as_is {
            snippet::indent_lines(&mut parsed, &indent);
        }
        snippet::normalize_endings(&mut parsed, ending);
        parsed
    });
    let text = snippet.as_ref().map_or_else(
        || loom_editor_core::text::normalize_endings(&plan.text, ending),
        |s| s.text.clone(),
    );
    let mut edits: Vec<Edit> = plan
        .additional
        .iter()
        .map(|(range, new_text)| {
            Edit::replace(
                range.clone(),
                loom_editor_core::text::normalize_endings(new_text, ending),
            )
        })
        .collect();
    edits.push(Edit::replace(plan.range.clone(), text.clone()));
    if let Some(c) = then {
        // Put the committing character back where it was typed.
        edits.push(Edit::replace(cursor..t.cursor, c.to_string()));
    }
    let start = plan.range.start;
    let Some(mut editor) = app.workspace.editor_mut(pane) else {
        return false;
    };
    let mut origin = 0;
    let applied = editor.edit(EditKind::Other, edits, |changes, _| {
        origin = changes.map(start, Assoc::Before);
        let end = origin + text.len();
        Selection::caret(if then.is_some() {
            changes.map(cursor, Assoc::After) + 1
        } else {
            end
        })
    });
    if applied.is_err() {
        return false;
    }
    let version = editor.version();
    drop(editor);
    if let Some(snippet) = snippet.take() {
        snippet_session::begin(app, pane, origin, &snippet, version);
    }
    true
}

/// The whitespace at the start of the line holding `offset`.
fn line_indent(text: &str, offset: usize) -> String {
    let start = text[..offset.min(text.len())]
        .rfind('\n')
        .map_or(0, |at| at + 1);
    text[start..]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

pub fn close(app: &mut AppState) {
    app.editor.language.completion = None;
    app.editor.language.completion_timer = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word_session() -> Session {
        let mut app = AppState::new();
        app.workspace.open_path(
            std::env::temp_dir().join("completion-refresh.rs"),
            "\nalpha0 alpha1 alpha2 alpha3 alpha4 alpha5 alpha6".into(),
        );
        app.workspace.active_editor_mut().unwrap().set_cursor(0);
        words(&mut app);
        app.editor.language.completion.take().unwrap()
    }

    fn candidates(items: serde_json::Value, instance: u64) -> Completion {
        Completion::merge(
            0,
            0,
            vec![(
                instance,
                0,
                reply::completion(
                    &items,
                    crate::features::language_service::capabilities::PositionEncoding::Utf16,
                ),
            )],
        )
    }

    #[test]
    fn replacing_candidates_rebuilds_old_indices_for_every_shorter_list() {
        for count in 0..=7 {
            let mut session = word_session();
            session.set_selected(6);
            let items = (0..count)
                .map(|index| json!({"label": format!("alpha{index}")}))
                .collect::<Vec<_>>();
            replace_candidates(&mut session, candidates(json!(items), 1), "");
            assert_eq!(session.shown.len(), count);
            assert_eq!(view(&session).rows.len(), count);
            assert!(
                session
                    .shown
                    .iter()
                    .all(|shown| shown.entry < session.data.entries.len())
            );
            if count == 0 {
                assert!(session.selected_entry().is_none());
            } else {
                assert!(session.selected_entry().is_some());
                assert_eq!(session.selected, if count == 7 { 6 } else { 0 });
            }
        }
        let mut session = word_session();
        session.set_selected(6);
        replace_candidates(
            &mut session,
            candidates(
                json!([
                    {"label":"alpha9"}, {"label":"alpha6"}, {"label":"alpha8"}
                ]),
                1,
            ),
            "",
        );
        assert_eq!(session.selected_entry().unwrap().item.label, "alpha6");
        assert_eq!(session.selected, 0);
        assert_eq!(session.shown[0].entry, 1);
    }

    #[test]
    fn resolved_candidates_are_reused_only_for_the_same_server_and_payload() {
        let original = json!([{"label":"alpha", "data":1}]);
        let mut session = word_session();
        replace_candidates(&mut session, candidates(original.clone(), 7), "");
        session.data.entries[0].resolved = true;
        session.data.entries[0].item.documentation = Some("old docs".into());
        for (items, instance, reused) in [
            (original.clone(), 7, true),
            (original, 8, false),
            (json!([{"label":"alpha", "data":2}]), 7, false),
        ] {
            let mut refreshed = session.clone();
            replace_candidates(&mut refreshed, candidates(items, instance), "");
            assert_eq!(refreshed.data.entries[0].resolved, reused);
            assert_eq!(
                refreshed.data.entries[0].item.documentation.is_some(),
                reused
            );
        }
    }

    #[test]
    fn words_are_identifiers_without_the_numbers() {
        let found = words_of("let value_1 = 42 + other; 9abc");
        let words: Vec<&str> = found.iter().map(|(_, w)| w.as_str()).collect();
        assert_eq!(words, ["let", "value_1", "other"]);
        assert_eq!(found[1].0, 4);
    }

    #[test]
    fn indentation_is_the_leading_whitespace_of_the_line() {
        assert_eq!(line_indent("  \tfoo bar", 6), "  \t");
        assert_eq!(line_indent("a\n    b", 7), "    ");
        assert_eq!(line_indent("x", 0), "");
    }

    #[test]
    fn kinds_have_badges() {
        assert_eq!(kind_badge(Some(3)), "fn");
        assert_eq!(kind_badge(Some(15)), "sn");
        assert_eq!(kind_badge(None), "");
    }
}
