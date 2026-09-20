//! leditor — a modular text editor port of the "Aura Code" mockup.
//!
//! Layer map:
//!   app      — root component, layout & composition
//!   state    — reactive AppState (the single shared model)
//!   theme    — design tokens (colors, metrics, text styles)
//!   model/   — pure data (buffer, document, workspace)
//!   editor/  — code surface (syntax, gutter, viewport)
//!   ui/      — chrome components (titlebar, sidebar, tabs, …)
//!   input/   — keymap: raw events → semantic actions
#![allow(dead_code)] // the data layer & palette expose an intentionally broad API

mod app;
mod editor;
mod input;
mod model;
mod state;
mod theme;
mod ui;

use lgui::icons::SvgIconRegistry;
use lgui::prelude::*;
use lgui::WinitApplication;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::with_backend(WinitApplication::new(GraphicsPreference::Auto))
        .svg_icons(
            SvgIconRegistry::new()
                .with_icon("panel-left", icondata::LuPanelLeft)
                .with_icon("git-branch", icondata::LuGitBranch)
                .with_icon("search", icondata::LuSearch)
                .with_icon("command", icondata::LuCommand)
                .with_icon("play", icondata::LuPlay)
                .with_icon("sparkles", icondata::LuSparkles)
                .with_icon("square", icondata::LuSquare)
                .with_icon("close", icondata::LuX)
                .with_icon("minus", icondata::LuMinus),
        )
        .provide(RendererKind::Skia(GraphicsPreference::Auto))
        .memory_options(MemoryOptions::unbounded(ImageCachePolicy::WhileVisible, false))
        .window_options(
            WindowOptions::new("leditor")
                .title("leditor")
                .size(Size::new(900.0, 600.0))
                // Frameless: we render our own custom title bar (see ui/titlebar.rs).
                .native_titlebar(false),
        )
        .run(app::app)?;
    Ok(())
}
