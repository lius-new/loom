//! Loom — a modular text editor port of the "Aura Code" mockup.
//!
//! Layer map:
//!   app      — root component, layout & composition
//!   app/state — legacy reactive model; field ownership is registered
//!   theme    — design tokens (colors, metrics, text styles)
//!   domain/  — framework-free types; model/ remains a tracked legacy layer
//!   editor/  — code surface (syntax, gutter, viewport)
//!   ui/      — chrome components (titlebar, sidebar, tabs, …)
//!   features/keymap — raw events → semantic actions (input alias during migration)
// Keep debug builds attached to the launching console so `cargo run` can be
// stopped with Ctrl+C. Test binaries also need a console for native PTY probes.
// Packaged release builds remain windowed and do not open a console window.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions), not(test)),
    windows_subsystem = "windows"
)]
#![allow(dead_code)] // the data layer & palette expose an intentionally broad API

mod app;
mod infrastructure;
use infrastructure::ui_dispatch as background_ui;
mod features;
mod file_loading;
mod file_watcher;
use features::keymap as input;
mod domain;
use app::key_actions;
mod launch;
mod model;
use app::state;
use app::workspace_restore as session_restore;
mod ui;
use loom_editor_core::vim;
mod window_geometry;
use app::workspace_commands as workspace_actions;
use app::workspace_session as workspace_persistence;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run()
}

#[cfg(test)]
use app::svg_icons;
