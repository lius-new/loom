//! Loom — a modular text editor port of the "Aura Code" mockup.
//!
//! Layer map:
//!   app      — root component, layout & composition
//!   state    — reactive AppState (the single shared model)
//!   theme    — design tokens (colors, metrics, text styles)
//!   model/   — pure data (buffer, document, workspace)
//!   editor/  — code surface (syntax, gutter, viewport)
//!   ui/      — chrome components (titlebar, sidebar, tabs, …)
//!   input/   — keymap: raw events → semantic actions
// Keep debug builds attached to the launching console so `cargo run` can be
// stopped with Ctrl+C. Packaged release builds remain windowed and do not open
// a console window.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]
#![allow(dead_code)] // the data layer & palette expose an intentionally broad API

mod app;
mod editor;
mod file_icons;
mod git;
mod git_actions;
mod input;
mod model;
mod state;
mod terminal_session;
mod theme;
mod ui;
mod window_geometry;
mod workspace_actions;
mod workspace_persistence;

#[cfg(not(target_os = "windows"))]
use image::imageops::FilterType;
use lgui::icons::SvgIconRegistry;
use lgui::prelude::*;
use lgui::{WinitApplication, WinitWindowIcon, WinitWindowOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let native_window_options = native_window_options()?;
    let window_options = window_geometry::restore_options(
        WindowOptions::new("loom")
            .title("Loom")
            .size(Size::new(900.0, 600.0))
            // Frameless: we render our own custom title bar (see ui/titlebar.rs).
            .native_titlebar(false)
            // Windows 11 DWM rounded corners.
            .corner_radius(8)
            .on_close_requested(window_geometry::handle_close_requested)
            .with_platform_options(native_window_options),
    );

    Application::with_backend(WinitApplication::new(GraphicsPreference::Auto))
        .svg_icons(file_icons::register(
            SvgIconRegistry::new()
                .with_icon("panel-left", icondata::LuPanelLeft)
                .with_icon("git-branch", icondata::LuGitBranch)
                .with_icon("check", icondata::LuCheck)
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
                .with_icon("explorer", icondata::LuLayers)
                .with_icon("chevron-right", icondata::LuChevronRight)
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
        .window_options(window_options)
        .run(app::app)?;
    Ok(())
}

fn native_window_options() -> Result<WinitWindowOptions, Box<dyn std::error::Error>> {
    #[cfg(target_os = "windows")]
    {
        // Match the embedded multi-size executable icon exactly. Winit's runtime
        // RGBA-to-HICON conversion renders differently from the PE icon resource.
        let options = WinitWindowOptions::new().window_icon(icon_from_ico_frame(16)?);
        return Ok(options.taskbar_icon(icon_from_ico_frame(32)?));
    }

    #[cfg(not(target_os = "windows"))]
    {
        let source = image::load_from_memory_with_format(
            include_bytes!("../icons/source/app-icon-transparent-1024.png"),
            image::ImageFormat::Png,
        )?
        .into_rgba8();
        let window_pixels = image::imageops::resize(&source, 32, 32, FilterType::Lanczos3);
        let window_icon = WinitWindowIcon::from_rgba(window_pixels.into_raw(), 32, 32)?;
        Ok(WinitWindowOptions::new().window_icon(window_icon))
    }
}

#[cfg(target_os = "windows")]
fn icon_from_ico_frame(size: u32) -> Result<WinitWindowIcon, Box<dyn std::error::Error>> {
    let ico = include_bytes!("../icons/app.ico");
    let entry_count = u16::from_le_bytes([ico[4], ico[5]]) as usize;

    for index in 0..entry_count {
        let entry = 6 + index * 16;
        if entry + 16 > ico.len() {
            break;
        }
        let width = if ico[entry] == 0 {
            256
        } else {
            u32::from(ico[entry])
        };
        let height = if ico[entry + 1] == 0 {
            256
        } else {
            u32::from(ico[entry + 1])
        };
        if width != size || height != size {
            continue;
        }

        let image_length = u32::from_le_bytes([
            ico[entry + 8],
            ico[entry + 9],
            ico[entry + 10],
            ico[entry + 11],
        ]) as usize;
        let image_offset = u32::from_le_bytes([
            ico[entry + 12],
            ico[entry + 13],
            ico[entry + 14],
            ico[entry + 15],
        ]) as usize;
        let image_end = image_offset
            .checked_add(image_length)
            .filter(|end| *end <= ico.len())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid ICO frame")
            })?;
        let pixels = image::load_from_memory_with_format(
            &ico[image_offset..image_end],
            image::ImageFormat::Png,
        )?
        .into_rgba8();
        return Ok(WinitWindowIcon::from_rgba(pixels.into_raw(), size, size)?);
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("Missing {size}x{size} PNG frame in icons/app.ico"),
    )
    .into())
}
