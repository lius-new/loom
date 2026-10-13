//! Search receives disk access through this injected interface.
use super::{Matcher, SearchSpec};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct SourceEntry {
    pub path: PathBuf,
    pub is_file: bool,
}
impl SourceEntry {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub trait SearchSourcePort: Send + Sync {
    fn normalize(&self, path: &Path) -> PathBuf;
    fn is_directory(&self, path: &Path) -> bool;
    fn read_text(&self, path: &Path) -> Result<String, String>;
    fn walk(
        &self,
        root: &Path,
        spec: &SearchSpec,
        matcher: Arc<Matcher>,
    ) -> Box<dyn Iterator<Item = Result<SourceEntry, String>> + '_>;
}
