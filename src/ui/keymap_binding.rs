//! Keymap immutable frame, filter projection and semantic command port.
use crate::features::keymap::bindings::Keymap;
use crate::ui::components::{input::InputState, state_access::StateAccess};
use std::sync::Arc;
#[derive(Clone, Default)]
pub struct KeymapUiState {
    pub search: InputState,
}
#[derive(Clone)]
pub struct Frame {
    pub keymap: Arc<Keymap>,
    pub scroll: (f32, f32),
    pub location: String,
}
pub enum Command {
    Scroll { delta: f32, max: f32 },
    OpenUserFile,
    ClearSearch,
}
#[derive(Clone)]
pub struct Binding {
    pub ui: StateAccess<KeymapUiState>,
    read: Arc<dyn Fn() -> Frame + Send + Sync>,
    run: Arc<dyn Fn(Command) + Send + Sync>,
}
impl Binding {
    pub fn new(
        ui: StateAccess<KeymapUiState>,
        read: impl Fn() -> Frame + Send + Sync + 'static,
        run: impl Fn(Command) + Send + Sync + 'static,
    ) -> Self {
        Self {
            ui,
            read: Arc::new(read),
            run: Arc::new(run),
        }
    }
    pub fn frame(&self) -> Frame {
        (self.read)()
    }
    pub fn run(&self, c: Command) {
        (self.run)(c);
    }
}
