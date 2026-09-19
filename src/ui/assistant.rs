//! AI Assistant side panel (static demo content matching the mockup).

use lgui::prelude::{panel, text, Element, UiRect, VisualStyle};

use crate::theme;

pub fn render(rect: UiRect) -> Element {
    let mut p = panel(rect, VisualStyle::filled(theme::SIDEBAR));

    // Header
    let hdr = UiRect::new(rect.left, rect.top, rect.right, rect.top + 44.0);
    p = p.child(panel(hdr, VisualStyle::filled(theme::SIDEBAR)));
    p = p.child(panel(
        UiRect::new(rect.left + 14.0, rect.top + 12.0, rect.left + 30.0, rect.top + 28.0),
        VisualStyle::filled(theme::ACCENT).radius(6.0),
    ));
    p = p.child(text(
        UiRect::new(rect.left + 38.0, rect.top + 14.0, rect.right - 40.0, rect.top + 32.0),
        "✦ AI Assistant",
        theme::sans_semibold(theme::ZINC_200, theme::UI_SIZE),
    ));
    p = p.child(text(
        UiRect::new(rect.right - 30.0, rect.top + 14.0, rect.right - 16.0, rect.top + 32.0),
        "✕",
        theme::mono(theme::ZINC_500, theme::SMALL),
    ));

    // Context pills
    let mut y = rect.top + 52.0;
    let pill1 = UiRect::new(rect.left + 14.0, y, rect.left + 130.0, y + 24.0);
    p = p.child(panel(pill1, VisualStyle::filled(theme::SURFACE).radius(12.0)));
    p = p.child(text(
        UiRect::new(pill1.left + 10.0, y + 4.0, pill1.right, y + 20.0),
        "@WalletService.cs",
        theme::mono(theme::ZINC_300, theme::SMALL),
    ));
    let pill2 = UiRect::new(rect.left + 138.0, y, rect.left + 232.0, y + 24.0);
    p = p.child(panel(pill2, VisualStyle::filled(theme::SURFACE).radius(12.0)));
    p = p.child(text(
        UiRect::new(pill2.left + 10.0, y + 4.0, pill2.right, y + 20.0),
        "@RedisSchema",
        theme::mono(theme::ZINC_300, theme::SMALL),
    ));

    // Chat bubble
    y += 40.0;
    let bubble = UiRect::new(rect.left + 14.0, y, rect.right - 14.0, y + 118.0);
    p = p.child(panel(bubble, VisualStyle::filled(theme::SURFACE).radius(8.0)));
    p = p.child(text(
        UiRect::new(bubble.left + 12.0, y + 10.0, bubble.right - 12.0, y + 54.0),
        "I've indexed PlayerWalletService.cs. The balance modification currently lacks concurrency protection under high TPS.",
        theme::sans(theme::ZINC_300, theme::SMALL),
    ));

    // Code block
    let cb = UiRect::new(bubble.left + 12.0, y + 54.0, bubble.right - 12.0, y + 96.0);
    p = p.child(panel(cb, VisualStyle::filled(theme::BG).radius(6.0)));
    p = p.child(text(
        UiRect::new(cb.left + 10.0, y + 60.0, cb.right - 10.0, y + 76.0),
        "Proposed: RedLock lease",
        theme::mono_bold(theme::ZINC_200, theme::SMALL),
    ));
    p = p.child(text(
        UiRect::new(cb.left + 10.0, y + 76.0, cb.right - 10.0, y + 92.0),
        "+ await using var redLock = await AcquireLockAsync(...);",
        theme::mono(theme::EMERALD_400, theme::SMALL),
    ));
    p = p.child(text(
        UiRect::new(cb.right - 96.0, y + 100.0, cb.right - 12.0, y + 116.0),
        "1-Click Apply",
        theme::mono_bold(theme::ACCENT, theme::SMALL),
    ));

    // Input box
    let inp = UiRect::new(rect.left + 14.0, rect.bottom - 74.0, rect.right - 14.0, rect.bottom - 14.0);
    p = p.child(panel(inp, VisualStyle::filled(theme::SURFACE).radius(8.0)));
    p = p.child(text(
        UiRect::new(inp.left + 12.0, inp.top + 10.0, inp.right - 12.0, inp.top + 30.0),
        "Ask about architecture, write unit tests, or request refactoring...",
        theme::sans(theme::ZINC_500, theme::SMALL),
    ));
    p = p.child(text(
        UiRect::new(inp.left + 12.0, inp.top + 36.0, inp.left + 80.0, inp.bottom - 8.0),
        "@Codebase",
        theme::mono(theme::ACCENT, theme::SMALL),
    ));
    p = p.child(text(
        UiRect::new(inp.left + 86.0, inp.top + 36.0, inp.left + 154.0, inp.bottom - 8.0),
        "@Selection",
        theme::mono(theme::ACCENT, theme::SMALL),
    ));
    p = p.child(text(
        UiRect::new(inp.right - 52.0, inp.top + 36.0, inp.right - 12.0, inp.bottom - 8.0),
        "Send ↵",
        theme::mono_bold(theme::ACCENT, theme::SMALL),
    ));

    p
}
