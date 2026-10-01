//! Application state — the single reactive root owned by the UI.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::input::keymap::Action;
use crate::model::workspace::Workspace;
use crate::ui::components::input::InputState;

/// One entry in a loaded directory listing.
#[derive(Clone)]
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExplorerCreateKind {
    File,
    Folder,
}

#[derive(Clone)]
pub struct ExplorerCreateRequest {
    pub kind: ExplorerCreateKind,
    pub parent: PathBuf,
}

#[derive(Clone)]
pub struct AppState {
    pub editor: crate::editor::interaction::EditorInteraction,
    pub workspace: Workspace,
    pub focused: bool,
    /// File explorer drawer on the right.
    pub show_drawer: bool,
    /// Source Control drawer on the left. It is independent from Explorer.
    pub show_source_control: bool,
    pub show_terminal: bool,
    /// Current terminal panel height in logical pixels.
    pub terminal_h: f32,
    /// True while the terminal's top resize handle is being dragged.
    pub resizing_terminal: bool,
    /// Whether keyboard input is currently routed to the terminal surface.
    pub terminal_focused: bool,
    /// Whether the terminal shell selector is expanded.
    pub terminal_shell_menu: bool,
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
    /// Width and drag state for the independent left Source Control drawer.
    pub git_sidebar_w: f32,
    pub resizing_git_sidebar: bool,
    pub git_scroll: f32,
    /// Whether the pointer is currently inside the Source Control drawer.
    pub git_sidebar_hovered: bool,
    pub git_scrollbar_dragging: bool,
    pub git_scrollbar_drag_offset: f32,
    /// Whether the pointer is currently inside the file-tree drawer.
    pub sidebar_hovered: bool,
    /// File-tree row path currently under the pointer.
    pub tree_hovered_path: Option<String>,
    /// Empty-space context menu anchor (screen coords) when open.
    pub context_menu: Option<(f32, f32)>,
    /// Directory targeted by the context menu. `None` means drawer background.
    pub context_menu_target: Option<PathBuf>,
    /// Index of the context-menu item currently hovered, if any.
    pub context_menu_hover: Option<usize>,
    /// Inline New File/New Folder editor currently shown in the file tree.
    pub explorer_create: Option<ExplorerCreateRequest>,
    pub explorer_create_input: InputState,
    pub explorer_create_error: Option<String>,
    /// Root folders currently browsed in the file tree, in display order.
    pub workspace_folders: Vec<PathBuf>,
    /// Previously opened workspace folders, most recently used first.
    pub recent_folders: Vec<PathBuf>,
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
    /// Welcome-page row currently under the pointer.
    pub welcome_hover: Option<usize>,
    /// Whether the repository-clone dialog is visible.
    pub show_clone_dialog: bool,
    /// Repository URL entered in the clone dialog.
    pub clone_repository_url: String,
    /// Whether a git clone process is currently running.
    pub cloning_repository: bool,
    /// Last clone validation or process error shown in the dialog.
    pub clone_repository_error: Option<String>,
    /// Whether the repository URL input currently owns keyboard focus.
    pub clone_input_focused: bool,
    /// Commit input retained across failures and repository refreshes.
    pub git_commit_input: InputState,
    pub git_commit_amend: bool,
    pub git_commit_signoff: bool,
    /// Expanded repository ids in a multi-root workspace.
    pub git_expanded_repositories: HashSet<u64>,
    /// Collapsed Source Control groups: conflicts, staged, unstaged, untracked.
    pub git_collapsed_sections: HashSet<u8>,
    pub git_tree_view: bool,
    pub git_split_diff: bool,
    pub git_inline_blame: bool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            editor: Default::default(),
            workspace: Workspace::new(),
            focused: false,
            show_drawer: true,
            show_source_control: false,
            show_terminal: false,
            terminal_h: crate::theme::TERMINAL_H,
            resizing_terminal: false,
            terminal_focused: false,
            terminal_shell_menu: false,
            show_palette: false,
            toast: None,
            dir_entries: HashMap::new(),
            expanded: HashSet::new(),
            sidebar_w: crate::theme::SIDEBAR_W,
            resizing_sidebar: false,
            git_sidebar_w: crate::theme::SIDEBAR_W,
            resizing_git_sidebar: false,
            git_scroll: 0.0,
            git_sidebar_hovered: false,
            git_scrollbar_dragging: false,
            git_scrollbar_drag_offset: 0.0,
            sidebar_hovered: false,
            tree_hovered_path: None,
            context_menu: None,
            context_menu_target: None,
            context_menu_hover: None,
            explorer_create: None,
            explorer_create_input: InputState::default(),
            explorer_create_error: None,
            workspace_folders: Vec::new(),
            recent_folders: Vec::new(),
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
            welcome_hover: None,
            show_clone_dialog: false,
            clone_repository_url: String::new(),
            cloning_repository: false,
            clone_repository_error: None,
            clone_input_focused: false,
            git_commit_input: InputState::default(),
            git_commit_amend: false,
            git_commit_signoff: false,
            git_expanded_repositories: HashSet::new(),
            git_collapsed_sections: HashSet::new(),
            git_tree_view: false,
            git_split_diff: false,
            git_inline_blame: false,
        }
    }

    /// Build the application state and restore the last file-tree session.
    pub fn restored() -> Self {
        let mut app = Self::new();
        let session = crate::workspace_persistence::load();
        app.show_terminal = session.show_terminal && !session.terminal_tabs.is_empty();
        if let Some(height) = session.terminal_height.filter(|height| height.is_finite()) {
            app.terminal_h = height;
        }
        app.workspace_folders = session.open_folders;
        app.recent_folders = session.recent_folders;
        app.show_source_control = session.source_control_open;
        app.git_tree_view = session.git_tree_view;
        app.git_split_diff = session.git_split_diff;
        app.git_inline_blame = session.git_inline_blame;
        crate::workspace_actions::hydrate_workspace_folders(&mut app);
        crate::workspace_actions::hydrate_file_tabs(
            &mut app,
            session.open_files,
            session.active_file,
        );
        app
    }

    pub fn apply(&mut self, action: Action) {
        match action {
            Action::OpenPalette => {
                self.show_palette = true;
            }
            Action::CloneRepository => {
                if !self.cloning_repository {
                    self.show_palette = false;
                    self.show_clone_dialog = true;
                    self.clone_repository_error = None;
                }
            }
            Action::OpenSourceControl => {
                self.show_palette = false;
                self.show_source_control = true;
            }
            Action::OpenExplorer => {
                self.show_drawer = true;
            }
            Action::ToggleDrawer => {
                self.show_drawer = !self.show_drawer;
                if !self.show_drawer {
                    self.sidebar_hovered = false;
                }
            }
            Action::ToggleTerminal => self.show_terminal = !self.show_terminal,
            Action::CloseOverlay => {
                self.editor.menu = None;
                self.show_palette = false;
                if !self.cloning_repository {
                    self.show_clone_dialog = false;
                    self.clone_input_focused = false;
                }
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
