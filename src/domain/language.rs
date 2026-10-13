//! Language identity and names without presentation or disk access.
use std::path::Path;

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
}
