//! Runtime document identity and metadata.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub use crate::domain::language::Language;

pub use crate::domain::identity::FileId;

#[derive(Clone, Debug)]
pub struct FileMeta {
    pub name: String,
    pub lang: Language,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskState {
    pub modified_time: Option<SystemTime>,
    pub size: u64,
    pub content_hash: u64,
}

/// A filesystem result. Missing and unreadable are distinct observations.
#[derive(Clone, Debug)]
pub enum DiskRead {
    Found { contents: String, state: DiskState },
    Missing,
    Failed(String),
}

impl DiskState {
    pub fn from_contents(contents: &str) -> Self {
        Self {
            modified_time: None,
            size: contents.len() as u64,
            content_hash: content_hash(contents),
        }
    }

    pub fn differs_from(&self, other: &Self) -> bool {
        self.size != other.size
            || self.content_hash != other.content_hash
            || self.modified_time != other.modified_time
    }
}

pub use crate::domain::document_snapshot::content_hash;

impl FileMeta {
    pub fn from_path(path: PathBuf) -> Self {
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let lang = Language::from_path(&path);
        Self { name, lang, path }
    }

    pub fn directory_label(&self) -> String {
        self.path
            .parent()
            .and_then(Path::file_name)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::document::*;

    #[test]
    fn detects_supported_editor_languages_from_paths() {
        assert_eq!(Language::from_path(Path::new("main.RS")), Language::Rust);
        assert_eq!(
            Language::from_path(Path::new("component.tsx")),
            Language::TypeScript
        );
        assert_eq!(
            Language::from_path(Path::new("script.mjs")),
            Language::JavaScript
        );
        assert_eq!(
            Language::from_path(Path::new("README.md")),
            Language::PlainText
        );
    }
}
