//! Terminal interaction and draft state; no session or process ownership.
use crate::ui::components::input::InputState;
use std::time::Instant;
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalTabContextMenuState {
    pub position: (f32, f32),
    pub target: crate::features::terminal::SessionId,
    pub kind: TerminalTabMenuKind,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalTabMenuKind {
    Actions,
    Purpose,
}
#[derive(Clone)]
pub struct ProfileEditor {
    pub open: bool,
    pub id: Option<u64>,
    pub name: InputState,
    pub program: InputState,
    pub args: InputState,
    pub error: Option<String>,
}
/// Live text-selection drag, kept so a timer can keep scrolling while the
/// pointer rests above or below the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionDrag {
    pub session: crate::features::terminal::SessionId,
    pub content: lgui::prelude::UiRect,
    pub point: lgui::core::Point,
    pub rows: u16,
    pub cols: u16,
}
#[derive(Clone, Default)]
pub struct TerminalUiState {
    /// Session under the pointer, used to reveal its overlay scrollbar.
    pub hovered_terminal: Option<crate::features::terminal::SessionId>,
    /// Session and pointer offset within the dragged scrollbar thumb.
    pub terminal_scrollbar_drag: Option<(crate::features::terminal::SessionId, f32)>,
    pub selecting_terminal: bool,
    pub selection_drag: Option<SelectionDrag>,
    pub terminal_focused: bool,
    pub terminal_shell_menu: bool,
    pub terminal_shell_scroll: usize,
    pub terminal_shell_selection: Option<crate::features::terminal::ShellKind>,
    pub terminal_workspace_root: Option<std::path::PathBuf>,
    pub terminal_workspace_picker: Option<WorkspacePicker>,
    pub terminal_tab_context_menu: Option<TerminalTabContextMenuState>,
    pub shell_editor: Option<ProfileEditor>,
    pub terminal_split_drag: Option<u64>,
    pub last_terminal_tab_click: Option<(Instant, u64, f32, f32, u8)>,
}
#[derive(Clone)]
pub struct WorkspacePicker {
    pub target: Option<crate::features::terminal::SessionId>,
    pub shell: Option<crate::features::terminal::ShellKind>,
    pub selected: usize,
    pub scroll: usize,
}
impl TerminalUiState {
    pub fn terminal_tab_click_count(&mut self, terminal: u64, x: f32, y: f32) -> u8 {
        next_click_count(&mut self.last_terminal_tab_click, terminal, x, y)
    }
}
fn next_click_count<K: PartialEq>(
    previous: &mut Option<(Instant, K, f32, f32, u8)>,
    key: K,
    x: f32,
    y: f32,
) -> u8 {
    let now = Instant::now();
    let count = previous
        .as_ref()
        .filter(|(time, previous_key, previous_x, previous_y, _)| {
            previous_key == &key
                && now.duration_since(*time).as_millis() < 500
                && (x - *previous_x).abs() < 5.0
                && (y - *previous_y).abs() < 5.0
        })
        .map_or(1, |(_, _, _, _, count)| count % 3 + 1);
    *previous = Some((now, key, x, y, count));
    count
}
