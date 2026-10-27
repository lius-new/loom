//! Delayed feedback for any visible image canvas waiting for its first pixels.
use super::{context::AppContext, image_preview, state::AppState, ui_timer};
use crate::domain::{document::DocumentStamp, identity::ViewId};
use lgui::ApplicationHandle;
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

struct Waiting {
    stamp: DocumentStamp,
    generation: u64,
    _timer: ui_timer::Timer,
}

pub(super) struct Loading {
    pending: Arc<Mutex<HashMap<ViewId, Waiting>>>,
    next: AtomicU64,
    stopped: AtomicBool,
    state: AppContext,
    application: ApplicationHandle,
}

impl Loading {
    pub fn new(state: AppContext, application: ApplicationHandle) -> Self {
        Self {
            pending: Arc::default(),
            next: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
            state,
            application,
        }
    }

    pub fn sync(&self, app: &AppState) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        let waiting: HashMap<_, _> = app
            .workspace
            .views
            .keys()
            .filter_map(|view| image_preview::loading_stamp(app, *view).map(|stamp| (*view, stamp)))
            .collect();
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|view, entry| waiting.get(view) == Some(&entry.stamp));
        for (view, stamp) in waiting {
            if pending.contains_key(&view) {
                continue;
            }
            let generation = self.next.fetch_add(1, Ordering::Relaxed) + 1;
            let entries = self.pending.clone();
            let state = self.state.clone();
            let timer = ui_timer::after(
                self.application.clone(),
                super::file_loading::INDICATOR_DELAY,
                move || {
                    publish(&state, &entries, view, stamp, generation);
                },
            );
            pending.insert(
                view,
                Waiting {
                    stamp,
                    generation,
                    _timer: timer,
                },
            );
        }
    }

    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        self.pending.lock().unwrap().clear();
    }
}

impl Drop for Loading {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn publish(
    state: &AppContext,
    pending: &Mutex<HashMap<ViewId, Waiting>>,
    view: ViewId,
    stamp: DocumentStamp,
    generation: u64,
) -> bool {
    let current = pending
        .lock()
        .unwrap()
        .get(&view)
        .is_some_and(|entry| entry.stamp == stamp && entry.generation == generation);
    if !current {
        return false;
    }
    state.try_update(|app| {
        if image_preview::loading_stamp(app, view) != Some(stamp)
            || app.editor.images.loading.get(&view) == Some(&stamp)
        {
            return false;
        }
        app.editor.images.loading.insert(view, stamp);
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::{UiTasks, live};
    use crate::domain::{document::DiskState, media::Image, workspace::PreparedTextFile};

    fn labels(state: &AppContext) -> Vec<String> {
        use lgui::{
            application::AppView,
            core::{UiId, UiScale},
            prelude::UiRect,
            session::UiSession,
        };
        let app = state.get();
        let frame = image_preview::frame(&app, app.workspace.active().unwrap()).unwrap();
        let view: AppView = Arc::new(move |cx| {
            crate::ui::image_preview::render(
                frame.clone(),
                UiId::new("image-loading-test"),
                cx.focus_handle(UiId::new("image-loading-test")),
                Arc::new(|_| {}),
            )
        });
        let mut session = UiSession::new();
        session.render_view(&view, UiRect::new(0.0, 0.0, 512.0, 512.0), UiScale::ONE);
        session
            .tree()
            .nodes()
            .iter()
            .filter_map(|node| node.text.as_ref().map(ToString::to_string))
            .collect()
    }

    fn ready(path: &str) -> PreparedTextFile {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        PreparedTextFile::image(
            path.into(),
            Image {
                width: 1,
                height: 1,
                format: "Image",
                byte_len: 1,
                identity: 1,
                frames: vec![crate::domain::media::Frame {
                    bytes: Arc::new(bytes.into_inner()),
                    duration_ms: 100,
                    sequence: 0,
                }],
                animation: None,
            },
            DiskState::from_contents(""),
        )
    }

    #[test]
    fn all_image_formats_share_delayed_feedback_and_ignore_late_callbacks() {
        for path in ["icon.png", "photo.jpg", "animation.gif", "logo.svg"] {
            let mut app = AppState::new();
            let id = app
                .workspace
                .open_prepared_text(ready(path), crate::app::workspace::OpenMode::Permanent);
            // The saved image view exists while the restore worker reads and decodes.
            let doc = app.workspace.documents.get_mut(&id).unwrap();
            doc.image = None;
            doc.loading = Some(false);
            let stamp = doc.stamp(id);
            let view = app.workspace.active().unwrap();
            let state = live(app);
            let tasks = UiTasks::default();
            let loading = Loading::new(state.clone(), tasks.application());
            loading.sync(&state.get());
            let generation = loading.pending.lock().unwrap()[&view].generation;
            assert!(labels(&state).is_empty(), "{path}: no first-frame text");

            // Deliver the delayed callback explicitly; no wall-clock race in the test.
            assert!(publish(&state, &loading.pending, view, stamp, generation));
            assert!(
                labels(&state)
                    .iter()
                    .any(|text| text == "Preparing image preview…")
            );
            assert!(
                state.get().editor.images.svg.is_empty(),
                "loading does not create an SVG result"
            );

            state.update(|app| {
                assert!(
                    app.workspace
                        .install_restored(id, path.as_ref(), ready(path))
                );
            });
            loading.sync(&state.get());
            assert!(!publish(&state, &loading.pending, view, stamp, generation));
            assert!(
                !labels(&state)
                    .iter()
                    .any(|text| text == "Preparing image preview…")
            );
            assert!(loading.pending.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn leaving_a_preview_restarts_the_delay_and_errors_or_shutdown_cancel_it() {
        let mut app = AppState::new();
        let id = app.workspace.open_path("logo.svg".into(), "<svg/>".into());
        let stamp = app.workspace.documents[&id].stamp(id);
        let view = app.workspace.active().unwrap();
        let state = live(app);
        let tasks = UiTasks::default();
        let loading = Loading::new(state.clone(), tasks.application());
        loading.sync(&state.get());
        let old = loading.pending.lock().unwrap()[&view].generation;
        state.update(|app| {
            image_preview::run(app, view, crate::ui::image_preview::Command::Toggle);
        });
        loading.sync(&state.get());
        state.update(|app| {
            image_preview::run(app, view, crate::ui::image_preview::Command::Toggle);
        });
        loading.sync(&state.get());
        let current = loading.pending.lock().unwrap()[&view].generation;
        assert!(!publish(&state, &loading.pending, view, stamp, old));
        assert!(labels(&state).is_empty());
        assert!(publish(&state, &loading.pending, view, stamp, current));

        state.update(|app| {
            app.editor.images.svg.insert(
                id,
                image_preview::SvgPreview {
                    stamp,
                    density: 100,
                    image: None,
                    error: Some("Invalid SVG".into()),
                },
            );
        });
        loading.sync(&state.get());
        assert!(!publish(&state, &loading.pending, view, stamp, current));
        assert!(labels(&state).iter().any(|text| text == "Invalid SVG"));
        state.update(|app| {
            app.editor.images.svg.clear();
        });
        loading.sync(&state.get());
        let current = loading.pending.lock().unwrap()[&view].generation;
        loading.shutdown();
        assert!(!publish(&state, &loading.pending, view, stamp, current));
    }
}
