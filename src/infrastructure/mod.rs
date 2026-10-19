//! Platform/framework adapters, composed by app.
pub(crate) mod ui_dispatch;

pub(crate) mod explorer_filesystem;
pub(crate) mod git;
pub(crate) mod search_filesystem;
pub(crate) mod session_filesystem;
pub(crate) mod workspace_filesystem;

pub(crate) mod terminal;

pub mod config_watcher;
pub mod lsp;
pub mod project_settings;
pub mod settings_filesystem;

pub mod keymap_filesystem;

pub mod file_loader;

pub mod file_watcher;

pub mod document_filesystem;
pub mod file_dialogs;
pub mod file_drop;
pub(crate) mod language_packs;
pub(crate) mod syntax_worker;

pub mod launch;
pub(crate) mod update;
pub mod window_geometry;
