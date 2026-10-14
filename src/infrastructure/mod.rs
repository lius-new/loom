//! Platform/framework adapters, composed by app.
pub(crate) mod ui_dispatch;

pub(crate) mod explorer_filesystem;
pub(crate) mod git;
pub(crate) mod search_filesystem;
pub(crate) mod session_filesystem;
pub(crate) mod workspace_filesystem;

pub(crate) mod terminal;

pub mod config_watcher;
pub mod project_settings;
pub mod settings_filesystem;
