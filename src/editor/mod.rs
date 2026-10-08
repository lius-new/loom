//! Editor layer: syntax highlighting, gutter and the code viewport.

#[cfg(test)]
mod baseline_tests;
pub mod commands;
pub mod edit_menu;
pub mod editor_view;
pub mod gutter;
pub mod interaction;
pub mod normal;
pub mod syntax;
pub mod vim_input;
#[cfg(test)]
mod vim_integration_tests;
