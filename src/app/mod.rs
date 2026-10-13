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

pub(crate) mod search;
mod search_tasks;
