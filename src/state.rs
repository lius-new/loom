//! Application state — the single reactive root owned by the UI.

use std::collections::HashSet;

use crate::input::keymap::Action;
use crate::model::workspace::Workspace;

#[derive(Clone)]
pub struct AppState {
    pub workspace: Workspace,
    pub focused: bool,
    pub show_drawer: bool,
    pub show_terminal: bool,
    pub show_palette: bool,
    pub toast: Option<String>,
    pub collapsed: HashSet<&'static str>,
    pub sidebar_w: f32,
    pub resizing_sidebar: bool,
    /// Empty-space context menu anchor (screen coords) when open.
    pub context_menu: Option<(f32, f32)>,
    /// Index of the context-menu item currently hovered, if any.
    pub context_menu_hover: Option<usize>,
    /// Folders added via the context menu (names under the `src` root).
    pub added_folders: Vec<String>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            workspace: Workspace::new(),
            focused: false,
            show_drawer: true,
            show_terminal: false,
            show_palette: false,
            toast: None,
            collapsed: HashSet::new(),
            sidebar_w: crate::theme::SIDEBAR_W,
            resizing_sidebar: false,
            context_menu: None,
            context_menu_hover: None,
            added_folders: Vec::new(),
        }
    }

    pub fn apply(&mut self, action: Action) {
        match action {
            Action::OpenPalette => {
                self.show_palette = true;
            }
            Action::ToggleDrawer => self.show_drawer = !self.show_drawer,
            Action::ToggleTerminal => self.show_terminal = !self.show_terminal,
            Action::CloseOverlay => {
                self.show_palette = false;
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
