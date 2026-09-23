//! UI layer: chrome components, each rendering a self-contained region.
//!
//! Every module exposes a `render(rect, …)` function returning an `Element`.
//! Components read `AppState` through `State<AppState>` and never talk to
//! each other directly — the root (`app.rs`) owns layout and composition.

pub mod command_palette;
pub mod context_menu;
pub mod sidebar;
pub mod statusbar;
pub mod tabs;
pub mod terminal;
pub mod titlebar;
pub mod toast;
pub mod welcome;
