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
pub mod settings;
pub mod sidebar;
pub mod statusbar;
pub mod tab_context_menu;
pub mod tab_layout;
pub mod tabs;
pub mod terminal;
pub mod titlebar;
pub mod toast;
pub mod welcome;
pub mod workspace_home;

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
