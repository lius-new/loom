//! Design tokens for the "Aura Code" editor (ported from code-preview.html).
//!
//! Centralizes color palettes, layout metrics and text-style helpers so every UI
//! module renders from a single source of truth.

use std::sync::atomic::{AtomicUsize, Ordering};

use lgui::prelude::{Color, Element, Stroke, TextAlign, TextStyle, UiRect, VisualStyle, panel};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

// ---- Palettes ----------------------------------------------------------

/// Every color the UI draws with. Built-in themes are `Palette` values in
/// `THEMES`; the active one is read through `c()`.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    // Editor chrome
    pub bg: Color,          // primary editor background
    pub sidebar: Color,     // slightly darker panel
    pub surface: Color,     // elevated cards / inputs
    pub border: Color,      // divider borders
    pub active_line: Color, // active row / hover highlight
    pub selection: Color,
    pub accent: Color,
    pub accent_hover: Color,
    pub on_accent: Color, // text and knobs drawn on an accent fill
    pub scrollbar: Color,
    pub shadow_alpha: u8, // opacity of popup drop shadows

    // Text, from most to least prominent
    pub text_bright: Color,
    pub text: Color,
    pub text_soft: Color,
    pub text_muted: Color,
    pub text_dim: Color,
    pub text_faint: Color,
    pub text_ghost: Color,

    // Status
    pub error: Color,
    pub error_text: Color,
    pub warning: Color,

    pub syntax: SyntaxColors,
    pub diff: DiffColors,
    pub git: GitColors,
    pub badge: BadgeColors,
    /// The 16 ANSI terminal colors, as `0xRRGGBB`.
    pub ansi: [u32; 16],
}

#[derive(Clone, Copy, Debug)]
pub struct SyntaxColors {
    pub keyword: Color,
    pub function: Color,
    pub type_name: Color,
    pub string: Color,
    pub comment: Color,
    pub number: Color,
    pub property: Color,
}

/// Diff rows and the editor gutter's change markers.
#[derive(Clone, Copy, Debug)]
pub struct DiffColors {
    pub add_bg: Color,
    pub add_fg: Color,
    pub add_mark: Color,
    pub del_bg: Color,
    pub del_fg: Color,
    pub del_mark: Color,
    pub mod_mark: Color,
}

/// Change-kind letters in Source Control and Explorer label colors.
#[derive(Clone, Copy, Debug)]
pub struct GitColors {
    pub added: Color,
    pub modified: Color,
    pub deleted: Color,
    pub staged: Color,
    /// Labels of git-ignored paths in the Explorer.
    pub ignored: Color,
}

/// Tab badges.
#[derive(Clone, Copy, Debug)]
pub struct BadgeColors {
    pub csharp: Color,
    pub rust: Color,
    pub typescript: Color,
    pub javascript: Color,
    pub diff: Color,
}

pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub palette: Palette,
}

/// Built-in themes; the first entry is the default.
pub static THEMES: &[Theme] = &[
    Theme {
        id: "aura-dark",
        name: "Aura Dark",
        palette: AURA_DARK,
    },
    Theme {
        id: "aura-light",
        name: "Aura Light",
        palette: AURA_LIGHT,
    },
    Theme {
        id: "ember",
        name: "Ember",
        palette: EMBER,
    },
    Theme {
        id: "high-contrast",
        name: "High Contrast",
        palette: HIGH_CONTRAST,
    },
];

const AURA_DARK: Palette = Palette {
    bg: Color(0x14161b),
    sidebar: Color(0x111216),
    surface: Color(0x1c1f26),
    border: Color(0x262a35),
    active_line: Color(0x1a1d24),
    selection: Color(0x273449),
    accent: Color(0x818cf8), // electric indigo
    accent_hover: Color(0x6366f1),
    on_accent: Color(0xf4f4f5),
    scrollbar: Color(0x27272a),
    shadow_alpha: 80,

    text_bright: Color(0xf4f4f5),
    text: Color(0xe4e4e7),
    text_soft: Color(0xd4d4d8),
    text_muted: Color(0xa1a1aa),
    text_dim: Color(0x71717a),
    text_faint: Color(0x52525b),
    text_ghost: Color(0x3f3f46),

    error: Color(0xf43f5e),
    error_text: Color(0xfda4af),
    warning: Color(0xfbbf24),

    syntax: SyntaxColors {
        keyword: Color(0xf472b6),
        function: Color(0x60a5fa),
        type_name: Color(0xfbbf24),
        string: Color(0x4ade80),
        comment: Color(0x64748b),
        number: Color(0xfb923c),
        property: Color(0xa78bfa),
    },
    diff: DiffColors {
        add_bg: Color(0x142a27), // rgba(16,185,129,.12) over bg
        add_fg: Color(0x6ee7b7),
        add_mark: Color(0x10b981),
        del_bg: Color(0x2f1b23), // rgba(244,63,94,.12) over bg
        del_fg: Color(0xfda4af),
        del_mark: Color(0xf43f5e),
        mod_mark: Color(0x60a5fa),
    },
    git: GitColors {
        added: Color(0x38bdf8),
        modified: Color(0xeab308),
        deleted: Color(0xf43f5e),
        staged: Color(0x34d399),
        ignored: Color(0x5f5f68),
    },
    badge: BadgeColors {
        csharp: Color(0xc084fc),
        rust: Color(0xfb923c),
        typescript: Color(0x60a5fa),
        javascript: Color(0xfbbf24),
        diff: Color(0xc084fc),
    },
    ansi: [
        0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, 0x666666,
        0xf14c4c, 0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xffffff,
    ],
};

const AURA_LIGHT: Palette = Palette {
    bg: Color(0xfafafa),
    sidebar: Color(0xf3f4f6),
    surface: Color(0xffffff),
    border: Color(0xe4e4e7),
    active_line: Color(0xeef0f4),
    selection: Color(0xdbe4ff),
    accent: Color(0x4f46e5),
    accent_hover: Color(0x4338ca),
    on_accent: Color(0xffffff),
    scrollbar: Color(0xd4d4d8),
    shadow_alpha: 30,

    text_bright: Color(0x09090b),
    text: Color(0x18181b),
    text_soft: Color(0x27272a),
    text_muted: Color(0x52525b),
    text_dim: Color(0x71717a),
    text_faint: Color(0xa1a1aa),
    text_ghost: Color(0xc4c4c8),

    error: Color(0xe11d48),
    error_text: Color(0xbe123c),
    warning: Color(0xb45309),

    syntax: SyntaxColors {
        keyword: Color(0xdb2777),
        function: Color(0x2563eb),
        type_name: Color(0xb45309),
        string: Color(0x15803d),
        comment: Color(0x64748b),
        number: Color(0xc2410c),
        property: Color(0x7c3aed),
    },
    diff: DiffColors {
        add_bg: Color(0xdef2eb), // rgba(16,185,129,.12) over bg
        add_fg: Color(0x047857),
        add_mark: Color(0x10b981),
        del_bg: Color(0xf9e4e7), // rgba(244,63,94,.12) over bg
        del_fg: Color(0xbe123c),
        del_mark: Color(0xf43f5e),
        mod_mark: Color(0x3b82f6),
    },
    git: GitColors {
        added: Color(0x0284c7),
        modified: Color(0xa16207),
        deleted: Color(0xe11d48),
        staged: Color(0x059669),
        ignored: Color(0xa1a1aa),
    },
    badge: BadgeColors {
        csharp: Color(0x9333ea),
        rust: Color(0xea580c),
        typescript: Color(0x2563eb),
        javascript: Color(0xca8a04),
        diff: Color(0x9333ea),
    },
    ansi: [
        0x000000, 0xcd3131, 0x00bc00, 0x949800, 0x0451a5, 0xbc05bc, 0x0598bc, 0x555555, 0x666666,
        0xcd3131, 0x14ce14, 0xb5ba00, 0x0451a5, 0xbc05bc, 0x0598bc, 0xa5a5a5,
    ],
};

/// Warm stone-and-ember dark theme.
const EMBER: Palette = Palette {
    bg: Color(0x1c1917),
    sidebar: Color(0x171412),
    surface: Color(0x262220),
    border: Color(0x332d29),
    active_line: Color(0x221e1b),
    selection: Color(0x44352a),
    accent: Color(0xfb923c),
    accent_hover: Color(0xf97316),
    on_accent: Color(0x1c1917),
    scrollbar: Color(0x2e2925),
    shadow_alpha: 90,

    text_bright: Color(0xfafaf9),
    text: Color(0xe7e5e4),
    text_soft: Color(0xd6d3d1),
    text_muted: Color(0xa8a29e),
    text_dim: Color(0x78716c),
    text_faint: Color(0x57534e),
    text_ghost: Color(0x44403c),

    error: Color(0xf87171),
    error_text: Color(0xfca5a5),
    warning: Color(0xfacc15),

    syntax: SyntaxColors {
        keyword: Color(0xfb7185),
        function: Color(0xfacc15),
        type_name: Color(0x5eead4),
        string: Color(0xa3e635),
        comment: Color(0x78716c),
        number: Color(0xc4b5fd),
        property: Color(0xfdba74),
    },
    diff: DiffColors {
        add_bg: Color(0x292e17), // rgba(132,204,22,.12) over bg
        add_fg: Color(0xbef264),
        add_mark: Color(0x84cc16),
        del_bg: Color(0x351e1c), // rgba(239,68,68,.12) over bg
        del_fg: Color(0xfca5a5),
        del_mark: Color(0xef4444),
        mod_mark: Color(0x60a5fa),
    },
    git: GitColors {
        added: Color(0x5eead4),
        modified: Color(0xfacc15),
        deleted: Color(0xf87171),
        staged: Color(0xa3e635),
        ignored: Color(0x67615c),
    },
    badge: BadgeColors {
        csharp: Color(0xc4b5fd),
        rust: Color(0xfb923c),
        typescript: Color(0x93c5fd),
        javascript: Color(0xfde047),
        diff: Color(0xc4b5fd),
    },
    ansi: [
        0x1c1917, 0xef4444, 0x84cc16, 0xeab308, 0x60a5fa, 0xc084fc, 0x2dd4bf, 0xd6d3d1, 0x57534e,
        0xf87171, 0xa3e635, 0xfacc15, 0x93c5fd, 0xd8b4fe, 0x5eead4, 0xfafaf9,
    ],
};

/// Black background, bright text and strong borders.
const HIGH_CONTRAST: Palette = Palette {
    bg: Color(0x000000),
    sidebar: Color(0x0a0a0a),
    surface: Color(0x141414),
    border: Color(0x6b6b6b),
    active_line: Color(0x1f1f1f),
    selection: Color(0x264f78),
    accent: Color(0x4cc2ff),
    accent_hover: Color(0x2aa7f0),
    on_accent: Color(0x000000),
    scrollbar: Color(0x5a5a5a),
    shadow_alpha: 80,

    text_bright: Color(0xffffff),
    text: Color(0xffffff),
    text_soft: Color(0xf0f0f0),
    text_muted: Color(0xd4d4d4),
    text_dim: Color(0xbdbdbd),
    text_faint: Color(0xa3a3a3),
    text_ghost: Color(0x8a8a8a),

    error: Color(0xff6b6b),
    error_text: Color(0xff9e9e),
    warning: Color(0xffd60a),

    syntax: SyntaxColors {
        keyword: Color(0xff7edb),
        function: Color(0x7cc7ff),
        type_name: Color(0xffd60a),
        string: Color(0x7dff8a),
        comment: Color(0x9ca3af),
        number: Color(0xffab5e),
        property: Color(0xd2a8ff),
    },
    diff: DiffColors {
        add_bg: Color(0x0f3a1e),
        add_fg: Color(0x8dff9e),
        add_mark: Color(0x3ddc84),
        del_bg: Color(0x4a1414),
        del_fg: Color(0xffb3b3),
        del_mark: Color(0xff5c5c),
        mod_mark: Color(0x7cc7ff),
    },
    git: GitColors {
        added: Color(0x7cc7ff),
        modified: Color(0xffd60a),
        deleted: Color(0xff6b6b),
        staged: Color(0x7dff8a),
        ignored: Color(0x8a8a8a),
    },
    badge: BadgeColors {
        csharp: Color(0xd2a8ff),
        rust: Color(0xffab5e),
        typescript: Color(0x7cc7ff),
        javascript: Color(0xffd60a),
        diff: Color(0xd2a8ff),
    },
    ansi: [
        0x000000, 0xff5c5c, 0x3ddc84, 0xffd60a, 0x5c9dff, 0xff7edb, 0x4fd8ff, 0xe5e5e5, 0x7f7f7f,
        0xff8080, 0x7dff8a, 0xffe866, 0x8cb8ff, 0xffa6ea, 0x8ce6ff, 0xffffff,
    ],
};

/// A built-in theme. Stored in `settings.json` as the theme's string id; an
/// unknown id loads as the default theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThemeId(usize); // index into THEMES; 0 is the default

impl ThemeId {
    pub fn all() -> impl Iterator<Item = Self> {
        (0..THEMES.len()).map(Self)
    }

    pub fn parse(id: &str) -> Option<Self> {
        THEMES.iter().position(|theme| theme.id == id).map(Self)
    }

    pub fn theme(self) -> &'static Theme {
        &THEMES[self.0]
    }
}

impl Serialize for ThemeId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.theme().id)
    }
}

impl<'de> Deserialize<'de> for ThemeId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        Ok(Self::parse(&id).unwrap_or_default())
    }
}

static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// The active palette.
pub fn c() -> &'static Palette {
    &current().theme().palette
}

pub fn current() -> ThemeId {
    ThemeId(CURRENT.load(Ordering::Relaxed))
}

pub fn set(id: ThemeId) {
    CURRENT.store(id.0, Ordering::Relaxed);
}

// ---- Layout metrics (px) ----------------------------------------------
pub const TITLEBAR_H: f32 = 32.0;
pub const TABS_H: f32 = 28.0;
pub const STATUS_H: f32 = 24.0;
pub const SIDEBAR_W: f32 = 288.0;
pub const SIDEBAR_MIN_W: f32 = 160.0;
pub const SIDEBAR_MAX_W: f32 = 400.0;
pub const TERMINAL_H: f32 = 176.0;
pub const TERMINAL_MIN_H: f32 = 120.0;
pub const TERMINAL_MAX_H: f32 = 480.0;
pub const EDITOR_MIN_H: f32 = 96.0;
pub const GUTTER_W: f32 = 48.0;
pub const LINE_H: f32 = 24.0;
pub const CODE_PAD: f32 = 16.0;
pub const CHAR_W: f32 = 7.2; // monospace advance at CODE_SIZE
/// UI glyph size. Codicons use a 16px grid, so any other size smears strokes.
pub const ICON_SIZE: f32 = 16.0;

/// An `ICON_SIZE` square sharing `slot`'s center.
pub fn icon_rect(slot: UiRect) -> UiRect {
    let left = (slot.left + slot.right - ICON_SIZE) / 2.0;
    let top = (slot.top + slot.bottom - ICON_SIZE) / 2.0;
    UiRect::new(left, top, left + ICON_SIZE, top + ICON_SIZE)
}
pub const MONO_FAMILIES: &[&str] = &["Cascadia Mono", "Cascadia Code", "Consolas", "Courier New"];

// ---- Font sizes --------------------------------------------------------
pub const CODE_SIZE: f32 = 12.0;
pub const UI_SIZE: f32 = 12.0;
pub const SMALL: f32 = 10.0;

// ---- Text-style helpers ------------------------------------------------
pub fn mono(color: Color, size: f32) -> TextStyle {
    TextStyle::new(color, size, 400).font_families(MONO_FAMILIES)
}

pub fn mono_bold(color: Color, size: f32) -> TextStyle {
    TextStyle::new(color, size, 700).font_families(MONO_FAMILIES)
}

pub fn mono_right(color: Color, size: f32) -> TextStyle {
    let mut s = TextStyle::new(color, size, 400).font_families(MONO_FAMILIES);
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

// ---- Surfaces ----------------------------------------------------------

/// A panel with a crisp `width`-px border, drawn as a filled ring.
///
/// WHY A FILLED RING INSTEAD OF `VisualStyle::stroked`?
/// Skia strokes are centered on the rect edge and anti-aliased. A 1px stroke
/// therefore smears half a pixel onto each side of the edge, and anti-aliasing
/// spreads the color across two pixel rows — so it renders as a blurry ~2px
/// line. Tuning the stroke width (0.5 / 0.75 / 1.0) only makes it fainter or
/// fatter; it can never land on exactly 1px.
///
/// Instead we paint an outer rect in `border` and an inner rect in `fill`,
/// inset by `width`, leaving a `width`-px ring. Filled rects are pixel-aligned
/// (the same technique as the 1px divider panels in the title/status bars), so
/// the border is exactly `width` px and stays crisp at any scale factor.
///
/// Use this anywhere you need a hairline border on a (possibly rounded)
/// surface. `radius` is the OUTER corner radius; the inner corner uses
/// `radius - width` so the ring thickness stays uniform through the corners.
pub fn bordered(rect: UiRect, fill: Color, border: Color, radius: f32, width: f32) -> Element {
    let outer = panel(rect, VisualStyle::filled(border).radius(radius));
    let inner_rect = UiRect::new(
        rect.left + width,
        rect.top + width,
        rect.right - width,
        rect.bottom - width,
    );
    outer.child(panel(
        inner_rect,
        VisualStyle::filled(fill).radius((radius - width).max(0.0)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_ids_are_unique() {
        for (index, theme) in THEMES.iter().enumerate() {
            assert!(
                THEMES[index + 1..].iter().all(|other| other.id != theme.id),
                "duplicate theme id {}",
                theme.id
            );
        }
    }

    #[test]
    fn default_theme_is_aura_dark() {
        assert_eq!(ThemeId::default().theme().id, "aura-dark");
    }

    #[test]
    fn theme_ids_round_trip_as_json_strings() {
        for id in ThemeId::all() {
            let json = serde_json::to_string(&id).unwrap();
            assert_eq!(json, format!("\"{}\"", id.theme().id));
            assert_eq!(serde_json::from_str::<ThemeId>(&json).unwrap(), id);
        }
    }

    #[test]
    fn unknown_theme_id_loads_as_the_default() {
        assert_eq!(ThemeId::parse("no-such-theme"), None);
        let decoded: ThemeId = serde_json::from_str("\"no-such-theme\"").unwrap();
        assert_eq!(decoded, ThemeId::default());
    }
}
