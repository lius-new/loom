//! Application state — the single reactive root owned by the UI.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::input::action::Action;
use crate::input::keystroke::Keystroke;
use crate::model::document::FileId;
use crate::model::pane_layout::{Direction, PaneId};
use crate::model::workspace::{AppPage, Workspace};
use crate::terminal_session::ShellKind;
use crate::theme::ThemeId;
use crate::ui::components::input::InputState;

/// One entry in a loaded directory listing.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// Tabs to close, as (pane, document).
    pub targets: Vec<(PaneId, FileId)>,
    pub continuation: CloseContinuation,
}

/// Where a dragged tab lands outside its own strip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TabDrop {
    /// Into another pane's strip, before `index` (the tab count appends).
    Insert { pane: PaneId, index: usize },
    /// Onto a pane's surface: merge into it, or split it on one side.
    Pane {
        pane: PaneId,
        split: Option<Direction>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabDragState {
    pub pane: PaneId,
    pub source: FileId,
    pub pointer_origin_x: f32,
    pub pointer_origin_y: f32,
    /// Final index while reordering within the source strip.
    pub target_index: usize,
    pub active: bool,
    /// Set while the pointer is outside the source strip over a drop target.
    pub drop: Option<TabDrop>,
}

/// A file pressed in the Explorer: a click on release, or a drag into the
/// editor once the pointer moves far enough.
#[derive(Clone, Debug, PartialEq)]
pub struct FileDragState {
    pub path: PathBuf,
    /// Click count of the press, so a release still opens like a click.
    pub clicks: u8,
    pub origin: (f32, f32),
    pub point: (f32, f32),
    pub active: bool,
    pub drop: Option<TabDrop>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabContextMenuState {
    pub position: (f32, f32),
    pub pane: PaneId,
    pub target: FileId,
    pub hovered: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalTabContextMenuState {
    pub position: (f32, f32),
    pub target: u64,
    pub kind: TerminalTabMenuKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalTabMenuKind {
    Actions,
    Purpose,
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
    /// An empty pane next to other panes.
    Empty,
    Editor,
    Diff,
    Settings,
    Keymap,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawerMode {
    #[default]
    Files,
    Search,
}

#[derive(Clone)]
pub struct AppState {
    pub editor: crate::editor::interaction::EditorInteraction,
    /// Vim editing behaviour, when enabled in Settings.
    pub vim: crate::vim::Vim,
    pub workspace: Workspace,
    /// Focus of the shared welcome/home/editor surface, retained across swaps.
    pub focused: bool,
    /// File explorer drawer on the right.
    pub show_drawer: bool,
    pub drawer_mode: DrawerMode,
    pub search: crate::search_actions::SearchState,
    /// Source Control drawer on the left. It is independent from Explorer.
    pub show_source_control: bool,
    /// Visibility of the default terminal region, independent of detached regions.
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
    pub terminal_shell_scroll: usize,
    pub terminal_tab_context_menu: Option<TerminalTabContextMenuState>,
    pub application_layout: Option<crate::model::application_layout::ApplicationLayout>,
    pub application_sash_drag: Option<crate::model::application_layout::Divider>,
    /// Shell used for terminals that are not opened from the shell selector.
    /// Active color theme, applied at the start of every frame.
    pub theme: ThemeId,
    pub default_shell: ShellKind,
    pub terminal_profiles: Vec<crate::terminal_profiles::ShellProfile>,
    pub shell_options: Vec<crate::terminal_profiles::ShellOption>,
    pub shell_editor: Option<crate::ui::terminal_settings::ProfileEditor>,
    /// The shell standing in for an unavailable default, if any.
    pub default_shell_fallback: Option<ShellKind>,
    pub terminal_cursor_blink: bool,
    pub smooth_caret: bool,
    pub editor_font: crate::fonts::FontSettings,
    pub terminal_font: crate::fonts::FontSettings,
    /// Font family fields on the Settings page; committed on Enter or blur.
    pub editor_font_input: crate::ui::components::input::InputState,
    pub terminal_font_input: crate::ui::components::input::InputState,
    /// Each workspace folder's `.loom/settings.json`, when it has one.
    pub project_settings: Vec<(PathBuf, crate::project_settings::ProjectSettings)>,
    /// Horizontal scroll offset of each pane's tab strip, in logical pixels.
    pub tab_scroll: HashMap<PaneId, f32>,
    /// Open tab currently under the pointer.
    pub tab_hovered: Option<(PaneId, FileId)>,
    /// The pane whose tab strip contains the pointer.
    pub tab_strip_hovered: Option<PaneId>,
    /// The pane whose tab scrollbar thumb is being dragged.
    pub tab_scrollbar_dragging: Option<PaneId>,
    /// Pointer x offset from the tab scrollbar thumb's left edge.
    pub tab_scrollbar_drag_offset: f32,
    /// Pending or active left-button tab reorder gesture.
    pub tab_drag: Option<TabDragState>,
    /// Pending click or active drag of an Explorer file.
    pub file_drag: Option<FileDragState>,
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
    pub dir_entries: HashMap<String, std::sync::Arc<Vec<DirEntry>>>,
    /// Directory paths currently expanded in the tree.
    pub expanded: HashSet<String>,
    pub sidebar_w: f32,
    pub resizing_sidebar: bool,
    /// The pane divider being dragged.
    pub sash_drag: Option<crate::model::pane_layout::Sash>,
    pub git_split_drag: Option<crate::model::pane_layout::PaneId>,
    pub git_split_ratio: f32,
    pub terminal_split_drag: Option<u64>,
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
    /// A disk refresh that is deferred until an inline Explorer edit ends.
    pub pending_file_tree_refresh: bool,
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
    pub editor_hovered: Option<PaneId>,
    /// The 1-based line under the pointer in a pane's gutter, which
    /// thickens that line's Git change bar.
    pub editor_gutter_hover: Option<(PaneId, usize)>,
    /// The Git change peeked at from its gutter bar.
    pub editor_hunk_peek: Option<crate::editor::hunk_peek::HunkPeek>,
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
    /// Shared by UI commands and the single repository scan worker.
    pub git_poll_control: crate::git::PollControl,
    pub git_application: Option<lgui::ApplicationHandle>,
    pub git_open_diff_sequence: u64,
    pub git_diff_loading: Option<crate::git::DiffRequest>,
    /// The title bar's branch picker, while it is open.
    pub branch_picker: Option<crate::ui::branch_picker::BranchPicker>,
    /// The picker's filter. Kept outside `branch_picker` so the input's
    /// binding stays valid while the picker closes under it.
    pub branch_picker_input: InputState,
    /// A branch switch blocked by local changes, awaiting a stash choice.
    pub stash_switch: Option<crate::ui::stash_switch::StashSwitchRequest>,
    /// The Source Control row under the pointer, showing its action buttons.
    pub git_hover: Option<crate::ui::git_panel::GitHover>,
    /// A Source Control file or commit options menu, while open.
    pub git_menu: Option<crate::ui::git_menu::GitMenu>,
    /// A Source Control action awaiting confirmation.
    pub git_confirm: Option<crate::ui::git_confirm::GitConfirm>,
    /// Recent commits of the active repository, for the history section.
    pub git_history: Option<crate::ui::git_panel::GitHistory>,
    /// The history commit expanded to list its files.
    pub git_history_commit: Option<crate::ui::git_panel::ExpandedCommit>,
    /// The hover card of a history commit; shown while its row is hovered.
    pub git_commit_card: Option<crate::ui::git_panel::CommitCard>,
    /// Hover card details already read, by commit id.
    pub git_commit_details: HashMap<String, Result<crate::git::CommitDetails, String>>,
    /// A repository chosen in Source Control, with the active file at the
    /// time; it stays active until the active file changes.
    pub git_pinned_repository: Option<(PathBuf, Option<PathBuf>)>,
    /// Keystrokes of a key sequence waiting for its next key.
    pub pending_keystrokes: Vec<Keystroke>,
    /// Bumped whenever `pending_keystrokes` changes, restarting its timeout.
    pub pending_keystrokes_seq: u64,
    /// Bumped whenever the active keymap is replaced, so shortcut labels re-render.
    pub keymap_version: u64,
    /// Filter typed into the Keymap page.
    pub keymap_search: InputState,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            editor: Default::default(),
            vim: Default::default(),
            workspace: Workspace::new(),
            focused: false,
            show_drawer: true,
            drawer_mode: DrawerMode::Files,
            search: Default::default(),
            show_source_control: false,
            show_terminal: false,
            terminal_h: crate::theme::TERMINAL_H,
            resizing_terminal: false,
            selecting_terminal: false,
            terminal_focused: false,
            terminal_shell_menu: false,
            terminal_shell_scroll: 0,
            terminal_tab_context_menu: None,
            application_layout: None,
            application_sash_drag: None,
            theme: ThemeId::default(),
            default_shell: ShellKind::default(),
            terminal_profiles: Vec::new(),
            shell_options: crate::terminal_profiles::options(&[]),
            shell_editor: None,
            default_shell_fallback: None,
            terminal_cursor_blink: true,
            smooth_caret: true,
            editor_font: crate::fonts::FontSettings::EDITOR,
            terminal_font: crate::fonts::FontSettings::TERMINAL,
            editor_font_input: Default::default(),
            terminal_font_input: Default::default(),
            project_settings: Vec::new(),
            tab_scroll: HashMap::new(),
            tab_hovered: None,
            tab_strip_hovered: None,
            tab_scrollbar_dragging: None,
            tab_scrollbar_drag_offset: 0.0,
            tab_drag: None,
            file_drag: None,
            tab_context_menu: None,
            close_request: None,
            toast: None,
            toast_seq: 0,
            dir_entries: HashMap::new(),
            expanded: HashSet::new(),
            sidebar_w: crate::theme::SIDEBAR_W,
            resizing_sidebar: false,
            sash_drag: None,
            git_split_drag: None,
            git_split_ratio: 0.5,
            terminal_split_drag: None,
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
            pending_file_tree_refresh: false,
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
            editor_hovered: None,
            editor_gutter_hover: None,
            editor_hunk_peek: None,
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
            git_poll_control: crate::git::PollControl::default(),
            git_application: None,
            git_open_diff_sequence: 0,
            git_diff_loading: None,
            branch_picker: None,
            branch_picker_input: InputState::default(),
            stash_switch: None,
            git_hover: None,
            git_menu: None,
            git_confirm: None,
            git_history: None,
            git_history_commit: None,
            git_commit_card: None,
            git_commit_details: HashMap::new(),
            git_pinned_repository: None,
            pending_keystrokes: Vec::new(),
            pending_keystrokes_seq: 0,
            keymap_version: 0,
            keymap_search: InputState::default(),
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
        app.apply_settings(crate::settings_persistence::load());
        app.refresh_shells();
        crate::workspace_actions::hydrate_workspace_folders(&mut app);
        crate::workspace_actions::hydrate_editor_layout(&mut app, session.editor_layout);
        crate::workspace_actions::open_launch_paths(&mut app, crate::launch::take_paths());
        app
    }

    /// The preferences stored in `settings.json`.
    pub fn settings(&self) -> crate::settings_persistence::Settings {
        crate::settings_persistence::Settings {
            theme: self.theme,
            default_shell: self.default_shell,
            terminal_profiles: self.terminal_profiles.clone(),
            terminal_cursor_blink: self.terminal_cursor_blink,
            smooth_caret: self.smooth_caret,
            git_inline_blame: self.git_inline_blame,
            git_split_diff: self.git_split_diff,
            git_tree_view: self.git_tree_view,
            vim: self.vim.options.into(),
            editor_font: self.editor_font.clone(),
            terminal_font: self.terminal_font.clone(),
        }
    }

    /// Applies preferences read from `settings.json`; returns whether
    /// anything changed.
    pub fn apply_settings(&mut self, settings: crate::settings_persistence::Settings) -> bool {
        if settings == self.settings() {
            return false;
        }
        let shells_changed = settings.default_shell != self.default_shell
            || settings.terminal_profiles != self.terminal_profiles;
        self.theme = settings.theme;
        self.default_shell = settings.default_shell;
        self.terminal_profiles = settings.terminal_profiles;
        self.terminal_cursor_blink = settings.terminal_cursor_blink;
        self.smooth_caret = settings.smooth_caret;
        self.git_inline_blame = settings.git_inline_blame;
        self.git_split_diff = settings.git_split_diff;
        self.git_tree_view = settings.git_tree_view;
        self.set_editor_font(settings.editor_font);
        self.set_terminal_font(settings.terminal_font);
        let vim: crate::vim::Options = settings.vim.into();
        if vim.enabled != self.vim.options.enabled {
            crate::editor::vim_input::set_enabled(self, vim.enabled);
        }
        self.vim.options = vim;
        if shells_changed {
            self.refresh_shells();
        }
        true
    }

    /// Applies an editor font preference, keeping open views on the same line.
    pub fn set_editor_font(&mut self, font: crate::fonts::FontSettings) {
        use crate::model::diff_document::row_height_for;
        let font = font.clamped();
        if font.family != self.editor_font.family {
            self.editor_font_input.set_text(font.family.clone());
        }
        if font == self.editor_font {
            return;
        }
        let (old_size, new_size) = (f32::from(self.editor_font.size), f32::from(font.size));
        self.workspace.rescale_code_scroll(
            font.line_px() / self.editor_font.line_px(),
            row_height_for(new_size) / row_height_for(old_size),
        );
        self.editor_font = font;
    }

    pub fn set_terminal_font(&mut self, font: crate::fonts::FontSettings) {
        let font = font.clamped();
        if font.family != self.terminal_font.family {
            self.terminal_font_input.set_text(font.family.clone());
        }
        self.terminal_font = font;
    }

    /// Re-discovers shells. The saved default is never rewritten here, so a
    /// repaired profile becomes the default again once it is available.
    pub fn refresh_shells(&mut self) {
        self.shell_options = crate::terminal_profiles::options(&self.terminal_profiles);
        let fallback = Some(self.effective_shell()).filter(|kind| *kind != self.default_shell);
        if fallback != self.default_shell_fallback
            && let Some(fallback) = fallback
        {
            let name = self.shell_name(fallback);
            self.show_toast(format!(
                "The default terminal is unavailable; using {name}."
            ));
        }
        self.default_shell_fallback = fallback;
    }

    /// The shell new terminals launch: the saved default when available,
    /// otherwise the first available option.
    pub fn effective_shell(&self) -> ShellKind {
        crate::terminal_profiles::effective_default(self.default_shell, &self.shell_options)
            .unwrap_or(self.default_shell)
    }

    /// `effective_shell` for a terminal starting in `cwd`, honoring the
    /// project settings of the workspace folder that holds it.
    pub fn effective_shell_at(&self, cwd: Option<&Path>) -> ShellKind {
        let preferred = self.settings_at(cwd).default_shell;
        crate::terminal_profiles::effective_default(preferred, &self.shell_options)
            .unwrap_or(preferred)
    }

    /// The workspace folder holding `path`; the innermost when folders nest.
    pub fn workspace_folder_of(&self, path: &Path) -> Option<&Path> {
        self.workspace_folders
            .iter()
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.components().count())
            .map(PathBuf::as_path)
    }

    /// The user's settings with the project settings for `path` applied.
    pub fn settings_at(&self, path: Option<&Path>) -> crate::settings_persistence::Settings {
        let settings = self.settings();
        let project = path
            .and_then(|path| self.workspace_folder_of(path))
            .and_then(|root| self.project_settings.iter().find(|(r, _)| r == root));
        match project {
            Some((_, project)) => {
                crate::settings_persistence::overlay(&settings, &project.overrides)
            }
            None => settings,
        }
    }

    /// Re-reads every workspace folder's `.loom/settings.json`, reporting
    /// problems that are new since the last read.
    pub fn refresh_project_settings(&mut self) {
        use crate::project_settings::{self, ProjectSettings};
        let mut fresh = Vec::new();
        for root in &self.workspace_folders {
            let project = match project_settings::read(root) {
                Ok(Some(project)) => project,
                Ok(None) => continue,
                Err(error) => ProjectSettings {
                    errors: vec![format!("not applied: {error}")],
                    ..Default::default()
                },
            };
            fresh.push((root.clone(), project));
        }
        let mut messages = Vec::new();
        for (root, project) in &fresh {
            let known = self
                .project_settings
                .iter()
                .any(|(r, p)| r == root && p == project);
            if !known && let Some(first) = project.errors.first() {
                let more = match project.errors.len() {
                    1 => String::new(),
                    count => format!(" (and {} more)", count - 1),
                };
                messages.push(format!(
                    "{}: {first}{more}",
                    project_settings::path(root).display()
                ));
            }
        }
        self.project_settings = fresh;
        for message in messages {
            self.show_error(message);
        }
    }

    pub fn shell_name(&self, kind: ShellKind) -> String {
        self.shell_options
            .iter()
            .find(|s| s.kind == kind)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| kind.label().into())
    }

    /// Whether the default terminal region currently owns visible sessions.
    pub fn default_terminal_open(&self) -> bool {
        self.show_terminal
            && self
                .application_layout
                .as_ref()
                .is_none_or(|layout| layout.default_terminal_active().is_some())
    }

    /// Apply a `workspace::`, `pane::` or `menu::` action. Actions that need a
    /// view or a platform service are performed by `key_actions` instead.
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
            Action::ToggleSourceControl => {
                self.show_source_control = !self.show_source_control;
                if !self.show_source_control {
                    self.git_sidebar_hovered = false;
                }
            }
            Action::MenuSelectNext => crate::ui::branch_picker::move_selection(self, 1),
            Action::MenuSelectPrevious => crate::ui::branch_picker::move_selection(self, -1),
            Action::OpenExplorer => {
                self.show_drawer = true;
                self.drawer_mode = DrawerMode::Files;
                self.search.blur();
            }
            Action::OpenSearch => {
                self.show_drawer = true;
                self.drawer_mode = DrawerMode::Search;
                self.search.focus_request += 1;
                self.search.results_focus_request = 0;
                self.close_menus();
            }
            Action::ToggleDrawer => {
                self.show_drawer = !self.show_drawer;
                if !self.show_drawer {
                    self.sidebar_hovered = false;
                    self.search.blur();
                }
            }
            Action::ToggleTerminal => {
                self.show_terminal = !self.default_terminal_open();
                self.terminal_tab_context_menu = None;
            }
            Action::CloseOverlay => {
                self.close_menus();
                self.close_request = None;
                self.tab_drag = None;
                self.file_drag = None;
                self.sash_drag = None;
                self.application_sash_drag = None;
                self.tab_scrollbar_dragging = None;
                if !self.cloning_repository {
                    self.show_clone_dialog = false;
                    self.clone_input_focused = false;
                }
            }
            Action::MenuCancel => self.close_menus(),
            Action::ActivateNextItem => self.workspace.next(),
            Action::ActivatePrevItem => self.workspace.prev(),
            Action::ActivateItem(index) => {
                self.workspace.activate_at(index);
            }
            Action::ActivateLastItem => {
                self.workspace.activate_last();
            }
            Action::OpenSettings => {
                self.refresh_shells();
                self.refresh_project_settings();
                self.workspace.open_settings();
            }
            Action::OpenKeymap => {
                self.workspace.open_keymap();
            }
            Action::IncreaseFontSize | Action::DecreaseFontSize | Action::ResetFontSize => {
                let size = match action {
                    Action::IncreaseFontSize => self.editor_font.size.saturating_add(1),
                    Action::DecreaseFontSize => self.editor_font.size.saturating_sub(1),
                    _ => crate::fonts::FontSettings::EDITOR.size,
                };
                self.set_editor_font(crate::fonts::FontSettings {
                    size,
                    ..self.editor_font.clone()
                });
            }
            Action::SplitRight => self.split_focused(Direction::Right),
            Action::SplitLeft => self.split_focused(Direction::Left),
            Action::SplitUp => self.split_focused(Direction::Up),
            Action::SplitDown => self.split_focused(Direction::Down),
            Action::ActivatePaneLeft => {
                self.workspace.activate_neighbor(Direction::Left);
            }
            Action::ActivatePaneRight => {
                self.workspace.activate_neighbor(Direction::Right);
            }
            Action::ActivatePaneUp => {
                self.workspace.activate_neighbor(Direction::Up);
            }
            Action::ActivatePaneDown => {
                self.workspace.activate_neighbor(Direction::Down);
            }
            Action::ActivateNextPane => {
                self.workspace.activate_next_pane(true);
            }
            Action::ActivatePrevPane => {
                self.workspace.activate_next_pane(false);
            }
            Action::MoveItemToPaneLeft => {
                self.workspace.move_active_item(Direction::Left);
            }
            Action::MoveItemToPaneRight => {
                self.workspace.move_active_item(Direction::Right);
            }
            Action::MoveItemToPaneUp => {
                self.workspace.move_active_item(Direction::Up);
            }
            Action::MoveItemToPaneDown => {
                self.workspace.move_active_item(Direction::Down);
            }
            Action::ClosePane => {
                if self.close_request.is_none() {
                    let pane = self.workspace.active_pane();
                    crate::workspace_actions::request_close_pane_in(self, pane);
                }
            }
            Action::CloseActiveItem => {
                // A pending (possibly batch) request must be resolved first.
                if self.close_request.is_none()
                    && let Some(id) = self.workspace.active()
                {
                    let pane = self.workspace.active_pane();
                    crate::workspace_actions::request_close_tab_in(self, pane, id);
                }
            }
            _ => {}
        }
    }

    fn close_menus(&mut self) {
        self.editor.menu = None;
        self.context_menu = None;
        self.context_menu_target = None;
        self.context_menu_hover = None;
        self.tab_context_menu = None;
        self.terminal_tab_context_menu = None;
        self.branch_picker = None;
        self.git_menu = None;
    }

    /// Split the focused pane, dropping pointer gestures tied to the old layout.
    fn split_focused(&mut self, direction: Direction) {
        let pane = self.workspace.active_pane();
        if self.workspace.split(pane, direction).is_some() {
            self.tab_drag = None;
            self.editor.drag = None;
            self.editor.menu = None;
        }
    }

    /// What the focused pane shows.
    pub fn main_surface(&self) -> MainSurface {
        self.pane_surface(self.workspace.active_pane())
    }

    /// What one pane shows. Welcome and Workspace Home fill the editor area
    /// only while it is a single empty pane.
    pub fn pane_surface(&self, pane: PaneId) -> MainSurface {
        if let Some(page) = self.workspace.pane_page(pane) {
            match page {
                AppPage::Settings => MainSurface::Settings,
                AppPage::Keymap => MainSurface::Keymap,
            }
        } else if self.workspace.diff(pane).is_some() {
            MainSurface::Diff
        } else if self.workspace.editor(pane).is_some() {
            MainSurface::Editor
        } else if self.workspace.pane_count() > 1 {
            MainSurface::Empty
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
