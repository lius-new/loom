//! Runtime document identity and metadata.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use lgui::prelude::Color;

use crate::theme;

pub use crate::domain::language::Language;

impl Language {
    pub fn badge_color(self) -> Color {
        match self {
            Self::CSharp => theme::c().badge.csharp,
            Self::Rust => theme::c().badge.rust,
            Self::TypeScript => theme::c().badge.typescript,
            Self::JavaScript => theme::c().badge.javascript,
            Self::PlainText => theme::c().text_muted,
        }
    }
}

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

impl DiskState {
    pub fn capture(path: &Path, contents: &str) -> Self {
        let metadata = std::fs::metadata(path).ok();
        Self {
            modified_time: metadata.as_ref().and_then(|value| value.modified().ok()),
            size: metadata
                .as_ref()
                .map_or(contents.len() as u64, std::fs::Metadata::len),
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
    use super::*;

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
