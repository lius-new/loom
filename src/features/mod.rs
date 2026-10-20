//! Feature entry points. Keymap internals retain the crate::input bridge until R8.
pub(crate) mod explorer;
pub(crate) mod git;
pub(crate) mod highlighting;
pub(crate) mod keymap;
pub(crate) mod language_service;
pub(crate) mod markdown;

pub(crate) mod search;

pub(crate) mod terminal;

pub mod settings;
pub(crate) mod syntax;
pub(crate) mod update;
