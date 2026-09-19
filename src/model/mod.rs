//! Data layer: pure models with no UI dependencies.
//!
//! * `buffer`    — the editable text of a single open file.
//! * `document`  — static file metadata + language + sample sources.
//! * `workspace` — the set of open buffers and the active one.

pub mod buffer;
pub mod document;
pub mod workspace;
