//! Every grammar is provided through the demand-loaded Resources adapter.
use std::sync::Arc;
use tree_sitter::{Language, Parser};

/// Languages must remain usable by parsers, trees and queries after this call.
/// Native adapters retain module handles; WASM languages use reference counting.
/// Called by background
/// syntax work; this port has no filesystem or application dependencies.
pub(crate) trait Resources: Send + Sync {
    fn language(&self, grammar: &str) -> Result<Language, String>;
    /// The adapter supplies any execution store needed by this language. Each
    /// parser owns its mutable store; trees and queries can share the language.
    fn parser(&self, grammar: &str) -> Result<Parser, String> {
        let language = self.language(grammar)?;
        let mut parser = Parser::new();
        parser.set_language(&language).map_err(|e| e.to_string())?;
        Ok(parser)
    }
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
