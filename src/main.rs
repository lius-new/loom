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
mod file_icons;
mod input;
mod model;
mod state;
mod terminal_session;
mod theme;
mod ui;
mod workspace_actions;

use lgui::WinitApplication;
use lgui::icons::SvgIconRegistry;
use lgui::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::with_backend(WinitApplication::new(GraphicsPreference::Auto))
        .svg_icons(file_icons::register(
            SvgIconRegistry::new()
                .with_icon("panel-left", icondata::LuPanelLeft)
                .with_icon("git-branch", icondata::LuGitBranch)
                .with_icon("search", icondata::LuSearch)
                .with_icon("command", icondata::LuCommand)
                .with_icon("play", icondata::LuPlay)
                .with_icon("terminal", icondata::LuTerminal)
                .with_icon("plus", icondata::LuPlus)
                .with_icon("chevron-down", icondata::LuChevronDown)
                .with_icon("split-terminal", icondata::LuColumns2)
                .with_icon("trash", icondata::LuTrash2)
                .with_icon("maximize", icondata::LuMaximize2)
                .with_icon("square", icondata::LuSquare)
                // X strokes: a 1px round cap (radius 0.5px) lands between pixel centers,
                // leaving the tips faint. Use butt caps and extend the endpoints outward
                // so each diagonal terminates on a solid pixel.
                .with_icon(
                    "close",
                    r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="butt" stroke-linejoin="round" opacity="currentOpacity"><path d="M19.5 4.5 4.5 19.5"/><path d="m4.5 4.5 15 15"/></svg>"#,
                )
                // y=11 (not 12) so the 1px stroke snaps to a single pixel row at 12px size.
                .with_icon(
                    "minus",
                    r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" opacity="currentOpacity"><path d="M5 11h14"/></svg>"#,
                ),
        ))
        .provide(RendererKind::Skia(GraphicsPreference::Auto))
        .memory_options(MemoryOptions::unbounded(ImageCachePolicy::WhileVisible, false))
        .window_options(
            WindowOptions::new("leditor")
                .title("leditor")
                .size(Size::new(900.0, 600.0))
                // Frameless: we render our own custom title bar (see ui/titlebar.rs).
                .native_titlebar(false)
                // Windows 11 DWM rounded corners.
                .corner_radius(8),
        )
        .run(app::app)?;
    Ok(())
}
