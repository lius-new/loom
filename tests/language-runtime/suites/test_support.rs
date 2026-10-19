//! Test-only composition against the same disk resources as the application.
//! Production syntax code knows only the Resources port.
use super::{Resources, TreeIndex};
use crate::infrastructure::language_packs;
use std::sync::Arc;

pub(crate) fn resources() -> Arc<dyn Resources> {
    assert!(
        std::env::var_os("LOOM_TEST_LANGUAGE_PACKS").is_some(),
        "provide complete prebuilt language artifacts with LOOM_TEST_LANGUAGE_PACKS"
    );
    language_packs::resources()
}
pub(crate) fn index() -> TreeIndex {
    TreeIndex::new(resources())
}
pub(crate) fn names() -> Vec<String> {
    language_packs::resources().grammar_names()
}
pub(crate) fn language(name: &str) -> Option<tree_sitter::Language> {
    resources().language(name).ok()
}
pub(crate) fn grammar_file(name: &str, file: &str) -> Option<Arc<str>> {
    resources().text(name, file).unwrap()
}
pub(crate) fn compile(name: &str) -> Result<super::queries::CompiledQuery, String> {
    super::queries::compile(resources().as_ref(), name)
}
