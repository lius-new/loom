//! Every grammar is provided through the demand-loaded Resources adapter.
use std::sync::Arc;
use tree_sitter::Language;

/// Languages must remain usable by parsers, trees and queries after this call.
/// Native adapters retain module handles for the process. Called by background
/// syntax work; this port has no filesystem or application dependencies.
pub(crate) trait Resources: Send + Sync {
    fn language(&self, grammar: &str) -> Result<Language, String>;
    fn text(&self, grammar: &str, file: &str) -> Result<Option<Arc<str>>, String>;
    fn report(&self, _error: &str) {}
}

/// A detached index can still handle plain text without installed resources.
pub(crate) struct EmptyResources;
impl Resources for EmptyResources {
    fn language(&self, grammar: &str) -> Result<Language, String> {
        Err(format!("no language resources for {grammar}"))
    }
    fn text(&self, _grammar: &str, _file: &str) -> Result<Option<Arc<str>>, String> {
        Ok(None)
    }
}
