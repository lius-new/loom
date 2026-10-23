//! Context menu for editor tabs.

use lgui::core::{CursorIcon, EventPolicy, UiEventKind};
use lgui::prelude::{Color, Element, ShadowStyle, UiRect, VisualStyle, panel, text};

use crate::ui::chrome::menu_binding::Binding;
use crate::ui::theme;

const MENU_W: f32 = 224.0;
const MENU_PAD: f32 = 4.0;
const ITEM_H: f32 = 22.0;
const ITEM_PAD_X: f32 = 10.0;
const SEP_H: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    KeepOpen,
    ToggleMarkdown,
    MarkdownToSide,
    MarkdownSource,
    MarkdownLarger,
    SplitRight,
    SplitDown,
    ReloadFromDisk,
    KeepEditorVersion,
    Close,
    CloseOthers,
    CloseRight,
    CloseSaved,
    CloseAll,
    CopyPath,
    CopyRelativePath,
    RevealInTree,
    RevealInFileExplorer,
}

pub(crate) enum Entry {
    Item(&'static str, Action),
    Separator,
}

pub(crate) const ENTRIES: &[Entry] = &[
    Entry::Item("Toggle Markdown Preview", Action::ToggleMarkdown),
    Entry::Item("Markdown Preview to Side", Action::MarkdownToSide),
    Entry::Item("Open Markdown Source", Action::MarkdownSource),
    Entry::Item("Preview Larger Document", Action::MarkdownLarger),
    Entry::Separator,
    Entry::Item("Keep Open", Action::KeepOpen),
    Entry::Separator,
    Entry::Item("Split Right", Action::SplitRight),
    Entry::Item("Split Down", Action::SplitDown),
    Entry::Separator,
    Entry::Item("Reload from Disk", Action::ReloadFromDisk),
    Entry::Item("Keep Editor Version", Action::KeepEditorVersion),
    Entry::Separator,
    Entry::Item("Close", Action::Close),
    Entry::Item("Close Others", Action::CloseOthers),
    Entry::Item("Close to the Right", Action::CloseRight),
    Entry::Item("Close Saved", Action::CloseSaved),
    Entry::Item("Close All", Action::CloseAll),
    Entry::Separator,
    Entry::Item("Copy Path", Action::CopyPath),
    Entry::Item("Copy Relative Path", Action::CopyRelativePath),
    Entry::Separator,
    Entry::Item("Reveal in File Tree", Action::RevealInTree),
    Entry::Item("Reveal in File Explorer", Action::RevealInFileExplorer),
];

pub fn render(viewport: UiRect, state: Binding) -> Element {
    let Some(menu) = state.frame() else {
        return panel(viewport, VisualStyle::default());
    };
    let target = menu.target;
    let pane = menu.pane;
    let menu_h = MENU_PAD * 2.0
        + ENTRIES
            .iter()
            .map(|entry| match entry {
                Entry::Item(..) => ITEM_H,
                Entry::Separator => SEP_H,
            })
            .sum::<f32>();
    let x = if menu.position.0 + MENU_W <= viewport.right {
        menu.position.0
    } else {
        menu.position.0 - MENU_W
    }
    .clamp(viewport.left, (viewport.right - MENU_W).max(viewport.left));
    let y = if menu.position.1 + menu_h <= viewport.bottom {
        menu.position.1
    } else {
        menu.position.1 - menu_h
    }
    .clamp(viewport.top, (viewport.bottom - menu_h).max(viewport.top));
    let card = UiRect::new(x, y, x + MENU_W, y + menu_h);

    let dismiss_state = state.clone();
    let overlay = panel(viewport, VisualStyle::default())
        .event_policy(EventPolicy::INTERACTIVE)
        .on_pointer_down_with_button(move |_cx, pointer, _button| {
            if !card.contains(pointer.point) {
                dismiss_state.dismiss(pane, target);
            }
        });

    let clear_hover = state.clone();
    let mut surface = theme::bordered(card, theme::c().surface, theme::c().border, 6.0, 1.0)
        .shadow(
            ShadowStyle::new(Color::BLACK)
                .alpha(theme::c().shadow_alpha)
                .offset(0.0, 2.0)
                .blur(3.0),
        )
        .event_policy(EventPolicy::INTERACTIVE)
        .on_event_capture(UiEventKind::PointerMove, move |_cx, _payload| {
            clear_hover.hover(pane, target, None);
        });

    let mut row_top = card.top + MENU_PAD;
    let mut item_index = 0usize;
    for entry in ENTRIES {
        match entry {
            Entry::Separator => {
                let line_y = row_top + (SEP_H - 1.0) / 2.0;
                surface = surface.child(panel(
                    UiRect::new(
                        card.left + ITEM_PAD_X,
                        line_y,
                        card.right - ITEM_PAD_X,
                        line_y + 1.0,
                    ),
                    VisualStyle::filled(theme::c().border),
                ));
                row_top += SEP_H;
            }
            Entry::Item(label, action) => {
                let enabled = menu.enabled.contains(action);
                let hovered = enabled && menu.hovered == Some(item_index);
                let row = UiRect::new(card.left + 1.0, row_top, card.right - 1.0, row_top + ITEM_H);
                let hover_state = state.clone();
                let click_state = state.clone();
                let index = item_index;
                let action = *action;
                let mut item = panel(
                    row,
                    if hovered {
                        VisualStyle::filled(theme::c().selection)
                    } else {
                        VisualStyle::default()
                    },
                )
                .event_policy(EventPolicy::INTERACTIVE)
                .cursor(if enabled {
                    CursorIcon::Pointer
                } else {
                    CursorIcon::Default
                })
                .on_event(UiEventKind::PointerMove, move |_cx, _payload| {
                    hover_state.hover(pane, target, enabled.then_some(index));
                })
                .on_click(move |cx: &mut lgui::core::UiEventContext| {
                    if enabled {
                        click_state.execute(pane, target, action, cx);
                    }
                });
                item = item.child(text(
                    UiRect::new(
                        row.left + ITEM_PAD_X,
                        row.top,
                        row.right - ITEM_PAD_X,
                        row.bottom,
                    ),
                    *label,
                    theme::mono(
                        if hovered {
                            theme::c().text_bright
                        } else if enabled {
                            theme::c().text_soft
                        } else {
                            theme::c().text_dim
                        },
                        theme::UI_SIZE,
                    ),
                ));
                surface = surface.child(item);
                row_top += ITEM_H;
                item_index += 1;
            }
        }
    }

    overlay.child(surface)
}
