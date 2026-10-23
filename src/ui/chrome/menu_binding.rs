//! Tab menu target frame and commands; the app resolves current targets.
use crate::domain::identity::{PaneId, ViewId};
use crate::ui::tab_context_menu::Action;
use lgui::core::UiEventContext;
use std::sync::Arc;
pub struct Frame {
    pub position: (f32, f32),
    pub pane: PaneId,
    pub target: ViewId,
    pub hovered: Option<usize>,
    pub enabled: Vec<Action>,
}
type Execute = dyn Fn(PaneId, ViewId, Action, &mut UiEventContext) + Send + Sync;
#[derive(Clone)]
pub struct Binding {
    read: Arc<dyn Fn() -> Option<Frame> + Send + Sync>,
    hover: Arc<dyn Fn(PaneId, ViewId, Option<usize>) + Send + Sync>,
    dismiss: Arc<dyn Fn(PaneId, ViewId) + Send + Sync>,
    execute: Arc<Execute>,
}
impl Binding {
    pub fn new(
        read: impl Fn() -> Option<Frame> + Send + Sync + 'static,
        hover: impl Fn(PaneId, ViewId, Option<usize>) + Send + Sync + 'static,
        dismiss: impl Fn(PaneId, ViewId) + Send + Sync + 'static,
        execute: impl Fn(PaneId, ViewId, Action, &mut UiEventContext) + Send + Sync + 'static,
    ) -> Self {
        Self {
            read: Arc::new(read),
            hover: Arc::new(hover),
            dismiss: Arc::new(dismiss),
            execute: Arc::new(execute),
        }
    }
    pub fn frame(&self) -> Option<Frame> {
        (self.read)()
    }
    pub fn hover(&self, p: PaneId, t: ViewId, h: Option<usize>) {
        (self.hover)(p, t, h)
    }
    pub fn dismiss(&self, p: PaneId, t: ViewId) {
        (self.dismiss)(p, t)
    }
    pub fn execute(&self, p: PaneId, t: ViewId, a: Action, cx: &mut UiEventContext) {
        (self.execute)(p, t, a, cx);
    }
}
