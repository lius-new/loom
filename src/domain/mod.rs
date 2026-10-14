//! Framework-free types. Existing model transactions migrate in later phases.
pub(crate) mod notification;

pub mod document;
pub(crate) mod document_snapshot;
pub mod geometry;
pub(crate) mod identity;
pub(crate) mod language;
pub(crate) mod line_change;
pub mod pane_layout;
pub mod workspace;
pub mod workspace_session;

pub mod application_layout;

pub mod window_geometry;
