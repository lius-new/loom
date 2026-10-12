//! Search lifecycle and editor navigation. Workers publish on the UI thread.
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use lgui::ApplicationHandle;
use lgui::core::UiFocusHandle;
use lgui::prelude::{State, UiRect};

use crate::background_ui::LatestNotification;
use crate::search::{self, FileMatches, Matcher, Request, Results, SearchSpec};
use crate::state::AppState;
use crate::ui::components::input::InputState;

pub const FILE_ROW_H: f32 = 24.0;
pub const MATCH_ROW_H: f32 = 20.0;

#[derive(Clone, Default)]
pub struct SearchState {
    pub query: InputState,
    pub include: InputState,
    pub exclude: InputState,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub include_hidden: bool,
    pub include_ignored: bool,
    pub filters_open: bool,
    pub details_open: bool,
    pub retaining_results: bool,

    pub results: Arc<Results>,
    pub running: bool,
    pub cancelled: bool,
    pub error: Option<String>,
    pub generation: u64,
    pub refresh: u64,
    pub focus_request: u64,
    pub results_focus_request: u64,
    pub results_focused: bool,
    pub scroll_y: f32,
    pub viewport_height: f32,
    pub scrollbar_drag: Option<(f32, f32)>,
    pub selected: Option<(PathBuf, usize)>,
    pub collapsed: HashSet<PathBuf>,
    pub cancel: Option<Arc<AtomicBool>>,
    pub applied_key: Option<Key>,
}

impl SearchState {
    pub fn spec(&self) -> SearchSpec {
        SearchSpec {
            query: self.query.text().into(),
            include: self.include.text().into(),
            exclude: self.exclude.text().into(),
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            regex: self.regex,
            include_hidden: self.include_hidden,
            include_ignored: self.include_ignored,
        }
    }

    pub fn composing(&self) -> bool {
        [&self.query, &self.include, &self.exclude]
            .iter()
            .any(|input| !input.preedit.is_empty())
    }

    pub fn blur(&mut self) {
        for input in [&mut self.query, &mut self.include, &mut self.exclude] {
            input.focused = false;
            input.preedit.clear();
            input.preedit_cursor = None;
        }
        self.results_focused = false;
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Key {
    spec: SearchSpec,
    roots: Vec<PathBuf>,
    versions: Vec<(PathBuf, u64)>,
    composing: bool,
    refresh: u64,
}

pub fn key(app: &AppState) -> Key {
    Key {
        spec: app.search.spec(),
        roots: app.workspace_folders.clone(),
        versions: app
            .workspace
            .search_versions()
            .into_iter()
            .filter(|(path, _)| {
                app.workspace_folders
                    .iter()
                    .any(|root| path.starts_with(root))
            })
            .collect(),
        composing: app.search.composing(),
        refresh: app.search.refresh,
    }
}

/// Dropping an effect signals cancellation; never join filesystem I/O on UI.
pub fn start(
    state: State<AppState>,
    application: ApplicationHandle,
    expected: Key,
) -> impl FnOnce() {
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let mut request = None;
    let mut generation = 0;
    state.try_update(|app| {
        if key(app) != expected {
            return false;
        }
        app.search.generation += 1;
        let retain = !expected.spec.query.is_empty()
            && app.search.applied_key.as_ref().is_some_and(|previous| {
                previous.spec == expected.spec && previous.roots == expected.roots
            })
            && !app.search.results.files.is_empty();
        generation = app.search.generation;
        app.search.cancel = Some(cancel.clone());
        app.search.applied_key = Some(expected.clone());
        app.search.running =
            !expected.composing && !expected.spec.query.is_empty() && !expected.roots.is_empty();
        app.search.cancelled = false;
        app.search.error = None;
        app.search.retaining_results = retain;
        if !retain {
            let previous = std::mem::take(&mut app.search.results);
            crate::background_ui::retire(previous);
            app.search.selected = None;
            app.search.scroll_y = 0.0;
        }
        if app.search.running {
            request = Some(Request {
                spec: expected.spec.clone(),
                roots: expected.roots.clone(),
                buffers: app.workspace.search_snapshots(),
            });
        }
        true
    });
    let (stop_tx, stop_rx) = mpsc::channel();
    if let Some(request) = request {
        let apply_state = state.clone();
        let apply_cancel = cancel.clone();
        let queue = LatestNotification::new(application, move |update: Result<Results, String>| {
            apply_state.try_update(|app| {
                if app.search.generation != generation || key(app) != expected {
                    return false;
                }
                match update {
                    Ok(mut results) => {
                        results.cancelled |= apply_cancel.load(Ordering::Relaxed);
                        app.search.running = !results.complete && !results.cancelled;
                        app.search.cancelled = results.cancelled;
                        if app.search.retaining_results
                            && results.files.is_empty()
                            && (!results.complete || results.cancelled)
                        {
                            return true;
                        }
                        app.search.retaining_results = false;
                        if let Some((path, index)) = &app.search.selected
                            && !results
                                .files
                                .iter()
                                .any(|file| &file.path == path && *index < file.matches.len())
                        {
                            app.search.selected = None;
                        }
                        let previous =
                            std::mem::replace(&mut app.search.results, Arc::new(results));
                        crate::background_ui::retire(previous);
                    }
                    Err(error) => {
                        app.search.running = false;
                        app.search.retaining_results = false;
                        app.search.error = Some(error);
                    }
                }
                true
            });
        });
        let failure = queue.clone();
        let spawned = std::thread::Builder::new()
            .name("loom-workspace-search".into())
            .spawn(move || {
                if stop_rx.recv_timeout(Duration::from_millis(200)).is_ok()
                    || cancel.load(Ordering::Relaxed)
                {
                    return;
                }
                if let Err(error) = search::run(request, &cancel, |results| queue.send(Ok(results)))
                {
                    queue.send(Err(error));
                }
            });
        if let Err(error) = spawned {
            failure.send(Err(format!("Could not start search: {error}")));
        }
    }
    move || {
        stop.store(true, Ordering::Relaxed);
        let _ = stop_tx.send(());
    }
}

pub fn toggle_filters(search: &mut SearchState) {
    let closing_focused = search.filters_open && (search.include.focused || search.exclude.focused);
    search.filters_open = !search.filters_open;
    if !search.filters_open {
        for input in [&mut search.include, &mut search.exclude] {
            input.focused = false;
            input.preedit.clear();
            input.preedit_cursor = None;
        }
        if closing_focused {
            search.focus_request += 1;
        }
    }
}

pub fn reset_filters(state: &State<AppState>) {
    state.update(|app| {
        app.search.include.clear();
        app.search.exclude.clear();
        app.search.include_hidden = false;
        app.search.include_ignored = false;
    });
}

pub fn clear(state: &State<AppState>) {
    state.update(|app| {
        if let Some(cancel) = &app.search.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        app.search.generation += 1;
        app.search.query.clear();
        app.search.running = false;
        app.search.cancelled = false;
        app.search.error = None;
        app.search.retaining_results = false;
        app.search.selected = None;
        app.search.scroll_y = 0.0;
        app.search.details_open = false;
        app.search.collapsed.clear();
        let previous = std::mem::take(&mut app.search.results);
        crate::background_ui::retire(previous);
    });
}

pub fn refresh(state: &State<AppState>) {
    state.update(|app| app.search.refresh += 1);
}

pub fn cancel(state: &State<AppState>) {
    state.try_update(|app| {
        if !app.search.running {
            return false;
        }
        if let Some(cancel) = &app.search.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        app.search.cancelled = true;
        app.search.running = false;
        true
    });
}

pub fn selected(app: &AppState) -> Option<(Arc<FileMatches>, usize)> {
    let (path, index) = app.search.selected.as_ref()?;
    app.search
        .results
        .files
        .iter()
        .find(|file| &file.path == path)
        .filter(|file| *index < file.matches.len())
        .map(|file| (file.clone(), *index))
}

pub fn move_selection(app: &mut AppState, delta: isize) {
    let mut total = 0;
    let mut current = None;
    for file in &app.search.results.files {
        if app.search.collapsed.contains(&file.path) {
            continue;
        }
        if let Some((path, index)) = &app.search.selected
            && path == &file.path
            && *index < file.matches.len()
        {
            current = Some(total + index);
        }
        total += file.matches.len();
    }
    if total == 0 {
        return;
    }
    let mut next = current.map_or_else(
        || if delta < 0 { total - 1 } else { 0 },
        |index| index.saturating_add_signed(delta).min(total - 1),
    );
    let mut top = 0.0;
    for file in &app.search.results.files {
        top += FILE_ROW_H;
        if !app.search.collapsed.contains(&file.path) {
            if next < file.matches.len() {
                app.search.selected = Some((file.path.clone(), next));
                top += next as f32 * MATCH_ROW_H;
                break;
            }
            top += file.matches.len() as f32 * MATCH_ROW_H;
            next -= file.matches.len();
        }
    }
    // Initial query focus has not measured the result viewport yet. The result
    // focus callback reveals the selection using its actual height.
    if app.search.viewport_height > 0.0 {
        reveal_at(app, top);
    }
    app.search.results_focus_request += 1;
}

fn reveal_at(app: &mut AppState, top: f32) {
    let height = app.search.viewport_height.max(MATCH_ROW_H);
    if top < app.search.scroll_y {
        app.search.scroll_y = top;
    } else if top + MATCH_ROW_H > app.search.scroll_y + height {
        app.search.scroll_y = (top + MATCH_ROW_H - height).max(0.0);
    }
}

pub fn reveal_selection(app: &mut AppState) {
    let Some((selected_path, index)) = app.search.selected.as_ref() else {
        return;
    };
    let mut top = 0.0;
    for file in &app.search.results.files {
        top += FILE_ROW_H;
        if app.search.collapsed.contains(&file.path) {
            continue;
        }
        if &file.path == selected_path && *index < file.matches.len() {
            top += *index as f32 * MATCH_ROW_H;
            reveal_at(app, top);
            return;
        }
        top += file.matches.len() as f32 * MATCH_ROW_H;
    }
}

pub fn navigate_selected(
    state: &State<AppState>,
    application: ApplicationHandle,
    rect: UiRect,
    editor_focus: UiFocusHandle,
    permanent: bool,
    focus_editor: bool,
) {
    let app = state.get();
    let Some((file, index)) = selected(&app) else {
        return;
    };
    navigate(
        state,
        application,
        rect,
        editor_focus,
        file,
        index,
        permanent,
        focus_editor,
    );
}

pub fn navigate(
    state: &State<AppState>,
    application: ApplicationHandle,
    rect: UiRect,
    editor_focus: UiFocusHandle,
    file: Arc<FileMatches>,
    index: usize,
    permanent: bool,
    focus_editor: bool,
) {
    let app = state.get();
    let Some(found) = file.matches.get(index).cloned() else {
        return;
    };
    if app.search.applied_key.as_ref() != Some(&key(&app))
        || !app
            .search
            .results
            .files
            .iter()
            .any(|current| Arc::ptr_eq(current, &file))
    {
        return;
    }
    let generation = app.search.generation;
    let spec = app.search.spec();
    let buffer = app
        .workspace
        .search_snapshots()
        .into_iter()
        .find(|buffer| buffer.path == file.path);
    state.update(|app| app.search.selected = Some((file.path.clone(), index)));
    let state = state.clone();
    let worker_state = state.clone();
    let worker = std::thread::Builder::new()
        .name("loom-search-open".into())
        .spawn(move || {
            let opened = (|| {
                let text = match &buffer {
                    Some(buffer) => buffer.text.clone(),
                    None => Arc::new(search::read_text(&file.path)?),
                };
                let range = Matcher::new(&spec)?
                    .locate(&text, &file, &found)
                    .ok_or_else(|| {
                        "File changed; searching again to update this result.".to_owned()
                    })?;
                Ok::<_, String>((text, range))
            })();
            application.post(move || {
                let mut focused = false;
                worker_state.try_update(|app| {
                    if app.search.generation != generation
                        || app.search.applied_key.as_ref() != Some(&key(app))
                        || app.search.selected != Some((file.path.clone(), index))
                    {
                        return false;
                    }
                    match opened {
                        Ok((text, range)) => {
                            let current = app
                                .workspace
                                .search_snapshots()
                                .into_iter()
                                .find(|snapshot| snapshot.path == file.path);
                            let unchanged = match (&buffer, &current) {
                                (Some(before), Some(current)) => {
                                    before.version == current.version
                                        && Arc::ptr_eq(&before.text, &current.text)
                                }
                                (None, None) => true,
                                _ => false,
                            };
                            if !unchanged {
                                app.search.refresh += 1;
                                return true;
                            }
                            let contents = if current.is_some() {
                                String::new()
                            } else {
                                text.as_ref().clone()
                            };
                            if permanent {
                                app.workspace.open_path(file.path.clone(), contents);
                            } else {
                                app.workspace.preview_path(file.path.clone(), contents);
                            }
                            if let Some(mut editor) = app.workspace.active_editor_mut() {
                                editor.select_range(range);
                            }
                            crate::editor::editor_view::reveal_cursor(app, rect);
                            app.editor.drag = None;
                            app.editor.ime = None;
                            focused = focus_editor;
                        }
                        Err(error) => {
                            app.show_error(error);
                            app.search.refresh += 1;
                        }
                    }
                    true
                });
                if focused {
                    editor_focus.focus();
                }
            });
        });
    if let Err(error) = worker {
        state.update(|app| app.show_error(format!("Could not open search result: {error}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::action::Action;
    #[test]
    fn selection_scroll_uses_file_and_match_heights_with_collapsed_groups() {
        let mut app = AppState::new();
        let matcher = Matcher::new(&SearchSpec {
            query: "x".into(),
            include: String::new(),
            exclude: String::new(),
            case_sensitive: false,
            whole_word: false,
            regex: false,
            include_hidden: false,
            include_ignored: false,
        })
        .unwrap();
        let files = ["a.rs", "b.rs", "c.rs"]
            .into_iter()
            .map(|name| {
                Arc::new(FileMatches {
                    path: name.into(),
                    root: PathBuf::new(),
                    content_hash: search::hash("x\nx\n"),
                    matches: matcher.matches("x\nx\n", &AtomicBool::new(false), 10),
                })
            })
            .collect::<Vec<_>>();
        app.search.results = Arc::new(Results {
            files,
            matches: 6,
            complete: true,
            ..Results::default()
        });
        app.search.collapsed.insert("a.rs".into());
        move_selection(&mut app, 1);
        assert_eq!(app.search.selected, Some(("b.rs".into(), 0)));
        assert_eq!(app.search.scroll_y, 0.0);
        app.search.viewport_height = 40.0;
        reveal_selection(&mut app);
        assert_eq!(app.search.scroll_y, 28.0);
        move_selection(&mut app, 1);
        assert_eq!(app.search.selected, Some(("b.rs".into(), 1)));
        assert_eq!(app.search.scroll_y, 48.0);
        move_selection(&mut app, 1);
        assert_eq!(app.search.selected, Some(("c.rs".into(), 0)));
        assert_eq!(app.search.scroll_y, 92.0);
        move_selection(&mut app, -1);
        move_selection(&mut app, -1);
        assert_eq!(app.search.scroll_y, 48.0);
    }
    #[test]
    fn query_key_tracks_search_inputs_and_workspace_edits_but_not_view_state() {
        let mut app = AppState::new();
        app.workspace_folders.push(PathBuf::from("project"));
        let original = key(&app);
        app.apply(Action::OpenSearch);
        assert!(key(&app) == original);
        app.search.query.set_text("needle");
        let query = key(&app);
        app.workspace
            .open_path(PathBuf::from("project/test.rs"), "needle".into());
        assert!(key(&app) == query);
        let before = key(&app);
        app.workspace
            .active_editor_mut()
            .unwrap()
            .replace_selection("x", crate::model::text::EditKind::Other)
            .unwrap();
        assert!(key(&app) != before);
        app.search.query.preedit = "中".into();
        assert!(key(&app).composing);
        app.apply(Action::OpenExplorer);
        assert!(
            !app.search.composing(),
            "Hidden inputs must not retain IME ownership"
        );
        let hidden = key(&app);
        app.workspace
            .active_editor_mut()
            .unwrap()
            .replace_selection("y", crate::model::text::EditKind::Other)
            .unwrap();
        assert!(
            key(&app) != hidden,
            "Content changes must invalidate cached results even with the drawer hidden"
        );
        let edited = key(&app);
        app.workspace
            .open_path(PathBuf::from("outside.rs"), "needle".into());
        app.workspace
            .active_editor_mut()
            .unwrap()
            .replace_selection("z", crate::model::text::EditKind::Other)
            .unwrap();
        assert!(
            key(&app) == edited,
            "Files outside the workspace do not affect searching"
        );
        app.search.case_sensitive = true;
        assert!(key(&app) != edited);
    }

    #[test]
    fn search_effect_preserves_results_across_unrelated_ui_and_drawer_changes() {
        use lgui::application::AppView;
        use lgui::core::UiScale;
        use lgui::prelude::{VisualStyle, panel};
        use lgui::session::UiSession;
        use std::sync::Mutex;
        use std::time::Instant;
        type Task = Box<dyn FnOnce() + Send>;
        let root = std::env::temp_dir().join(format!("loom-search-cache-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.txt"), "needle").unwrap();
        let fixture_root = root.clone();
        let exposed = Arc::new(Mutex::new(None::<State<AppState>>));
        let output = exposed.clone();
        let tasks = Arc::new(Mutex::new(Vec::<Task>::new()));
        let posted = tasks.clone();
        let application =
            ApplicationHandle::new(move |task| posted.lock().unwrap().push(task), || {});
        let viewport = UiRect::new(0.0, 0.0, 500.0, 400.0);
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state_with(|| {
                let mut app = AppState::new();
                app.workspace_folders.push(fixture_root.clone());
                app.apply(Action::OpenSearch);
                app.search.query.set_text("needle");
                app
            });
            *output.lock().unwrap() = Some(state.clone());
            let expected = key(&state.get());
            let application = application.clone();
            cx.use_effect(expected.clone(), move || {
                start(state, application, expected)
            });
            panel(viewport, VisualStyle::default())
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        session.runtime().run_effects();
        let state = exposed.lock().unwrap().clone().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.get().search.running && Instant::now() < deadline {
            for task in std::mem::take(&mut *tasks.lock().unwrap()) {
                task();
            }
            session.render_view(&view, viewport, UiScale::ONE);
            session.runtime().run_effects();
            std::thread::sleep(Duration::from_millis(10));
        }
        let result = state.get().search.results.clone();
        assert!(result.complete);
        assert_eq!(result.matches, 1);
        let generation = state.get().search.generation;
        for action in [Action::OpenExplorer, Action::OpenSearch] {
            state.update(|app| app.apply(action));
            session.render_view(&view, viewport, UiScale::ONE);
            session.runtime().run_effects();
            assert_eq!(state.get().search.generation, generation);
            assert!(Arc::ptr_eq(&state.get().search.results, &result));
        }
        state.update(|app| {
            app.search.filters_open = true;
            app.search.details_open = true;
            app.search.scroll_y = 20.0;
            app.show_source_control = !app.show_source_control;
            app.show_drawer = false;
        });
        session.render_view(&view, viewport, UiScale::ONE);
        session.runtime().run_effects();
        state.update(|app| app.show_drawer = true);
        session.render_view(&view, viewport, UiScale::ONE);
        session.runtime().run_effects();
        assert_eq!(state.get().search.generation, generation);
        assert!(Arc::ptr_eq(&state.get().search.results, &result));
        refresh(&state);
        session.render_view(&view, viewport, UiScale::ONE);
        session.runtime().run_effects();
        assert_eq!(state.get().search.generation, generation + 1);
        drop(session);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn older_search_notifications_cannot_replace_new_query_and_navigation_uses_buffer() {
        use lgui::application::AppView;
        use lgui::core::UiScale;
        use lgui::prelude::{VisualStyle, panel};
        use lgui::session::UiSession;
        use std::sync::Mutex;
        use std::time::Instant;
        type Task = Box<dyn FnOnce() + Send>;
        let root = std::env::temp_dir().join(format!("loom-search-actions-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("document.txt");
        std::fs::write(&path, "old disk\n").unwrap();
        let fixture_root = root.clone();
        let fixture_path = path.clone();
        let exposed = Arc::new(Mutex::new(None::<(State<AppState>, UiFocusHandle)>));
        let output = exposed.clone();
        let viewport = UiRect::new(0.0, 0.0, 500.0, 400.0);
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state_with(|| {
                let mut app = AppState::new();
                app.apply(Action::OpenSearch);
                app.workspace_folders.push(fixture_root.clone());
                app.workspace
                    .open_path(fixture_path.clone(), "old disk\n".into());
                app.workspace.active_editor_mut().unwrap().select_all();
                app.workspace
                    .active_editor_mut()
                    .unwrap()
                    .replace_selection("new 中文\r\n", crate::model::text::EditKind::Other)
                    .unwrap();
                app.search.query.set_text("old");
                app
            });
            *output.lock().unwrap() =
                Some((state, cx.focus_handle(lgui::core::UiId::new("test-editor"))));
            panel(viewport, VisualStyle::default())
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let (state, focus) = exposed.lock().unwrap().clone().unwrap();
        let tasks = Arc::new(Mutex::new(Vec::<Task>::new()));
        let posted = tasks.clone();
        let application =
            ApplicationHandle::new(move |task| posted.lock().unwrap().push(task), || {});
        let stop_old = start(state.clone(), application.clone(), key(&state.get()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while tasks.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!tasks.lock().unwrap().is_empty());
        state.update(|app| app.search.query.set_text("中文"));
        let stop_new = start(state.clone(), application.clone(), key(&state.get()));
        stop_old();
        let stale = std::mem::take(&mut *tasks.lock().unwrap());
        for task in stale {
            task();
        }
        assert_eq!(state.get().search.results.matches, 0);
        while state.get().search.running && Instant::now() < deadline {
            let pending = std::mem::take(&mut *tasks.lock().unwrap());
            for task in pending {
                task();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(state.get().search.results.matches, 1);
        state.update(|app| move_selection(app, 1));
        navigate_selected(&state, application, viewport, focus, false, false);
        while tasks.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let pending = std::mem::take(&mut *tasks.lock().unwrap());
        for task in pending {
            task();
        }
        let app = state.get();
        assert_eq!(
            app.workspace.active_editor().unwrap().selected_text(),
            Some("中文")
        );
        assert!(app.workspace.is_dirty(app.workspace.active().unwrap()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old disk\n");
        stop_new();
        let _ = std::fs::remove_dir_all(root);
    }
}
