//! UI layer: chrome components, each rendering a self-contained region.
//!
//! Every module exposes a `render(rect, …)` function returning an `Element`.
//! Components read `AppState` through `State<AppState>` and never talk to
//! each other directly — the root (`app.rs`) owns layout and composition.

pub mod clone_repository;
pub mod close_confirmation;
pub mod components;
pub mod context_menu;
pub mod diff_editor;
pub mod git_panel;
pub mod keymap_page;
pub mod pane_sash;
pub mod settings;
pub mod sidebar;
pub mod statusbar;
pub mod tab_context_menu;
pub mod tab_layout;
pub mod tabs;
pub mod terminal;
pub mod terminal_tab_context_menu;
pub mod titlebar;
pub mod toast;
pub mod welcome;
pub mod workspace_home;

/// These surfaces share an identity, so switching between them does not emit
/// another focus event. Track focus on every surface, including the home pages.
pub(crate) fn track_main_surface_focus(
    surface: lgui::prelude::Element,
    state: lgui::prelude::State<crate::state::AppState>,
) -> lgui::prelude::Element {
    let focus_state = state.clone();
    surface
        .on_focus(move |_| focus_state.update(|app| app.focused = true))
        .on_blur(move |_| {
            state.update(|app| {
                app.focused = false;
                app.editor.drag = None;
                app.editor.ime = None;
                if let Some(mut buffer) = app.workspace.active_editor_mut() {
                    buffer.break_undo_group();
                }
            });
        })
}

/// A path as shown to or copied by the user. `fs::canonicalize` yields
/// verbatim paths on Windows (`\\?\D:\dir`), so drop that prefix.
#[cfg(target_os = "windows")]
pub fn display_path(path: &std::path::Path) -> String {
    let value = path.to_string_lossy();
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(local) = value.strip_prefix(r"\\?\") {
        local.to_owned()
    } else {
        value.into_owned()
    }
}

#[cfg(not(target_os = "windows"))]
pub fn display_path(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}
