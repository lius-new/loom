//! File preparation stays off the event loop. Only a ready document is installed.
use crate::app::context::AppContext;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::app::state::{AppState, TabDrop};
use crate::app::workspace::{OpenMode, PreparedTextFile};
use crate::model::document::FileId;
use crate::model::pane_layout::PaneId;
use lgui::ApplicationHandle;

pub const INDICATOR_DELAY: Duration = Duration::from_millis(300);

#[derive(Clone)]
pub struct PendingFile {
    pub generation: u64,
    pub path: PathBuf,
    pub pane: PaneId,
    pub mode: OpenMode,
    pub drop: Option<TabDrop>,
    pub indicator: bool,
    started: Instant,
    active: Option<FileId>,
    origin_pane: PaneId,
    folders: Vec<PathBuf>,
}

#[derive(Clone)]
pub struct FileLoader {
    worker: crate::infrastructure::file_loader::Worker<PendingFile>,
}
impl crate::infrastructure::file_loader::Request for PendingFile {
    fn generation(&self) -> u64 {
        self.generation
    }
    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}
impl FileLoader {
    pub fn new(state: AppContext, application: ApplicationHandle) -> Self {
        Self::with_prepare(state, application, |request| {
            let text = crate::app::workspace_commands::read_text_file(&request.path)?;
            Ok(PreparedTextFile::new(request.path.clone(), text))
        })
    }
    fn with_prepare(
        state: AppContext,
        application: ApplicationHandle,
        prepare: impl Fn(&PendingFile) -> Result<PreparedTextFile, String> + Send + 'static,
    ) -> Self {
        let ready_state = state.clone();
        let ready_application = application.clone();
        let worker = crate::infrastructure::file_loader::Worker::new(
            prepare,
            move |generation, prepared, lease| {
                let state = ready_state.clone();
                ready_application.post(move || {
                    if lease.is_current(generation) {
                        state.try_update(|app| finish(app, generation, prepared));
                    } else {
                        crate::infrastructure::ui_dispatch::retire(prepared);
                    }
                });
            },
            move |generation, lease| {
                let state = state.clone();
                application.post(move || {
                    if lease.is_current(generation) {
                        state.try_update(|app| show_indicator(app, generation, Instant::now()));
                    }
                });
            },
            INDICATOR_DELAY,
        );
        Self { worker }
    }
    pub(crate) fn invalidate(&self) {
        self.worker.invalidate();
    }
    pub fn shutdown(&self) {
        self.worker.shutdown();
    }
}

/// Returns false only before the application has installed its worker.
pub fn request(
    app: &mut AppState,
    path: PathBuf,
    mode: OpenMode,
    drop: Option<TabDrop>,
    loader: Option<&FileLoader>,
) -> bool {
    let Some(loader) = loader else {
        return false;
    };
    let origin_pane = app.workspace.active_pane();
    if let Some(pending) = app
        .editor
        .pending_file
        .as_mut()
        .filter(|p| p.path == path && p.drop == drop && p.origin_pane == origin_pane)
    {
        if mode == OpenMode::Permanent {
            pending.mode = mode;
        }
        return true;
    }
    let generation = loader.worker.reserve_generation();
    let pane = match drop {
        Some(TabDrop::Insert { pane, .. } | TabDrop::Pane { pane, .. }) => pane,
        None => app.workspace.active_pane(),
    };
    let pending = PendingFile {
        generation,
        path,
        pane,
        mode,
        drop,
        indicator: false,
        started: Instant::now(),
        active: app.workspace.pane(pane).and_then(|p| p.active()),
        origin_pane,
        folders: app.explorer.model.roots().to_vec(),
    };
    app.editor.pending_file = Some(pending.clone());
    if !loader.worker.send_work(pending.clone()) {
        cancel(app);
        app.show_error("Could not start file loading.");
        return true;
    }
    loader.worker.send_timer(pending);
    true
}

pub fn cancel(app: &mut AppState) {
    app.editor.pending_file = None;
}

fn show_indicator(app: &mut AppState, generation: u64, now: Instant) -> bool {
    let Some(pending) = &mut app.editor.pending_file else {
        return false;
    };
    if pending.generation != generation
        || now.saturating_duration_since(pending.started) < INDICATOR_DELAY
    {
        return false;
    }
    if pending.indicator {
        return false;
    }
    pending.indicator = true;
    true
}

fn finish(app: &mut AppState, generation: u64, prepared: Result<PreparedTextFile, String>) -> bool {
    let Some(pending) = app
        .editor
        .pending_file
        .as_ref()
        .filter(|p| p.generation == generation)
        .cloned()
    else {
        crate::infrastructure::ui_dispatch::retire(prepared);
        return false;
    };
    let valid = app.explorer.model.roots() == pending.folders
        && app.workspace.active_pane() == pending.origin_pane
        && app
            .workspace
            .pane(pending.pane)
            .is_some_and(|p| p.active() == pending.active);
    cancel(app);
    if !valid {
        crate::infrastructure::ui_dispatch::retire(prepared);
        return true;
    }
    match prepared {
        Err(error) => app.show_error(error),
        Ok(prepared) => {
            let pane = if let Some(TabDrop::Pane {
                pane,
                split: Some(direction),
            }) = pending.drop
            {
                let Some(new) = app.workspace.split_empty(pane, direction) else {
                    crate::infrastructure::ui_dispatch::retire(prepared);
                    return true;
                };
                new
            } else {
                pending.pane
            };
            app.workspace.activate_pane(pane);
            let id = app.workspace.open_prepared_text(prepared, pending.mode);
            if let Some(TabDrop::Insert { index, .. }) = pending.drop {
                if let Some(position) = app
                    .workspace
                    .pane(pane)
                    .and_then(|p| p.items().iter().position(|item| *item == id))
                {
                    app.workspace.move_tab(
                        pane,
                        id,
                        if position < index {
                            index.saturating_sub(1)
                        } else {
                            index
                        },
                    );
                }
            }
            app.chrome.toast = None;
            app.chrome.welcome_hover = None;
            app.chrome.workspace_home_hover = None;
            app.editor.drag = None;
            app.editor.ime = None;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::UiTasks;
    use lgui::{
        application::AppView,
        core::UiScale,
        prelude::{UiRect, VisualStyle, panel},
        session::UiSession,
    };
    use std::sync::{Arc, Mutex, mpsc};

    struct Harness {
        state: AppContext,
        tasks: UiTasks,
        runtime: Arc<crate::app::runtime::AppRuntime>,
        _session: UiSession,
    }
    impl Harness {
        fn new(
            prepare: impl Fn(&PendingFile) -> Result<PreparedTextFile, String> + Send + 'static,
        ) -> Self {
            let exposed = Arc::new(Mutex::new(None));
            let output = exposed.clone();
            let rect = UiRect::new(0.0, 0.0, 600.0, 400.0);
            let view: AppView = Arc::new(move |cx| {
                let state = AppContext::detached(cx.state_with(|| {
                    let mut app = AppState::new();
                    app.workspace
                        .open_path("current.txt".into(), "Keep this screen".into());
                    app
                }));
                *output.lock().unwrap() = Some(state);
                panel(rect, VisualStyle::default())
            });
            let mut session = UiSession::new();
            session.render_view(&view, rect, UiScale::ONE);
            let state = exposed.lock().unwrap().clone().unwrap();
            let tasks = UiTasks::default();
            let application = tasks.application();
            let runtime = Arc::new(crate::app::runtime::AppRuntime::default());
            let state = state.with_runtime(&runtime);
            let loader = FileLoader::with_prepare(state.clone(), application, prepare);
            runtime.install_file_loader(loader);
            Self {
                state,
                tasks,
                runtime,
                _session: session,
            }
        }
        fn open(&self, path: &str) {
            self.state.update(|app| {
                assert!(crate::app::explorer::open_tree_file(
                    app,
                    path.into(),
                    1,
                    self.runtime.file_loader().as_ref()
                ));
            });
        }
        fn pump_until(&self, predicate: impl Fn(&AppState) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                self.tasks.run_all();
                if predicate(&self.state.get()) {
                    break;
                }
                assert!(Instant::now() < deadline, "file loader did not respond");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
    impl Drop for Harness {
        fn drop(&mut self) {
            self.runtime.clear_file_loader();
        }
    }

    #[test]
    fn slow_open_keeps_current_screen_interactive_and_delays_only_the_icon() {
        let (release, gate) = mpsc::channel();
        let h = Harness::new(move |request| {
            gate.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(PreparedTextFile::new(
                request.path.clone(),
                "new contents".into(),
            ))
        });
        h.open("slow.json");
        let before = h.state.get().workspace.active();
        h.state.update(|app| {
            let pending = app.editor.pending_file.as_ref().unwrap();
            let generation = pending.generation;
            let started = pending.started;
            let mut clock = crate::app::test_support::ManualClock::default();
            clock.advance(Duration::from_millis(299));
            assert!(!show_indicator(app, generation, started + clock.elapsed()));
            assert!(!app.editor.pending_file.as_ref().unwrap().indicator);
            // An unrelated UI click and edit complete while the worker is held.
            app.drawer.visible = !app.drawer.visible;
            app.workspace.active_editor_mut().unwrap().set_cursor(4);
            assert_eq!(app.workspace.active(), before);
            assert_eq!(
                app.workspace.active_editor().unwrap().text(),
                "Keep this screen"
            );
        });
        h.pump_until(|app| app.editor.pending_file.as_ref().unwrap().indicator);
        assert_eq!(h.state.get().workspace.active(), before);
        // Double-click promotion does not restart the timer or disk operation.
        h.state.update(|app| {
            crate::app::explorer::open_tree_file(
                app,
                "slow.json".into(),
                2,
                h.runtime.file_loader().as_ref(),
            );
        });
        release.send(()).unwrap();
        h.pump_until(|app| app.editor.pending_file.is_none());
        let app = h.state.get();
        assert_eq!(
            app.workspace.active_path(),
            Some(std::path::Path::new("slow.json"))
        );
        assert!(
            !app.workspace
                .pane(app.workspace.active_pane())
                .unwrap()
                .is_preview(app.workspace.active().unwrap())
        );
        assert_eq!(
            app.workspace.active_editor().unwrap().text(),
            "new contents"
        );
    }

    #[test]
    fn last_selection_wins_and_a_cancelled_result_cannot_replace_the_screen() {
        let (entered, received) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let h = Harness::new(move |request| {
            if request.path == std::path::Path::new("first.json") {
                entered.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            Ok(PreparedTextFile::new(
                request.path.clone(),
                request.path.to_string_lossy().into_owned(),
            ))
        });
        h.open("first.json");
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        h.open("last.json");
        release.send(()).unwrap();
        h.pump_until(|app| app.editor.pending_file.is_none());
        let app = h.state.get();
        assert_eq!(
            app.workspace.active_path(),
            Some(std::path::Path::new("last.json"))
        );
        assert!(
            app.workspace
                .file_id_for_path(std::path::Path::new("first.json"))
                .is_none()
        );
        assert!(app.editor.pending_file.is_none());
    }

    #[test]
    fn returning_to_an_existing_tab_cancels_a_load_and_its_late_error() {
        let (entered, received) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        let h = Harness::new(move |_| {
            entered.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(5)).unwrap();
            finished.send(()).unwrap();
            Err("old error".into())
        });
        h.open("cancelled.json");
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        h.open("current.txt");
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(5)).unwrap();
        h.pump_until(|app| app.editor.pending_file.is_none());
        assert_eq!(
            h.state.get().workspace.active_path(),
            Some(std::path::Path::new("current.txt"))
        );
        assert!(h.state.get().chrome.toast.is_none());
    }

    #[test]
    fn failures_preserve_the_document_and_finished_loads_ignore_delayed_indicators() {
        let h = Harness::new(|_| Err("Could not open missing.json".into()));
        h.open("missing.json");
        let generation = h
            .state
            .get()
            .editor
            .pending_file
            .as_ref()
            .unwrap()
            .generation;
        h.pump_until(|app| app.editor.pending_file.is_none());
        h.state.update(|app| {
            assert!(!show_indicator(
                app,
                generation,
                Instant::now() + INDICATOR_DELAY
            ));
            assert_eq!(
                app.workspace.active_editor().unwrap().text(),
                "Keep this screen"
            );
            assert!(
                app.chrome
                    .toast
                    .as_ref()
                    .unwrap()
                    .message
                    .contains("missing.json")
            );
        });
    }

    #[test]
    fn final_session_snapshot_includes_pending_and_just_loaded_files_without_a_render() {
        let (release, gate) = mpsc::channel();
        let h = Harness::new(move |request| {
            gate.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(PreparedTextFile::new(request.path.clone(), "ready".into()))
        });
        h.open("single-click.json");
        let expected = crate::app::workspace_session::EditorLayout {
            root: crate::app::workspace_session::SavedPaneNode::Pane {
                items: vec!["current.txt".into(), "single-click.json".into()],
                active: Some("single-click.json".into()),
            },
            active_pane: 0,
        };
        assert_eq!(
            crate::app::workspace_commands::saved_app_editor_layout(&h.state.get()),
            Some(expected.clone())
        );
        release.send(()).unwrap();
        h.pump_until(|app| app.editor.pending_file.is_none());
        // No session/render effect has run since the background result was installed.
        assert_eq!(
            crate::app::workspace_commands::saved_app_editor_layout(&h.state.get()),
            Some(expected)
        );
        assert!(
            h.state
                .get()
                .workspace
                .pane(PaneId::new(1))
                .unwrap()
                .is_preview(h.state.get().workspace.active().unwrap())
        );
    }

    #[test]
    fn changing_workspaces_discards_a_prepared_result_without_reopening_it() {
        let (release, gate) = mpsc::channel();
        let h = Harness::new(move |request| {
            gate.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(PreparedTextFile::new(request.path.clone(), "late".into()))
        });
        h.open("old-workspace.txt");
        h.state.update(|app| {
            app.explorer
                .model
                .restore(vec!["different-workspace".into()], Vec::new())
        });
        release.send(()).unwrap();
        h.pump_until(|app| app.editor.pending_file.is_none());
        assert_eq!(
            h.state.get().workspace.active_path(),
            Some(std::path::Path::new("current.txt"))
        );
    }

    #[test]
    #[ignore = "manual SVG tab appearance verification"]
    fn render_vector_tab_preview() {
        use lgui::{
            application::ApplicationContext,
            core::{PhysicalRect, UiId},
            icons::IconRegistration,
            prelude::group,
            render_skia::SkiaSoftwareSurface,
            renderer::{FrameInfo, FrameReason},
        };
        let rect = UiRect::new(0.0, 0.0, 300.0, 108.0);
        let view: AppView = Arc::new(move |cx| {
            let mut root = group(rect);
            for row in 0..3 {
                let state = AppContext::detached(cx.state_with(|| {
                    let mut app = AppState::new();
                    app.drawer.visible = true;
                    for _ in 0..row {
                        let pane = app.workspace.active_pane();
                        let new = app
                            .workspace
                            .split_empty(pane, crate::model::pane_layout::Direction::Right)
                            .unwrap();
                        app.workspace.activate_pane(new);
                    }
                    let pane = app.workspace.active_pane();
                    if row < 2 {
                        app.editor.pending_file = Some(PendingFile {
                            generation: 1,
                            path: "seed.database.json".into(),
                            pane,
                            mode: OpenMode::Permanent,
                            drop: None,
                            indicator: row == 1,
                            started: Instant::now(),
                            active: None,
                            origin_pane: pane,
                            folders: Vec::new(),
                        });
                    } else {
                        app.workspace
                            .open_path("seed.database.json".into(), String::new());
                    }
                    app
                }));
                let id = UiId::owned(format!("preview-focus-{row}"));
                let focus = cx.focus_handle(id);
                let pane = state.get().workspace.active_pane();
                root = root.child(crate::app::tabs::render(
                    pane,
                    UiRect::new(0.0, row as f32 * 36.0, 300.0, (row + 1) as f32 * 36.0),
                    state,
                    focus,
                ));
            }
            root
        });
        let mut session = UiSession::new();
        session.render_view(&view, rect, UiScale::new(2.0));
        let context = ApplicationContext::empty(Default::default());
        context
            .resources()
            .provide(IconRegistration(Arc::new(crate::app::svg_icons())));
        let mut surface = SkiaSoftwareSurface::new(1024 * 1024);
        let scale = UiScale::new(2.0);
        let frame = FrameInfo::new(
            PhysicalRect::new(0, 0, 600, 216),
            &[],
            scale,
            FrameReason::SceneChange,
            true,
        );
        lgui::assets::backend::with_render_resources(
            &context,
            lgui::assets::backend::render_resources(),
            || {
                surface
                    .draw(&session.tree().scene().project_to_physical(scale), &frame)
                    .unwrap();
            },
        );
        let pixels = surface.pixels().to_vec();
        let image = image::RgbaImage::from_raw(600, 216, pixels).unwrap();
        image
            .save(std::env::var("LOOM_TAB_PREVIEW").expect("preview destination"))
            .unwrap();
    }
    #[test]
    fn queued_completion_cannot_publish_after_loader_shutdown_with_snapshot_retained() {
        let harness = Harness::new(|request| {
            Ok(PreparedTextFile::new(
                request.path.clone(),
                "late payload".into(),
            ))
        });
        harness.open("queued-after-shutdown.txt");
        let deadline = Instant::now() + Duration::from_secs(5);
        while harness.tasks.pending_ids().is_empty() {
            assert!(Instant::now() < deadline, "worker did not queue completion");
            std::thread::yield_now();
        }
        let retained = harness.state.get();
        harness.runtime.clear_file_loader();
        harness.tasks.run_all();
        assert!(
            harness
                .state
                .get()
                .workspace
                .file_id_for_path(std::path::Path::new("queued-after-shutdown.txt"))
                .is_none()
        );
        assert_eq!(
            harness.state.get().workspace.active_path().unwrap(),
            std::path::Path::new("current.txt")
        );
        assert!(retained.editor.pending_file.is_some());
    }
}
