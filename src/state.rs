//! Application state — the single reactive root owned by the UI.

use crate::input::keymap::Action;
use crate::model::workspace::Workspace;

#[derive(Clone)]
pub struct AppState {
    pub workspace: Workspace,
    pub focused: bool,
    pub show_drawer: bool,
    pub show_assistant: bool,
    pub show_terminal: bool,
    pub show_palette: bool,
    pub show_cmdk: bool,
    pub toast: Option<String>,
    /// Tracks the maximize/restore toggle performed by the custom title bar.
    pub window_maximized: bool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            workspace: Workspace::new(),
            focused: false,
            show_drawer: true,
            show_assistant: true,
            show_terminal: false,
            show_palette: false,
            show_cmdk: false,
            toast: None,
            window_maximized: false,
        }
    }

    pub fn apply(&mut self, action: Action) {
        match action {
            Action::OpenPalette => {
                self.show_palette = true;
                self.show_cmdk = false;
            }
            Action::OpenCmdK => {
                self.show_cmdk = true;
                self.show_palette = false;
            }
            Action::ToggleDrawer => self.show_drawer = !self.show_drawer,
            Action::ToggleAssistant => self.show_assistant = !self.show_assistant,
            Action::ToggleTerminal => self.show_terminal = !self.show_terminal,
            Action::CloseOverlay => {
                self.show_palette = false;
                self.show_cmdk = false;
            }
            Action::OpenFile(id) => self.workspace.set_active(id),
            Action::CloseFile(id) => self.workspace.close(id),
            Action::NextFile => self.workspace.next(),
            Action::PrevFile => self.workspace.prev(),
        }
    }

    pub fn show_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some(msg.into());
    }

    pub fn clear_toast(&mut self) {
        self.toast = None;
    }
}
