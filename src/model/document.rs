//! Runtime document identity and metadata.

use std::path::{Path, PathBuf};

use lgui::prelude::Color;

use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Language {
    CSharp,
    Rust,
    TypeScript,
    JavaScript,
    PlainText,
}

impl Language {
    pub fn from_path(path: &Path) -> Self {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "cs" => Self::CSharp,
            "rs" => Self::Rust,
            "ts" | "tsx" | "mts" | "cts" => Self::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Self::JavaScript,
            _ => Self::PlainText,
        }
    }

    pub fn badge(self) -> &'static str {
        match self {
            Self::CSharp => "C#",
            Self::Rust => "RS",
            Self::TypeScript => "TS",
            Self::JavaScript => "JS",
            Self::PlainText => "TXT",
        }
    }

    pub fn long_name(self) -> &'static str {
        match self {
            Self::CSharp => "C#",
            Self::Rust => "Rust",
            Self::TypeScript => "TypeScript",
            Self::JavaScript => "JavaScript",
            Self::PlainText => "Plain Text",
        }
    }

    pub fn badge_color(self) -> Color {
        match self {
            Self::CSharp => theme::PURPLE_400,
            Self::Rust => theme::ORANGE_400,
            Self::TypeScript => theme::BLUE_400,
            Self::JavaScript => theme::AMBER_400,
            Self::PlainText => theme::ZINC_400,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FileId(u64);

impl FileId {
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug)]
pub struct FileMeta {
    pub name: String,
    pub lang: Language,
    pub path: PathBuf,
}

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
