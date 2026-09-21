//! Empty-space context menu overlay for the file drawer.
//!
//! Right-clicking the empty area of the drawer opens a small menu of the
//! actions that make sense in that context: create new items, open a terminal,
//! or add folders to the project. Items are grouped by hairline separators.
//! Every action is a no-op for now — clicking an item simply closes the menu.

use lgui::core::{EventPolicy, UiEventKind};
use lgui::prelude::{panel, text, Color, Element, ShadowStyle, State, UiRect, VisualStyle};
use lgui::text::measure_width;

use crate::state::AppState;
use crate::theme;

const MENU_PAD: f32 = 4.0; // padding above/below the item list
const ITEM_H: f32 = 22.0; // item row height
const ITEM_PAD_X: f32 = 10.0; // horizontal padding inside an item
const SEP_H: f32 = 5.0; // separator row height
const SHORTCUT_GAP: f32 = 18.0; // min gap between a label and its shortcut
const TEXT_MARGIN: f32 = 6.0; // right margin so the last glyph is not clipped
const MENU_BORDER: f32 = 1.0; // menu border width

/// One row of the menu; `Separator` renders a hairline divider.
enum Entry {
    Item {
        label: &'static str,
        shortcut: Option<&'static str>,
    },
    Separator,
}

const ENTRIES: &[Entry] = &[
    Entry::Item { label: "New File", shortcut: None },
    Entry::Item { label: "New Folder", shortcut: None },
    Entry::Separator,
    Entry::Item { label: "Open in Terminal", shortcut: None },
    Entry::Separator,
    Entry::Item { label: "Add Folders to Project", shortcut: None },
];

/// Natural width of `s` at the menu text size, using the renderer's own text
/// system. Falls back to a per-character estimate when unavailable.
fn measure(s: &str) -> f32 {
    let bounds = UiRect::new(0.0, 0.0, 10_000.0, theme::UI_SIZE);
    measure_width(s, bounds, theme::UI_SIZE, 400).unwrap_or_else(|| {
        s.chars().count() as f32 * theme::CHAR_W * (theme::UI_SIZE / theme::CODE_SIZE)
    })
}

/// Renders the right-click menu anchored at `pos` (screen coords), flipped and
/// clamped so it stays inside `rect` (the viewport).
pub fn render(rect: UiRect, pos: (f32, f32), state: State<AppState>) -> Element {
    let s = state.get();

    // Size the menu to its widest entry (label + optional shortcut).
    let content_w = ENTRIES
        .iter()
        .map(|e| match e {
            Entry::Item { label, shortcut } => {
                let mut w = measure(label);
                if let Some(sc) = shortcut {
                    w += SHORTCUT_GAP + measure(sc);
                }
                w
            }
            Entry::Separator => 0.0,
        })
        .fold(0.0, f32::max);
    let menu_w = content_w + ITEM_PAD_X * 2.0 + TEXT_MARGIN;

    let menu_h = MENU_PAD * 2.0
        + ENTRIES
            .iter()
            .map(|e| match e {
                Entry::Item { .. } => ITEM_H,
                Entry::Separator => SEP_H,
            })
            .sum::<f32>();

    // Anchor at the cursor, flipping when it would overflow the window.
    let x = if pos.0 + menu_w <= rect.right {
        pos.0
    } else {
        pos.0 - menu_w
    };
    let y = if pos.1 + menu_h <= rect.bottom {
        pos.1
    } else {
        pos.1 - menu_h
    };
    let x = x.clamp(rect.left, (rect.right - menu_w).max(rect.left));
    let y = y.clamp(rect.top, (rect.bottom - menu_h).max(rect.top));
    let card = UiRect::new(x, y, x + menu_w, y + menu_h);

    // Invisible backdrop: a press outside the menu closes it. Presses inside
    // the menu are left to the items (so a click can both act and close).
    let st_close = state.clone();
    let overlay = panel(rect, VisualStyle::default())
        .event_policy(EventPolicy::INTERACTIVE)
        .on_pointer_down_with_button(move |_cx, p, _button| {
            if !card.contains(p.point) {
                st_close.update(move |app| {
                    app.context_menu = None;
                    app.context_menu_hover = None;
                });
            }
        });

    // Menu surface. A capture-phase move clears the hover highlight whenever
    // the pointer is over the surface but not on an item (padding, separators).
    let st_clear = state.clone();
    let mut surface = theme::bordered(card, theme::SURFACE, theme::BORDER, 6.0, MENU_BORDER)
        .shadow(ShadowStyle::new(Color::BLACK).alpha(80).offset(0.0, 2.0).blur(3.0))
        .event_policy(EventPolicy::INTERACTIVE)
        .on_event_capture(UiEventKind::PointerMove, move |_cx, _p| {
            st_clear.try_update(|app| {
                let changed = app.context_menu_hover.is_some();
                if changed {
                    app.context_menu_hover = None;
                }
                changed
            });
        });

    // Rows.
    let mut y = card.top + MENU_PAD;
    let mut index = 0usize;
    for entry in ENTRIES {
        match entry {
            Entry::Separator => {
                let line_y = y + (SEP_H - 1.0) / 2.0;
                surface = surface.child(panel(
                    UiRect::new(
                        card.left + ITEM_PAD_X,
                        line_y,
                        card.right - ITEM_PAD_X,
                        line_y + 1.0,
                    ),
                    VisualStyle::filled(theme::BORDER),
                ));
                y += SEP_H;
            }
            Entry::Item { label, shortcut } => {
                let item_rect = UiRect::new(card.left + MENU_BORDER, y, card.right - MENU_BORDER, y + ITEM_H);
                let hovered = s.context_menu_hover == Some(index);
                let idx = index;
                let label = *label;

                let st_hover = state.clone();
                let st_click = state.clone();
                let mut item = panel(
                    item_rect,
                    if hovered {
                        VisualStyle::filled(theme::SELECTION)
                    } else {
                        VisualStyle::default()
                    },
                )
                .event_policy(EventPolicy::INTERACTIVE)
                .on_event(UiEventKind::PointerMove, move |_cx, _p| {
                    st_hover.try_update(move |app| {
                        let changed = app.context_menu_hover != Some(idx);
                        if changed {
                            app.context_menu_hover = Some(idx);
                        }
                        changed
                    });
                })
                .on_click(move || {
                    st_click.update(move |app| {
                        app.context_menu = None;
                        app.context_menu_hover = None;
                    });
                });

                // Label (left-aligned).
                let label_rect = UiRect::new(
                    item_rect.left + ITEM_PAD_X,
                    item_rect.top,
                    item_rect.right - ITEM_PAD_X,
                    item_rect.bottom,
                );
                let label_color = if hovered { theme::ZINC_100 } else { theme::ZINC_300 };
                item = item.child(text(
                    label_rect,
                    label,
                    theme::mono(label_color, theme::UI_SIZE),
                ));

                // Shortcut (right-aligned, dimmed).
                if let Some(sc) = *shortcut {
                    let sc_w = measure(sc);
                    let sc_rect = UiRect::new(
                        item_rect.right - ITEM_PAD_X - sc_w - TEXT_MARGIN,
                        item_rect.top,
                        item_rect.right - ITEM_PAD_X,
                        item_rect.bottom,
                    );
                    item = item.child(text(
                        sc_rect,
                        sc,
                        theme::mono_right(theme::ZINC_500, theme::UI_SIZE),
                    ));
                }

                surface = surface.child(item);
                y += ITEM_H;
                index += 1;
            }
        }
    }

    overlay.child(surface)
}
