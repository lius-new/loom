//! Terminal dock (static demo content matching the mockup).

use lgui::prelude::{panel, text, Element, UiRect, VisualStyle};

use crate::theme;

pub fn render(rect: UiRect) -> Element {
    let mut p = panel(rect, VisualStyle::filled(theme::SIDEBAR));

    // Header bar
    let hdr = UiRect::new(rect.left, rect.top, rect.right, rect.top + 32.0);
    p = p.child(panel(hdr, VisualStyle::filled(theme::BG)));
    p = p.child(text(
        UiRect::new(rect.left + 14.0, rect.top + 8.0, rect.left + 120.0, rect.top + 24.0),
        "❯ Quick Terminal",
        theme::mono_bold(theme::ACCENT, theme::SMALL),
    ));
    p = p.child(text(
        UiRect::new(rect.left + 124.0, rect.top + 8.0, rect.left + 240.0, rect.top + 24.0),
        "zsh (node v22.12.0)",
        theme::mono(theme::ZINC_500, theme::SMALL),
    ));
    p = p.child(text(
        UiRect::new(rect.right - 100.0, rect.top + 8.0, rect.right - 48.0, rect.top + 24.0),
        "Clear",
        theme::mono(theme::ZINC_400, theme::SMALL),
    ));
    p = p.child(text(
        UiRect::new(rect.right - 36.0, rect.top + 8.0, rect.right - 16.0, rect.top + 24.0),
        "✕",
        theme::mono(theme::ZINC_500, theme::SMALL),
    ));

    // Log lines
    let logs = [
        "Aura Runtime Subsystem initialized.",
        "➜ GameServer dotnet test --filter Category=Unit",
        "Passed! - Failed: 0, Passed: 14, Skipped: 0, Total: 14, Duration: 240ms.",
    ];
    for (i, line) in logs.iter().enumerate() {
        let y = rect.top + 44.0 + i as f32 * 20.0;
        let color = if i == 1 { theme::ZINC_200 } else { theme::ZINC_500 };
        p = p.child(text(
            UiRect::new(rect.left + 14.0, y, rect.right - 14.0, y + 20.0),
            *line,
            theme::mono(color, theme::SMALL),
        ));
    }

    // CLI input
    let inp = UiRect::new(rect.left + 14.0, rect.bottom - 26.0, rect.right - 14.0, rect.bottom - 6.0);
    p = p.child(text(
        UiRect::new(inp.left, inp.top, inp.right, inp.bottom),
        "➜  run command...",
        theme::mono(theme::ZINC_400, theme::SMALL),
    ));

    p
}
