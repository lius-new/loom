//! Shared immutable text snapshots, independent of consumers such as Search.
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
#[derive(Clone)]
pub struct DocumentSnapshot {
    pub path: PathBuf,
    pub text: Arc<String>,
    pub version: u64,
}
pub fn content_hash(contents: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    contents.hash(&mut hasher);
    hasher.finish()
}
