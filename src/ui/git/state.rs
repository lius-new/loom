//! Source Control view state: pointer, scrolling, the commit form, menus,
//! pickers and dialogs, and the user's view preferences. Business state
//! lives in `features::git::model`.
use std::collections::HashSet;

use crate::features::git::model::GitModel;
use crate::ui::components::input::InputState;
use crate::ui::git::branch_picker::BranchPicker;
use crate::ui::git::confirm::GitConfirm;
use crate::ui::git::menu::GitMenu;
use crate::ui::git::panel::{CommitCard, GitHover, TagCard};
use crate::ui::git::stash_switch::StashSwitchRequest;

#[derive(Clone)]
pub struct GitUiState {
    /// Pointer is inside the Source Control drawer.
    pub hovered: bool,
    /// The drawer row under the pointer, which shows its action buttons.
    pub hover: Option<GitHover>,
    pub scroll: f32,
    /// Independent scroll of the Worktrees/Stashes/Tags/Commits area.
    pub details_scroll: f32,
    /// User-selected height of the bottom sections; None keeps automatic sizing.
    pub details_height: Option<f32>,
    /// Pointer's offset from the divider while dragging it.
    pub details_resize_offset: Option<f32>,
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
    pub tag_input: InputState,
    /// The hovered history commit's card.
    pub commit_card: Option<CommitCard>,
    pub tag_card: Option<TagCard>,
    pub hover_sequence: u64,
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
            details_height: None,
            details_resize_offset: None,
            scrollbar_dragging: false,
            scrollbar_drag_offset: 0.0,
            details_scrollbar_dragging: false,
            details_scrollbar_drag_offset: 0.0,
            commit_input: InputState::default(),
            commit_amend: false,
            commit_signoff: false,
            expanded_repositories: HashSet::new(),
            collapsed_sections: [
                crate::ui::git::panel::WORKTREES,
                crate::ui::git::panel::STASHES,
                crate::ui::git::panel::TAGS,
                crate::ui::git::panel::HISTORY,
            ]
            .into_iter()
            .collect(),
            tree_view: false,
            split_diff: false,
            inline_blame: false,
            menu: None,
            confirm: None,
            tag_input: InputState::default(),
            commit_card: None,
            tag_card: None,
            hover_sequence: 0,
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
        self.hover = None;
        self.clear_hover_cards();
    }

    pub fn clear_hover_cards(&mut self) {
        self.commit_card = None;
        self.tag_card = None;
        self.hover_sequence = self.hover_sequence.wrapping_add(1);
    }
}

#[derive(Clone, Default)]
pub struct GitPanelState {
    pub model: GitModel,
    pub ui: GitUiState,
}
