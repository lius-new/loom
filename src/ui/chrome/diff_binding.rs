//! Diff view snapshot and semantic command port.
use crate::domain::identity::PaneId;
use crate::features::git::DiffRequest;
use crate::ui::chrome::tabs_binding::Core;
use lgui::prelude::UiRect;
use std::{path::PathBuf, sync::Arc};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiffSplit {
    pub drag: Option<PaneId>,
    pub ratio: f32,
}
impl Default for DiffSplit {
    fn default() -> Self {
        Self {
            drag: None,
            ratio: 0.5,
        }
    }
}
pub struct EditorFrame {
    pub diff_split: DiffSplit,
    pub editor_hovered: Option<PaneId>,
    pub editor_vertical_scrollbar_dragging: bool,
    pub editor_horizontal_scrollbar_dragging: bool,
}
pub struct Snapshot {
    pub workspace: Core,
    pub split: bool,
    pub editor: EditorFrame,
}
pub enum Command {
    Wheel(PaneId, f32, f32, f32, f32),
    Activate(PaneId),
    SplitStart(PaneId),
    SplitDrag(PaneId, UiRect, f32),
    SplitEnd,
    Open(PathBuf),
    Layout(PaneId, bool),
    Navigate(PaneId, Arc<Vec<usize>>, bool),
    Toggle(PaneId, DiffRequest),
    ToggleAll(PaneId),
}
#[derive(Clone)]
pub struct Binding {
    read: Arc<dyn Fn() -> Snapshot + Send + Sync>,
    run: Arc<dyn Fn(Command) -> bool + Send + Sync>,
}
impl Binding {
    pub fn new(
        read: impl Fn() -> Snapshot + Send + Sync + 'static,
        run: impl Fn(Command) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            read: Arc::new(read),
            run: Arc::new(run),
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        (self.read)()
    }
    pub fn run(&self, c: Command) -> bool {
        (self.run)(c)
    }
}
