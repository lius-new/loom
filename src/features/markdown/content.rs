//! Immutable host markup. Identity is assigned on ingestion, never by hashing a
//! document every frame; composed identities retain their constituent boundaries.
use super::model::ContentFormat;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Debug)]
pub struct Markup {
    pub identity: Arc<[u64]>,
    pub text: Arc<String>,
    pub format: ContentFormat,
}
impl PartialEq for Markup {
    fn eq(&self, other: &Self) -> bool {
        self.format == other.format && self.text == other.text
    }
}
impl Eq for Markup {}
impl PartialEq<str> for Markup {
    fn eq(&self, other: &str) -> bool {
        self.text.as_str() == other
    }
}
impl PartialEq<&str> for Markup {
    fn eq(&self, other: &&str) -> bool {
        self.text.as_str() == *other
    }
}
impl std::ops::Deref for Markup {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}
impl From<String> for Markup {
    fn from(value: String) -> Self {
        Self::new(value, ContentFormat::Markdown)
    }
}
impl From<&str> for Markup {
    fn from(value: &str) -> Self {
        Self::from(value.to_owned())
    }
}
impl Markup {
    pub fn new(text: String, format: ContentFormat) -> Self {
        Self {
            identity: vec![NEXT.fetch_add(1, Ordering::Relaxed)].into(),
            text: Arc::new(text),
            format,
        }
    }
    pub fn markdown(text: impl Into<String>) -> Self {
        Self::new(text.into(), ContentFormat::Markdown)
    }
    pub fn plain(text: impl Into<String>) -> Self {
        Self::new(text.into(), ContentFormat::PlainText)
    }
    pub fn code(text: &str, language: &str) -> Self {
        let fence = "\x60".repeat(
            text.split(|c| c != '\x60')
                .map(str::len)
                .max()
                .unwrap_or(0)
                .saturating_add(1)
                .max(3),
        );
        Self::markdown(format!(
            "{fence}{}\n{text}\n{fence}",
            language.replace(['\r', '\n', '\x60'], "")
        ))
    }
    pub fn join(parts: &[Self]) -> Option<Self> {
        let parts = parts
            .iter()
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>();
        if parts.len() == 1 {
            return Some(parts[0].clone());
        }
        if parts.is_empty() {
            return None;
        }
        let mut identity = Vec::new();
        let mut text = String::new();
        for part in parts {
            identity.push(u64::MAX);
            identity.push(part.identity.len() as u64);
            identity.extend(part.identity.iter().copied());
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            if part.format == ContentFormat::Markdown {
                text.push_str(&part.text);
            } else {
                for c in part.chars() {
                    if c.is_ascii_punctuation() {
                        text.push('\\');
                    }
                    if c == '\n' {
                        text.push_str("  \n");
                    } else if c != '\r' {
                        text.push(c);
                    }
                }
            }
        }
        Some(Self {
            identity: identity.into(),
            text: Arc::new(text),
            format: ContentFormat::Markdown,
        })
    }
}
