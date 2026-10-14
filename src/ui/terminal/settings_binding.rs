//! Narrow settings projection and profile command port.
use super::state::TerminalUiState;
use crate::features::terminal::{ShellKind, profiles::ShellOption};
use crate::ui::components::state_access::StateAccess;
use std::sync::Arc;
#[derive(Clone)]
pub struct Frame {
    pub default_shell: ShellKind,
    pub effective_shell: ShellKind,
    pub shell_options: Vec<ShellOption>,
    pub project_shell: Option<(String, String)>,
}
impl Frame {
    pub fn shell_name(&self, kind: ShellKind) -> String {
        self.shell_options
            .iter()
            .find(|o| o.kind == kind)
            .map(|o| o.name.clone())
            .unwrap_or_else(|| kind.label().to_string())
    }
}
#[derive(Clone, Copy)]
pub enum Command {
    Edit(Option<u64>),
    Save,
    Remove(u64),
    Close,
    Select(ShellKind),
    Focus,
}
#[derive(Clone)]
pub struct Binding {
    pub ui: StateAccess<TerminalUiState>,
    frame: Arc<dyn Fn() -> Frame + Send + Sync>,
    run: Arc<dyn Fn(Command) + Send + Sync>,
}
impl Binding {
    pub fn new(
        ui: StateAccess<TerminalUiState>,
        frame: impl Fn() -> Frame + Send + Sync + 'static,
        run: impl Fn(Command) + Send + Sync + 'static,
    ) -> Self {
        Self {
            ui,
            frame: Arc::new(frame),
            run: Arc::new(run),
        }
    }
    pub fn frame(&self) -> Frame {
        (self.frame)()
    }
    pub fn run(&self, command: Command) {
        (self.run)(command);
    }
}
