//! Private composition root. Only bootstrap and the registered state bridge escape.
mod bootstrap;
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
