//! Shared editing algorithms, identity and behaviour without a GUI or I/O runtime.
//!
//! The host supplies view selections and clipboard/application capabilities.
//! Each edit still updates the document and its peer selections in one borrow.
pub mod commands;
pub mod identity;
pub mod normal;
pub mod text;
pub mod vim;
