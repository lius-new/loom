//! Language badge colors are presentation policy.
use lgui::prelude::Color;

use crate::ui::theme;

use crate::domain::language::Language;

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
