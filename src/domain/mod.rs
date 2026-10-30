//! Framework-free types. Existing model transactions migrate in later phases.
pub(crate) mod notification;

mod dirty;
pub mod document;
pub(crate) mod document_snapshot;
pub mod encoding;
pub mod geometry;
pub(crate) mod identity;
pub(crate) mod language;
pub(crate) mod line_change;
pub mod media;
pub mod pane_layout;
pub mod view;
pub mod workspace;
pub mod workspace_session;

pub mod application_layout;

pub mod window_geometry;
