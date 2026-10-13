//! Explorer pointer, menu, inline-edit input and viewport state; owned
//! separately from the business model.
use crate::features::explorer::ExplorerContextTarget;
use crate::features::explorer::model::ExplorerModel;
use crate::ui::components::input::InputState;

#[derive(Clone, Default)]
pub struct ExplorerUiState {
    /// Pointer is over the file drawer (shows its scrollbars).
    pub hovered: bool,
    /// Cache key of the tree row under the pointer.
    pub hovered_path: Option<String>,
    /// Open context menu anchor in window coordinates.
    pub menu: Option<(f32, f32)>,
    /// Row the menu was opened on; `None` acts on the containing root.
    pub menu_target: Option<ExplorerContextTarget>,
    pub menu_hover: Option<usize>,
    /// Text of the inline create/rename row.
    pub create_input: InputState,
    pub create_error: Option<String>,
    pub scroll: f32,
    pub scroll_x: f32,
    pub scrollbar_dragging: bool,
    pub scrollbar_drag_offset: f32,
    pub horizontal_scrollbar_dragging: bool,
    pub horizontal_scrollbar_drag_offset: f32,
}

impl ExplorerUiState {
    pub fn dismiss_menu(&mut self) {
        self.menu = None;
        self.menu_target = None;
        self.menu_hover = None;
    }

    /// Reset the inline row after its request opened, closed or failed.
    pub fn clear_edit(&mut self) {
        self.create_input.clear();
        self.create_error = None;
    }

    pub fn reset_scroll(&mut self) {
        self.scroll = 0.0;
        self.scroll_x = 0.0;
    }
}

#[derive(Clone, Default)]
pub struct ExplorerPanelState {
    pub model: ExplorerModel,
    pub ui: ExplorerUiState,
}
