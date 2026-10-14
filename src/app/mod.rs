//! Private composition root. Only bootstrap and the registered state bridge escape.
mod bootstrap;
pub(crate) mod editor;
#[cfg(test)]
mod editor_baseline_tests;
pub(crate) mod editor_keys;
pub(crate) mod editor_view;
pub(crate) mod editor_vim;
#[cfg(test)]
mod editor_vim_integration_tests;
pub(crate) mod explorer;
pub(crate) mod explorer_view;
pub(crate) mod git;
mod git_poll;
mod git_tasks;
#[cfg(test)]
mod phase0_baseline;
mod presentation;
pub(crate) mod runtime;
mod shell;
pub(crate) mod state;
#[cfg(test)]
pub(crate) mod test_support;

pub(crate) use bootstrap::run;

#[cfg(test)]
pub(crate) use bootstrap::svg_icons;

mod git_clone;
mod git_peek;
#[cfg(test)]
mod git_performance;
pub(crate) mod git_view;
#[cfg(test)]
mod git_view_tests;
pub(crate) mod search;
mod search_tasks;
pub(crate) mod workspace;
mod workspace_git;
pub(crate) mod workspace_restore;
pub(crate) mod workspace_session;

pub(crate) mod terminal;
pub(crate) mod terminal_effects;
#[cfg(test)]
mod terminal_profile_tests;
pub(crate) mod terminal_profiles;
pub(crate) mod terminal_runtime;

pub(crate) mod terminal_binding;
#[cfg(test)]
mod terminal_session_tests;

#[cfg(test)]
mod terminal_command_tests;

pub(crate) mod layout_map;
pub(crate) mod terminal_view;
#[cfg(test)]
mod terminal_view_tests;

#[cfg(test)]
mod terminal_performance;

#[cfg(test)]
mod settings_project_tests;

pub(crate) mod workspace_commands;

pub(crate) mod settings;

pub(crate) mod key_actions;
pub(crate) mod keymap_context;

pub(crate) mod keymap_view;

pub(crate) mod file_loading;

pub(crate) mod file_watcher;

pub(crate) mod window;
