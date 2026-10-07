//! Application state — the single reactive root owned by the UI.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::input::keymap::Action;
use crate::model::document::FileId;
use crate::model::workspace::Workspace;
use crate::terminal_session::ShellKind;
use crate::theme::ThemeId;
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
pub struct ExplorerRenameRequest {
    pub path: PathBuf,
    pub kind: ExplorerTargetKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExplorerTargetKind {
    File,
    Directory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplorerContextTarget {
    pub path: PathBuf,
    pub kind: ExplorerTargetKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseContinuation {
    CloseTabs,
    ExitApplication,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseRequest {
    pub targets: Vec<FileId>,
    pub continuation: CloseContinuation,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabDragState {
    pub source: FileId,
    pub pointer_origin_x: f32,
    pub target_index: usize,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabContextMenuState {
    pub position: (f32, f32),
    pub target: FileId,
    pub hovered: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

impl ToastKind {
    /// How long the toast stays up; errors linger so they can be read.
    pub fn duration(self) -> std::time::Duration {
        std::time::Duration::from_millis(match self {
            Self::Info | Self::Success => 3_000,
            Self::Error => 6_000,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub message: String,
}

/// Top-level content shown in the editor region.
///
/// This is derived from the workspace rather than persisted, so opening or
/// closing a project/document cannot leave the chrome and content out of sync.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainSurface {
    Welcome,
    WorkspaceHome,
    Editor,
    Diff,
    Settings,
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
    pub selecting_terminal: bool,
    /// Whether keyboard input is currently routed to the terminal surface.
    pub terminal_focused: bool,
    /// Whether the terminal shell selector is expanded.
    pub terminal_shell_menu: bool,
    /// Shell used for terminals that are not opened from the shell selector.
    /// Active color theme, applied at the start of every frame.
    pub theme: ThemeId,
    pub default_shell: ShellKind,
    pub terminal_cursor_blink: bool,
    /// Horizontal scroll offset of the editor tab strip, in logical pixels.
    pub tab_scroll_x: f32,
    /// Open tab currently under the pointer.
    pub tab_hovered: Option<FileId>,
    /// Whether the pointer is inside the editor tab strip.
    pub tab_strip_hovered: bool,
    /// Active horizontal tab scrollbar thumb drag.
    pub tab_scrollbar_dragging: bool,
    /// Pointer x offset from the tab scrollbar thumb's left edge.
    pub tab_scrollbar_drag_offset: f32,
    /// Pending or active left-button tab reorder gesture.
    pub tab_drag: Option<TabDragState>,
    /// Context menu opened for one editor tab.
    pub tab_context_menu: Option<TabContextMenuState>,
    /// Pending close operation waiting for the user to resolve dirty files.
    pub close_request: Option<CloseRequest>,
    /// Notification above the status bar; cleared by its timer or close button.
    pub toast: Option<Toast>,
    /// Source of `Toast::id`, so a replacement restarts the dismiss timer.
    pub toast_seq: u64,
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
    /// File-tree item targeted by the context menu. `None` means drawer background.
    pub context_menu_target: Option<ExplorerContextTarget>,
    /// Index of the context-menu item currently hovered, if any.
    pub context_menu_hover: Option<usize>,
    /// Inline New File/New Folder editor currently shown in the file tree.
    pub explorer_create: Option<ExplorerCreateRequest>,
    /// Inline directory rename editor currently shown in the file tree.
    pub explorer_rename: Option<ExplorerRenameRequest>,
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
    /// Workspace Home action currently under the pointer.
    pub workspace_home_hover: Option<usize>,
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
            selecting_terminal: false,
            terminal_focused: false,
            terminal_shell_menu: false,
            theme: ThemeId::default(),
            default_shell: ShellKind::default(),
            terminal_cursor_blink: true,
            tab_scroll_x: 0.0,
            tab_hovered: None,
            tab_strip_hovered: false,
            tab_scrollbar_dragging: false,
            tab_scrollbar_drag_offset: 0.0,
            tab_drag: None,
            tab_context_menu: None,
            close_request: None,
            toast: None,
            toast_seq: 0,
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
            explorer_rename: None,
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
            workspace_home_hover: None,
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
        let settings = crate::settings_persistence::load();
        app.git_tree_view = settings.git_tree_view;
        app.git_split_diff = settings.git_split_diff;
        app.git_inline_blame = settings.git_inline_blame;
        app.theme = settings.theme;
        app.default_shell = settings.default_shell;
        app.terminal_cursor_blink = settings.terminal_cursor_blink;
        crate::workspace_actions::hydrate_workspace_folders(&mut app);
        crate::workspace_actions::hydrate_file_tabs(
            &mut app,
            session.open_files,
            session.active_file,
        );
        crate::workspace_actions::open_launch_paths(&mut app, crate::launch::take_paths());
        app
    }

    pub fn apply(&mut self, action: Action) {
        match action {
            Action::CloneRepository => {
                if !self.cloning_repository {
                    self.show_clone_dialog = true;
                    self.clone_repository_error = None;
                }
            }
            Action::OpenSourceControl => {
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
                self.context_menu = None;
                self.context_menu_target = None;
                self.context_menu_hover = None;
                self.tab_context_menu = None;
                self.close_request = None;
                self.tab_drag = None;
                self.tab_scrollbar_dragging = false;
                if !self.cloning_repository {
                    self.show_clone_dialog = false;
                    self.clone_input_focused = false;
                }
            }
            Action::NextFile => self.workspace.next(),
            Action::PrevFile => self.workspace.prev(),
            Action::SelectFile(index) => {
                self.workspace.activate_at(index);
            }
            Action::SelectLastFile => {
                self.workspace.activate_last();
            }
            Action::OpenSettings => {
                self.workspace.open_settings();
            }
            Action::CloseActiveFile => {
                // A pending (possibly batch) request must be resolved first.
                if self.close_request.is_none()
                    && let Some(id) = self.workspace.active()
                {
                    crate::workspace_actions::request_close_tab_in(self, id);
                }
            }
        }
    }

    pub fn main_surface(&self) -> MainSurface {
        if self.workspace.active_is_settings() {
            MainSurface::Settings
        } else if self.workspace.active_diff().is_some() {
            MainSurface::Diff
        } else if self.workspace.active_buffer().is_some() {
            MainSurface::Editor
        } else if self.workspace_folders.is_empty() {
            MainSurface::Welcome
        } else {
            MainSurface::WorkspaceHome
        }
    }

    /// Neutral guidance, e.g. a missing commit message.
    pub fn show_toast(&mut self, msg: impl Into<String>) {
        self.push_toast(ToastKind::Info, msg.into());
    }

    /// Confirms a completed action, e.g. a copied path.
    pub fn show_success(&mut self, msg: impl Into<String>) {
        self.push_toast(ToastKind::Success, msg.into());
    }

    /// Reports a failed operation.
    pub fn show_error(&mut self, msg: impl Into<String>) {
        self.push_toast(ToastKind::Error, msg.into());
    }

    fn push_toast(&mut self, kind: ToastKind, message: String) {
        self.toast_seq += 1;
        self.toast = Some(Toast {
            id: self.toast_seq,
            kind,
            message,
        });
    }

    /// Clears the toast only if it is still `id`, so a stale timer cannot
    /// dismiss a newer message. Returns whether anything changed.
    pub fn dismiss_toast(&mut self, id: u64) -> bool {
        let matches = self.toast.as_ref().is_some_and(|toast| toast.id == id);
        if matches {
            self.toast = None;
        }
        matches
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{DiffTarget, diff::UnifiedDiff};
    use crate::model::diff_document::DiffDocument;
    use std::path::PathBuf;

    #[test]
    fn main_surface_is_derived_from_projects_and_documents() {
        let mut app = AppState::new();
        assert_eq!(app.main_surface(), MainSurface::Welcome);

        app.workspace_folders.push(PathBuf::from("project"));
        assert_eq!(app.main_surface(), MainSurface::WorkspaceHome);

        let file = app
            .workspace
            .open_path(PathBuf::from("project/main.rs"), String::new());
        assert_eq!(app.main_surface(), MainSurface::Editor);

        app.workspace.close(file);
        assert_eq!(app.main_surface(), MainSurface::WorkspaceHome);

        app.workspace.open_settings();
        assert_eq!(app.main_surface(), MainSurface::Settings);

        let settings = app.workspace.active().unwrap();
        app.workspace.close(settings);
        assert_eq!(app.main_surface(), MainSurface::WorkspaceHome);

        let diff = app.workspace.open_diff(DiffDocument::from_unified(
            PathBuf::from("project"),
            PathBuf::from("main.rs"),
            DiffTarget::IndexToWorktree,
            UnifiedDiff::default(),
        ));
        assert_eq!(app.main_surface(), MainSurface::Diff);

        app.workspace.close(diff);
        app.workspace_folders.clear();
        assert_eq!(app.main_surface(), MainSurface::Welcome);
    }

    #[test]
    fn standalone_file_uses_editor_then_returns_to_welcome() {
        let mut app = AppState::new();
        let file = app
            .workspace
            .open_path(PathBuf::from("notes.txt"), String::new());
        assert_eq!(app.main_surface(), MainSurface::Editor);

        app.workspace.close(file);
        assert_eq!(app.main_surface(), MainSurface::Welcome);
    }

    #[test]
    fn stale_toast_timer_does_not_dismiss_a_newer_toast() {
        let mut app = AppState::new();
        app.show_success("Copied path.");
        let first = app.toast.as_ref().unwrap().id;
        app.show_error("Could not save.");

        assert!(!app.dismiss_toast(first));
        assert_eq!(app.toast.as_ref().unwrap().kind, ToastKind::Error);

        let second = app.toast.as_ref().unwrap().id;
        assert!(app.dismiss_toast(second));
        assert!(app.toast.is_none());
    }
}
