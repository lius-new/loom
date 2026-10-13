//! Source Control view state: pointer, scrolling, the commit form, menus,
//! pickers and dialogs, and the user's view preferences. Business state
//! lives in `features::git::model`.
use std::collections::HashSet;

use crate::features::git::model::GitModel;
use crate::ui::branch_picker::BranchPicker;
use crate::ui::components::input::InputState;
use crate::ui::git_confirm::GitConfirm;
use crate::ui::git_menu::GitMenu;
use crate::ui::git_panel::{CommitCard, GitHover};
use crate::ui::stash_switch::StashSwitchRequest;

#[derive(Clone)]
pub struct GitUiState {
    /// Pointer is inside the Source Control drawer.
    pub hovered: bool,
    /// The drawer row under the pointer, which shows its action buttons.
    pub hover: Option<GitHover>,
    pub scroll: f32,
    /// Independent scroll of the Worktrees/Stashes/Commits area.
    pub details_scroll: f32,
    pub scrollbar_dragging: bool,
    pub scrollbar_drag_offset: f32,
    pub details_scrollbar_dragging: bool,
    pub details_scrollbar_drag_offset: f32,
    pub commit_input: InputState,
    pub commit_amend: bool,
    pub commit_signoff: bool,
    pub expanded_repositories: HashSet<u64>,
    pub collapsed_sections: HashSet<u8>,
    /// Group change sections by folder.
    pub tree_view: bool,
    /// Show diffs side by side.
    pub split_diff: bool,
    /// Blame the caret line in the editor.
    pub inline_blame: bool,
    pub menu: Option<GitMenu>,
    pub confirm: Option<GitConfirm>,
    /// The hovered history commit's card.
    pub commit_card: Option<CommitCard>,
    pub branch_picker: Option<BranchPicker>,
    /// The picker's filter, kept outside `branch_picker` so the input's
    /// focus and IME state survive listing reloads.
    pub branch_picker_input: InputState,
    pub stash_switch: Option<StashSwitchRequest>,
    pub clone_visible: bool,
    pub clone_url: String,
    pub clone_error: Option<String>,
    pub clone_input_focused: bool,
}

impl Default for GitUiState {
    fn default() -> Self {
        Self {
            hovered: false,
            hover: None,
            scroll: 0.0,
            details_scroll: 0.0,
            scrollbar_dragging: false,
            scrollbar_drag_offset: 0.0,
            details_scrollbar_dragging: false,
            details_scrollbar_drag_offset: 0.0,
            commit_input: InputState::default(),
            commit_amend: false,
            commit_signoff: false,
            expanded_repositories: HashSet::new(),
            collapsed_sections: [
                crate::ui::git_panel::WORKTREES,
                crate::ui::git_panel::STASHES,
                crate::ui::git_panel::HISTORY,
            ]
            .into_iter()
            .collect(),
            tree_view: false,
            split_diff: false,
            inline_blame: false,
            menu: None,
            confirm: None,
            commit_card: None,
            branch_picker: None,
            branch_picker_input: InputState::default(),
            stash_switch: None,
            clone_visible: false,
            clone_url: String::new(),
            clone_error: None,
            clone_input_focused: false,
        }
    }
}

impl GitUiState {
    pub fn reset_scroll(&mut self) {
        self.scroll = 0.0;
        self.details_scroll = 0.0;
    }
}

#[derive(Clone, Default)]
pub struct GitPanelState {
    pub model: GitModel,
    pub ui: GitUiState,
}
