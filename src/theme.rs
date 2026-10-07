//! Design tokens for the "Aura Code" editor (ported from code-preview.html).
//!
//! Centralizes color palettes, layout metrics and text-style helpers so every UI
//! module renders from a single source of truth.

use std::sync::atomic::{AtomicUsize, Ordering};

use lgui::prelude::{Color, Element, Stroke, TextAlign, TextStyle, UiRect, VisualStyle, panel};

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
    pub scrollbar: Color,

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

/// Change-kind letters in the Source Control panel.
#[derive(Clone, Copy, Debug)]
pub struct GitColors {
    pub added: Color,
    pub modified: Color,
    pub deleted: Color,
    pub staged: Color,
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

pub const DEFAULT_THEME: &str = "aura-dark";

pub static THEMES: &[Theme] = &[Theme {
    id: "aura-dark",
    name: "Aura Dark",
    palette: AURA_DARK,
}];

const AURA_DARK: Palette = Palette {
    bg: Color(0x14161b),
    sidebar: Color(0x111216),
    surface: Color(0x1c1f26),
    border: Color(0x262a35),
    active_line: Color(0x1a1d24),
    selection: Color(0x273449),
    accent: Color(0x818cf8), // electric indigo
    accent_hover: Color(0x6366f1),
    scrollbar: Color(0x27272a),

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

static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// The active palette.
pub fn c() -> &'static Palette {
    &THEMES[CURRENT.load(Ordering::Relaxed)].palette
}

/// Id of the active theme.
pub fn current_id() -> &'static str {
    THEMES[CURRENT.load(Ordering::Relaxed)].id
}

/// Switch to the theme with `id`. Returns `false` and keeps the current
/// theme when no built-in theme has that id.
pub fn set(id: &str) -> bool {
    match THEMES.iter().position(|theme| theme.id == id) {
        Some(index) => {
            CURRENT.store(index, Ordering::Relaxed);
            true
        }
        None => false,
    }
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
