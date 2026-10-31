//! Context actions target the right-clicked terminal, not the active session.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lgui::core::{
    CursorIcon, EventPolicy, LogicalKey, NamedKey, SemanticRole, Semantics, UiElement, UiId,
};
use lgui::prelude::{Color, Element, RenderCx, ShadowStyle, UiRect, VisualStyle, panel, text};

use super::bindings as commands;
use super::bindings::{Binding, Command, FrameRequest, Sessions, View as TerminalTabs};
use super::state::TerminalTabMenuKind;
use crate::features::terminal::TERMINAL_PURPOSES as PURPOSES;
use crate::ui::tabs::ellipsize;
use crate::ui::theme;

use crate::features::terminal::actions::{
    MenuAction as Action, close_targets_in as model_close_targets,
};
fn close_targets(
    tabs: &TerminalTabs,
    regions: &[Vec<u64>],
    target: u64,
    action: Action,
) -> Vec<u64> {
    let region = regions
        .iter()
        .find(|ids| ids.contains(&target))
        .map_or(&[][..], Vec::as_slice);
    model_close_targets(tabs.model(), target, action, region)
}

const MENU_W: f32 = 224.0;
const PAD: f32 = 4.0;
const ITEM_H: f32 = 22.0;
const SEP_H: f32 = 5.0;
/// Space between a row's highlight and the card edge on each side.
const ROW_INSET: f32 = 4.0;

const ITEMS: &[(&str, Action)] = &[
    ("New Terminal", Action::New),
    ("Split Horizontally", Action::Split),
    ("Close", Action::Close),
    ("Close Others", Action::Others),
    ("Close to the Right", Action::Right),
    ("Close All", Action::All),
];

/// The menu's previous focus may be a tab button. Restore the terminal surface
/// explicitly so Escape and outside clicks let the user keep typing immediately.
pub fn restore_focus_on_dismiss(
    cx: &mut RenderCx<'_, '_>,
    open: bool,
    terminal_visible: bool,
    terminal_focus: lgui::core::UiFocusHandle,
    editor_focus: lgui::core::UiFocusHandle,
) {
    let was_open = cx.state_with(|| Arc::new(AtomicBool::new(false))).get();
    cx.use_effect((open, terminal_visible), move || {
        if was_open.swap(open, Ordering::AcqRel) && !open {
            if terminal_visible {
                terminal_focus.focus();
            } else {
                editor_focus.focus();
            }
        }
    });
}

pub fn render(
    viewport: UiRect,
    state: Binding,
    tabs: Sessions,
    application: FrameRequest,
    terminal_focus: lgui::core::UiFocusHandle,
    editor_focus: lgui::core::UiFocusHandle,
) -> Element {
    let snapshot = state.get();
    if snapshot.ui.terminal_workspace_picker.is_some() {
        return super::workspace_picker::render(viewport, state, terminal_focus);
    }
    let Some(menu) = snapshot.ui.terminal_tab_context_menu else {
        return panel(viewport, VisualStyle::default());
    };
    let tab_snapshot = tabs.get();
    let Some(target_tab) = tab_snapshot
        .tabs()
        .into_iter()
        .find(|tab| tab.id == menu.target)
    else {
        return panel(viewport, VisualStyle::default());
    };
    match menu.kind {
        TerminalTabMenuKind::Actions => {}
        TerminalTabMenuKind::Purpose => {
            return render_purpose(viewport, state, tabs, menu, target_tab.purpose.clone());
        }
        TerminalTabMenuKind::More => return render_more(viewport, state, menu),
        TerminalTabMenuKind::Sessions => return render_sessions(viewport, state, tabs, menu),
    }
    let target = menu.target;
    let height = PAD * 2.0 + ITEM_H * ITEMS.len() as f32 + SEP_H;
    let width = MENU_W.min(viewport.width().max(0.0));
    let left = menu
        .position
        .0
        .clamp(viewport.left, (viewport.right - width).max(viewport.left));
    let top = menu
        .position
        .1
        .clamp(viewport.top, (viewport.bottom - height).max(viewport.top));
    let card = UiRect::new(left, top, left + width, top + height);
    let dismiss = state.clone();
    let escape = state.clone();
    let mut overlay = Element::new(move |cx| {
        UiElement::panel(cx.id, viewport, VisualStyle::default())
            .auto_focus()
            .children(cx.children)
    })
    .key("terminal-tab-menu")
    .event_policy(EventPolicy::INTERACTIVE)
    .focus_scope()
    .on_pointer_down_with_button(move |cx, pointer, _| {
        if !card.contains(pointer.point) {
            dismiss
                .ui
                .update(|app| app.terminal_tab_context_menu = None);
        }
        cx.stop_propagation();
    })
    .on_input(|cx, _| cx.stop_propagation())
    .on_key_down(move |cx, event| {
        if event.key == LogicalKey::Named(NamedKey::Escape) {
            escape.ui.update(|app| app.terminal_tab_context_menu = None);
        }
        cx.prevent_default();
        cx.stop_propagation();
    });
    let mut surface = theme::bordered(card, theme::c().surface, theme::c().border, 6.0, 1.0)
        .shadow(
            ShadowStyle::new(Color::BLACK)
                .alpha(theme::c().shadow_alpha)
                .offset(0.0, 2.0)
                .blur(3.0),
        );
    let mut y = card.top + PAD;
    for &(label, action) in ITEMS {
        if action == Action::Close {
            surface = surface.child(panel(
                UiRect::new(card.left + 10.0, y + 2.0, card.right - 10.0, y + 3.0),
                VisualStyle::filled(theme::c().border),
            ));
            y += SEP_H;
        }
        let enabled = matches!(action, Action::New | Action::Split)
            || !close_targets(&tab_snapshot, &snapshot.regions, target, action).is_empty();
        let row = UiRect::new(card.left + 1.0, y, card.right - 1.0, y + ITEM_H);
        let action_state = state.clone();
        let action_tabs = tabs.clone();
        let action_application = application.clone();
        let action_terminal_focus = terminal_focus.clone();
        let action_editor_focus = editor_focus.clone();
        surface = surface.child(
            Element::new(move |cx| {
                let hovered = cx.context.interaction_flags(&cx.id).hovered;
                UiElement::panel(
                    cx.id,
                    row,
                    if enabled && hovered {
                        VisualStyle::filled(theme::c().selection)
                    } else {
                        VisualStyle::default()
                    },
                )
                .children(cx.children)
            })
            .key(label)
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(if enabled {
                CursorIcon::Pointer
            } else {
                CursorIcon::Default
            })
            .on_click(move || {
                if !enabled {
                    return;
                }
                if !commands::menu_action(
                    &action_state,
                    &action_tabs,
                    target,
                    action,
                    &action_application,
                ) {
                    return;
                }
                if action_tabs.get().is_empty() {
                    action_editor_focus.focus();
                } else {
                    action_terminal_focus.focus();
                }
            })
            .child(text(
                UiRect::new(row.left + 10.0, row.top, row.right - 10.0, row.bottom),
                label,
                theme::mono(
                    if enabled {
                        theme::c().text
                    } else {
                        theme::c().text_faint
                    },
                    theme::SMALL,
                ),
            )),
        );
        y += ITEM_H;
    }
    overlay = overlay.child(surface);
    overlay
}

/// Every terminal in a list, for when the strip holds more tabs than fit.
/// `menu.position` is the right edge of the button that opened it and the top.
fn render_sessions(
    viewport: UiRect,
    state: Binding,
    tabs: Sessions,
    menu: super::state::TerminalTabContextMenuState,
) -> Element {
    let all = tabs.get().tabs();
    let rows = all.len().min(14);
    let height = PAD * 2.0 + ITEM_H * rows as f32;
    let width = 280.0_f32.min(viewport.width().max(0.0));
    let left =
        (menu.position.0 - width).clamp(viewport.left, (viewport.right - width).max(viewport.left));
    let top = menu
        .position
        .1
        .clamp(viewport.top, (viewport.bottom - height).max(viewport.top));
    let card = UiRect::new(left, top, left + width, top + height);
    let dismiss = state.clone();
    let escape = state.clone();
    let overlay = Element::new(move |cx| {
        UiElement::panel(cx.id, viewport, VisualStyle::default())
            .auto_focus()
            .children(cx.children)
    })
    .key("terminal-sessions-menu")
    .event_policy(EventPolicy::INTERACTIVE)
    .focus_scope()
    .on_pointer_down_with_button(move |cx, pointer, _| {
        if !card.contains(pointer.point) {
            dismiss
                .ui
                .update(|app| app.terminal_tab_context_menu = None);
        }
        cx.stop_propagation();
    })
    .on_input(|cx, _| cx.stop_propagation())
    .on_key_down(move |cx, event| {
        if event.key == LogicalKey::Named(NamedKey::Escape) {
            escape.ui.update(|app| app.terminal_tab_context_menu = None);
        }
        cx.prevent_default();
        cx.stop_propagation();
    });
    let mut surface = theme::bordered(card, theme::c().surface, theme::c().border, 6.0, 1.0)
        .shadow(
            ShadowStyle::new(Color::BLACK)
                .alpha(theme::c().shadow_alpha)
                .offset(0.0, 2.0)
                .blur(3.0),
        );
    for (index, tab) in all.iter().take(rows).enumerate() {
        let top = card.top + PAD + ITEM_H * index as f32;
        let row = UiRect::new(card.left + 1.0, top, card.right - 1.0, top + ITEM_H);
        let (id, pick_state, pick_tabs) = (tab.id, state.clone(), tabs.clone());
        surface = surface.child(
            keyed_row(
                row,
                &ellipsize(&tab.label(), width - 40.0, theme::SMALL, 400),
                tab.id == menu.target,
                format!("terminal-session-row-{id}"),
            )
            .on_click(move || {
                commands::select(&pick_state, &pick_tabs, id);
                pick_state
                    .ui
                    .update(|app| app.terminal_tab_context_menu = None);
            }),
        );
    }
    overlay.child(surface)
}

/// The header's more-actions menu. `menu.position` is the right edge of the
/// button that opened it and the top of the menu.
fn render_more(
    viewport: UiRect,
    state: Binding,
    menu: super::state::TerminalTabContextMenuState,
) -> Element {
    let target = menu.target;
    let height = PAD * 2.0 + ITEM_H * 2.0;
    let width = MENU_W.min(viewport.width().max(0.0));
    let left =
        (menu.position.0 - width).clamp(viewport.left, (viewport.right - width).max(viewport.left));
    let top = menu
        .position
        .1
        .clamp(viewport.top, (viewport.bottom - height).max(viewport.top));
    let card = UiRect::new(left, top, left + width, top + height);
    let dismiss = state.clone();
    let escape = state.clone();
    let overlay = Element::new(move |cx| {
        UiElement::panel(cx.id, viewport, VisualStyle::default())
            .auto_focus()
            .children(cx.children)
    })
    .key("terminal-more-menu")
    .event_policy(EventPolicy::INTERACTIVE)
    .focus_scope()
    .on_pointer_down_with_button(move |cx, pointer, _| {
        if !card.contains(pointer.point) {
            dismiss
                .ui
                .update(|app| app.terminal_tab_context_menu = None);
        }
        cx.stop_propagation();
    })
    .on_input(|cx, _| cx.stop_propagation())
    .on_key_down(move |cx, event| {
        if event.key == LogicalKey::Named(NamedKey::Escape) {
            escape.ui.update(|app| app.terminal_tab_context_menu = None);
        }
        cx.prevent_default();
        cx.stop_propagation();
    });
    let mut surface = theme::bordered(card, theme::c().surface, theme::c().border, 6.0, 1.0)
        .shadow(
            ShadowStyle::new(Color::BLACK)
                .alpha(theme::c().shadow_alpha)
                .offset(0.0, 2.0)
                .blur(3.0),
        );
    let rename_row = UiRect::new(
        card.left + 1.0,
        card.top + PAD,
        card.right - 1.0,
        card.top + PAD + ITEM_H,
    );
    // The label renames by hand; the trailing button picks a preset purpose,
    // which replaces double-clicking the tab. One highlight covers the whole
    // row, button included, and the button only tints itself on top of it.
    let button_right = rename_row.right - ROW_INSET - 2.0;
    let button_rect = UiRect::new(
        button_right - ITEM_H,
        rename_row.top,
        button_right,
        rename_row.bottom,
    );
    let label_rect = UiRect::new(
        rename_row.left,
        rename_row.top,
        button_rect.left,
        rename_row.bottom,
    );
    let label_id = UiId::owned("terminal-more-rename");
    let button_id = UiId::owned("terminal-purpose-button");
    let (hover_label, hover_button) = (label_id.clone(), button_id.clone());
    let highlight = UiRect::new(
        rename_row.left + ROW_INSET,
        rename_row.top,
        rename_row.right - ROW_INSET,
        rename_row.bottom,
    );
    let rename_state = state.clone();
    let label_node = label_id.clone();
    let rename_label = Element::new(move |cx| {
        UiElement::panel(label_node.clone(), label_rect, VisualStyle::default())
            .children(cx.children)
    })
    .key("terminal-more-rename")
    .event_policy(EventPolicy::INTERACTIVE)
    .cursor(CursorIcon::Pointer)
    .semantics(Semantics::new(SemanticRole::Button).name("Rename Session"))
    .on_click(move || {
        rename_state.run(Command::BeginRename(target));
    })
    .child(text(
        UiRect::new(
            label_rect.left + 10.0,
            label_rect.top,
            label_rect.right,
            label_rect.bottom,
        ),
        "  Rename Session",
        theme::mono(theme::c().text, theme::SMALL),
    ));
    let purpose_state = state.clone();
    let purpose_anchor = (card.left, card.top);
    let purpose_button = super::header::button_styled(
        "terminal-purpose-button",
        button_rect,
        button_rect,
        "terminal-purpose",
        "Choose purpose",
        false,
        theme::c().text_soft,
        (theme::c().accent, 45),
        move || {
            purpose_state.run(Command::OpenMenu {
                target,
                position: purpose_anchor,
                kind: TerminalTabMenuKind::Purpose,
            });
        },
    );
    surface = surface.child(
        Element::new(move |cx| {
            let hovered = [&cx.id, &hover_label, &hover_button]
                .into_iter()
                .any(|id| cx.context.interaction_flags(id).hovered);
            UiElement::panel(
                cx.id,
                highlight,
                if hovered {
                    VisualStyle::filled(theme::c().selection).radius(4.0)
                } else {
                    VisualStyle::default()
                },
            )
            .children(cx.children)
        })
        .key("terminal-more-rename-row")
        .child(rename_label)
        .child(purpose_button),
    );
    let clear_state = state.clone();
    let clear_row = UiRect::new(
        card.left + 1.0,
        rename_row.bottom,
        card.right - 1.0,
        rename_row.bottom + ITEM_H,
    );
    surface = surface.child(
        purpose_row(clear_row, "Clear Output", false).on_click(move || {
            clear_state.run(Command::ClearOutput(target));
        }),
    );
    overlay.child(surface)
}

fn render_purpose(
    viewport: UiRect,
    state: Binding,
    tabs: Sessions,
    menu: super::state::TerminalTabContextMenuState,
    current: Option<String>,
) -> Element {
    let target = menu.target;
    let height = PAD * 2.0 + ITEM_H * (PURPOSES.len() + 1) as f32 + SEP_H;
    let width = MENU_W.min(viewport.width().max(0.0));
    let left = menu
        .position
        .0
        .clamp(viewport.left, (viewport.right - width).max(viewport.left));
    let top = menu
        .position
        .1
        .clamp(viewport.top, (viewport.bottom - height).max(viewport.top));
    let card = UiRect::new(left, top, left + width, top + height);
    let dismiss = state.clone();
    let escape = state.clone();
    let overlay = Element::new(move |cx| {
        UiElement::panel(cx.id, viewport, VisualStyle::default())
            .auto_focus()
            .children(cx.children)
    })
    .key(format!("terminal-purpose-menu-{target}"))
    .event_policy(EventPolicy::INTERACTIVE)
    .focus_scope()
    .on_pointer_down_with_button(move |cx, pointer, _| {
        if !card.contains(pointer.point) {
            dismiss
                .ui
                .update(|app| app.terminal_tab_context_menu = None);
        }
        cx.stop_propagation();
    })
    .on_input(|cx, _| cx.stop_propagation())
    .on_key_down(move |cx, event| {
        if event.key == LogicalKey::Named(NamedKey::Escape) {
            escape.ui.update(|app| app.terminal_tab_context_menu = None);
        }
        cx.prevent_default();
        cx.stop_propagation();
    });
    let mut surface = theme::bordered(card, theme::c().surface, theme::c().border, 6.0, 1.0)
        .shadow(
            ShadowStyle::new(Color::BLACK)
                .alpha(theme::c().shadow_alpha)
                .offset(0.0, 2.0)
                .blur(3.0),
        );
    let mut y = card.top + PAD;
    for &purpose in PURPOSES {
        let row = UiRect::new(card.left + 1.0, y, card.right - 1.0, y + ITEM_H);
        let choose_tabs = tabs.clone();
        let choose_state = state.clone();
        surface = surface.child(
            purpose_row(row, purpose, current.as_deref() == Some(purpose)).on_click(move || {
                commands::purpose(&choose_state, &choose_tabs, target, Some(purpose));
            }),
        );
        y += ITEM_H;
    }
    surface = surface.child(panel(
        UiRect::new(card.left + 10.0, y + 2.0, card.right - 10.0, y + 3.0),
        VisualStyle::filled(theme::c().border),
    ));
    y += SEP_H;
    let clear_tabs = tabs.clone();
    let clear_state = state.clone();
    let row = UiRect::new(card.left + 1.0, y, card.right - 1.0, y + ITEM_H);
    surface = surface.child(
        purpose_row(row, "Automatic name", current.is_none()).on_click(move || {
            commands::purpose(&clear_state, &clear_tabs, target, None);
        }),
    );
    overlay.child(surface)
}

fn purpose_row(row: UiRect, label: &str, selected: bool) -> Element {
    keyed_row(row, label, selected, label.to_owned())
}

/// A menu row with an explicit key, for rows whose labels can repeat.
fn keyed_row(row: UiRect, label: &str, selected: bool, key: String) -> Element {
    // The highlight stops short of the card's edges; the whole row stays clickable.
    let highlight = UiRect::new(
        row.left + ROW_INSET,
        row.top,
        row.right - ROW_INSET,
        row.bottom,
    );
    Element::new(move |cx| {
        let hovered = cx.context.interaction_flags(&cx.id).hovered;
        UiElement::panel(
            cx.id,
            highlight,
            if hovered {
                VisualStyle::filled(theme::c().selection).radius(4.0)
            } else {
                VisualStyle::default()
            },
        )
        .hit_rect(row)
        .children(cx.children)
    })
    .key(key)
    .event_policy(EventPolicy::INTERACTIVE)
    .cursor(CursorIcon::Pointer)
    .semantics(Semantics::new(SemanticRole::Button).name(label))
    .child(text(
        UiRect::new(row.left + 10.0, row.top, row.right - 10.0, row.bottom),
        if selected {
            format!("✓ {label}")
        } else {
            format!("  {label}")
        },
        theme::mono(theme::c().text, theme::SMALL),
    ))
}
