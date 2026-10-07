//! Data layer: pure models with no UI dependencies.
//!
//! * `buffer`    — the editable text of a single open file.
//! * `document`  — static file metadata + language + sample sources.
//! * `pane_layout` — the split tree that arranges editor panes.
//! * `workspace` — open documents and the panes that show them.

pub mod buffer;
pub mod diff_document;
pub mod document;
pub mod pane_layout;
pub mod workspace;
