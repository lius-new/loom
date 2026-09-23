//! Application state — the single reactive root owned by the UI.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::input::keymap::Action;
use crate::model::workspace::Workspace;

/// One entry in a loaded directory listing.
#[derive(Clone)]
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

#[derive(Clone)]
pub struct AppState {
    pub workspace: Workspace,
    pub focused: bool,
    pub show_drawer: bool,
    pub show_terminal: bool,
    /// Current terminal panel height in logical pixels.
    pub terminal_h: f32,
    /// True while the terminal's top resize handle is being dragged.
    pub resizing_terminal: bool,
    pub show_palette: bool,
    pub toast: Option<String>,
    /// Cached entries per loaded directory (key = directory path string).
    /// Renders read from this cache; the filesystem is only hit on open and
    /// on first expand.
    pub dir_entries: HashMap<String, Vec<DirEntry>>,
    /// Directory paths currently expanded in the tree.
    pub expanded: HashSet<String>,
    pub sidebar_w: f32,
    pub resizing_sidebar: bool,
    /// Whether the pointer is currently inside the file-tree drawer.
    pub sidebar_hovered: bool,
    /// Empty-space context menu anchor (screen coords) when open.
    pub context_menu: Option<(f32, f32)>,
    /// Index of the context-menu item currently hovered, if any.
    pub context_menu_hover: Option<usize>,
    /// The folder currently browsed in the file tree, if any.
    pub open_dir: Option<PathBuf>,
    /// Vertical scroll offset of the file tree, in pixels (0 = top).
    pub tree_scroll: f32,
    /// Horizontal scroll offset of the file tree, in pixels (0 = left).
    pub tree_scroll_x: f32,
    /// True while the file-tree scrollbar thumb is being dragged.
    pub scrollbar_dragging: bool,
    /// Pointer y offset from the thumb's top when the drag began.
    pub scrollbar_drag_offset: f32,
    /// True while the horizontal file-tree scrollbar thumb is being dragged.
    pub horizontal_scrollbar_dragging: bool,
    /// Pointer x offset from the horizontal thumb's left when dragging began.
    pub horizontal_scrollbar_drag_offset: f32,
    /// Whether the pointer is currently inside the visible code editor.
    pub editor_hovered: bool,
    /// True while the editor's vertical overlay scrollbar is being dragged.
    pub editor_vertical_scrollbar_dragging: bool,
    /// Pointer y offset from the editor vertical thumb's top.
    pub editor_vertical_scrollbar_drag_offset: f32,
    /// True while the editor's horizontal overlay scrollbar is being dragged.
    pub editor_horizontal_scrollbar_dragging: bool,
    /// Pointer x offset from the editor horizontal thumb's left.
    pub editor_horizontal_scrollbar_drag_offset: f32,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            workspace: Workspace::new(),
            focused: false,
            show_drawer: true,
            show_terminal: false,
            terminal_h: crate::theme::TERMINAL_H,
            resizing_terminal: false,
            show_palette: false,
            toast: None,
            dir_entries: HashMap::new(),
            expanded: HashSet::new(),
            sidebar_w: crate::theme::SIDEBAR_W,
            resizing_sidebar: false,
            sidebar_hovered: false,
            context_menu: None,
            context_menu_hover: None,
            open_dir: None,
            tree_scroll: 0.0,
            tree_scroll_x: 0.0,
            scrollbar_dragging: false,
            scrollbar_drag_offset: 0.0,
            horizontal_scrollbar_dragging: false,
            horizontal_scrollbar_drag_offset: 0.0,
            editor_hovered: false,
            editor_vertical_scrollbar_dragging: false,
            editor_vertical_scrollbar_drag_offset: 0.0,
            editor_horizontal_scrollbar_dragging: false,
            editor_horizontal_scrollbar_drag_offset: 0.0,
        }
    }

    pub fn apply(&mut self, action: Action) {
        match action {
            Action::OpenPalette => {
                self.show_palette = true;
            }
            Action::ToggleDrawer => {
                self.show_drawer = !self.show_drawer;
                if !self.show_drawer {
                    self.sidebar_hovered = false;
                }
            }
            Action::ToggleTerminal => self.show_terminal = !self.show_terminal,
            Action::CloseOverlay => {
                self.show_palette = false;
            }
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
