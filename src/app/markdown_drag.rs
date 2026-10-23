//! A single cancellable timer per active reading gesture; callbacks keep only a
//! weak runtime owner and re-check the current view/frame before each step.
use super::markdown_service::MarkdownService;
use crate::{
    domain::view::Presentation, infrastructure::markdown_worker::Consumer,
    ui::markdown::reader::ReaderEvent,
};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
impl MarkdownService {
    fn drag_active(&self, consumer: Consumer) -> bool {
        if !self.alive.load(Ordering::Acquire) {
            return false;
        }
        match consumer {
            Consumer::Card(owner) => self.card_drag_active(owner),
            Consumer::Document(view) => {
                let app = self.context.get();
                let Some(result) = self.render.latest(consumer) else {
                    return false;
                };
                app.editor.focused
                    && app.workspace.active() == Some(view)
                    && app
                        .workspace
                        .view(view)
                        .is_some_and(|view| view.presentation == Presentation::Markdown)
                    && app.editor.markdown_views.get(&view).is_some_and(|state| {
                        state.pressed.is_some()
                            && state.drag_point.is_some_and(|point| {
                                point.1 < 0.0 || point.1 > result.viewport.height
                            })
                    })
            }
        }
    }
    pub(super) fn schedule_drag(self: &Arc<Self>, consumer: Consumer) {
        if !self.drag_active(consumer) {
            self.drag_timers.lock().unwrap().remove(&consumer);
            return;
        }
        let mut timers = self.drag_timers.lock().unwrap();
        if timers.contains_key(&consumer) {
            return;
        }
        let weak = Arc::downgrade(self);
        let timer = super::ui_timer::after(
            self.application.clone(),
            Duration::from_millis(16),
            move || {
                if let Some(service) = weak.upgrade() {
                    service.drag_timers.lock().unwrap().remove(&consumer);
                    if !service.drag_active(consumer) {
                        return;
                    }
                    service.drag_tick(consumer);
                    service.schedule_drag(consumer);
                }
            },
        );
        timers.insert(consumer, Arc::new(timer));
    }
    fn drag_tick(self: &Arc<Self>, consumer: Consumer) {
        match consumer {
            Consumer::Card(owner) => self.tick_card_drag(owner),
            Consumer::Document(view) => {
                let Some(result) = self.render.latest(consumer) else {
                    return;
                };
                let app = self.context.get();
                let Some(value) = app.workspace.view(view).copied() else {
                    return;
                };
                let Some(pane) = app.workspace.pane_of(view) else {
                    return;
                };
                let Some(point) = app
                    .editor
                    .markdown_views
                    .get(&view)
                    .and_then(|state| state.drag_point)
                else {
                    return;
                };
                let frame = super::markdown_view::frame_for(&app, view, &result);
                drop(app);
                let hit = crate::features::markdown::reader::hit_at(
                    &frame.layout,
                    &frame.geometry,
                    &frame.horizontal,
                    frame.viewport,
                    point,
                );
                super::markdown_view::event(
                    &self.context,
                    pane,
                    view,
                    value.input_generation,
                    &result,
                    ReaderEvent::PointerDrag {
                        key: result.frame,
                        hit,
                        point,
                    },
                    self.clipboard.clone(),
                );
            }
        }
    }
}
