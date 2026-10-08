//! Data layer: pure models with no UI dependencies.
//!
//! * `text`      — the editing core: text, edits, selections and history.
//! * `document`  — static file metadata + language + sample sources.
//! * `pane_layout` — the split tree that arranges editor panes.
//! * `workspace` — open documents and the panes that show them.

pub mod diff_document;
pub mod document;
pub mod pane_layout;
pub mod text;
pub mod workspace;
