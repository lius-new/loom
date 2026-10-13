//! Content search feature. No dependency on the application or GUI.
pub mod matcher;
pub mod model;
pub mod ports;
pub mod service;
pub mod types;
pub use self::matcher::{Matcher, hash};
pub use self::service::{MAX_FILE_BYTES, MAX_MATCHES};
pub use self::types::*;

pub mod navigation;

#[cfg(test)]
pub(crate) mod tests;
