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
pub mod sidebar;
pub mod statusbar;
pub mod tab_context_menu;
pub mod tab_layout;
pub mod tabs;
pub mod terminal;
pub mod titlebar;
pub mod toast;
pub mod welcome;
