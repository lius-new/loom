//! Design tokens for the "Aura Code" editor (ported from code-preview.html).
//!
//! Centralizes colors, layout metrics and text-style helpers so every UI
//! module renders from a single source of truth.

use lgui::prelude::{Color, Stroke, TextAlign, TextStyle};

// ---- Editor chrome -----------------------------------------------------
pub const BG: Color = Color(0x14161b); // primary editor background
pub const SIDEBAR: Color = Color(0x111216); // slightly darker panel
pub const SURFACE: Color = Color(0x1c1f26); // elevated cards / inputs
pub const BORDER: Color = Color(0x262a35); // divider borders
pub const ACTIVE_LINE: Color = Color(0x1a1d24); // active row highlight
pub const SELECTION: Color = Color(0x273449);
pub const ACCENT: Color = Color(0x818cf8); // electric indigo
pub const ACCENT_HOVER: Color = Color(0x6366f1);

// ---- Syntax palette ----------------------------------------------------
pub const KW: Color = Color(0xf472b6); // keywords (pink)
pub const FUNC: Color = Color(0x60a5fa); // functions (blue)
pub const TYPE: Color = Color(0xfbbf24); // classes & structs (amber)
pub const STR: Color = Color(0x4ade80); // strings (emerald)
pub const COMMENT: Color = Color(0x64748b); // comments (slate)
pub const NUM: Color = Color(0xfb923c); // numbers (orange)
pub const PROP: Color = Color(0xa78bfa); // properties (purple)

// ---- Zinc ramp (neutral text) -----------------------------------------
pub const ZINC_100: Color = Color(0xf4f4f5);
pub const ZINC_200: Color = Color(0xe4e4e7);
pub const ZINC_300: Color = Color(0xd4d4d8);
pub const ZINC_400: Color = Color(0xa1a1aa);
pub const ZINC_500: Color = Color(0x71717a);
pub const ZINC_600: Color = Color(0x52525b);
pub const ZINC_700: Color = Color(0x3f3f46);
pub const ZINC_800: Color = Color(0x27272a);

// ---- Accent hues -------------------------------------------------------
pub const PURPLE_400: Color = Color(0xc084fc);
pub const ORANGE_400: Color = Color(0xfb923c);
pub const BLUE_400: Color = Color(0x60a5fa);
pub const EMERALD_400: Color = Color(0x34d399);
pub const EMERALD_500: Color = Color(0x10b981);
pub const EMERALD_600: Color = Color(0x059669);
pub const ROSE_400: Color = Color(0xfb7185);
pub const ROSE_500: Color = Color(0xf43f5e);
pub const AMBER_400: Color = Color(0xfbbf24);
pub const AMBER_500: Color = Color(0xf59e0b);

// ---- Git diff / ghost --------------------------------------------------
pub const DIFF_DEL_BG: Color = Color(0x2f1b23); // rgba(244,63,94,.12) over BG
pub const DIFF_DEL_BORDER: Color = Color(0xf43f5e);
pub const DIFF_DEL_FG: Color = Color(0xfda4af);
pub const DIFF_ADD_BG: Color = Color(0x142a27); // rgba(16,185,129,.12) over BG
pub const DIFF_ADD_BORDER: Color = Color(0x10b981);
pub const DIFF_ADD_FG: Color = Color(0x6ee7b7);
pub const GHOST: Color = Color(0x64748b);

// ---- Layout metrics (px) ----------------------------------------------
pub const TITLEBAR_H: f32 = 32.0;
pub const TABS_H: f32 = 32.0;
pub const STATUS_H: f32 = 24.0;
pub const SIDEBAR_W: f32 = 224.0;
pub const ASSISTANT_W: f32 = 320.0;
pub const TERMINAL_H: f32 = 176.0;
pub const GUTTER_W: f32 = 48.0;
pub const LINE_H: f32 = 24.0;
pub const CODE_PAD: f32 = 16.0;
pub const CHAR_W: f32 = 7.2; // monospace advance at CODE_SIZE

// ---- Font sizes --------------------------------------------------------
pub const CODE_SIZE: f32 = 12.0;
pub const UI_SIZE: f32 = 12.0;
pub const SMALL: f32 = 10.0;

// ---- Text-style helpers ------------------------------------------------
pub fn mono(color: Color, size: f32) -> TextStyle {
    TextStyle::new(color, size, 400)
}

pub fn mono_bold(color: Color, size: f32) -> TextStyle {
    TextStyle::new(color, size, 700)
}

pub fn mono_right(color: Color, size: f32) -> TextStyle {
    let mut s = TextStyle::new(color, size, 400);
    s.align = TextAlign::Right;
    s
}

pub fn sans(color: Color, size: f32) -> TextStyle {
    TextStyle::new(color, size, 400)
}

pub fn sans_semibold(color: Color, size: f32) -> TextStyle {
    TextStyle::new(color, size, 600)
}

pub fn hairline(color: Color) -> Stroke {
    Stroke::new(color, 1.0, 255)
}
