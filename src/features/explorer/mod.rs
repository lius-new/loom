//! Explorer feature: directory listings, workspace roots and naming rules.
//! No dependency on the application, GUI or filesystem; disk access is
//! injected through `ports`.
pub mod edit;
pub mod ports;
pub mod roots;
pub mod tree;
pub mod types;
pub use self::types::*;

#[cfg(test)]
pub(crate) mod tests;
