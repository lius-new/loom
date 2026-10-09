//! Context actions target the right-clicked terminal, not the active session.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lgui::ApplicationHandle;
use lgui::core::{
    CursorIcon, EventPolicy, LogicalKey, NamedKey, SemanticRole, Semantics, UiElement,
};
use lgui::prelude::{
    Color, Element, RenderCx, ShadowStyle, State, UiRect, VisualStyle, panel, text,
};

use crate::state::{AppState, TerminalTabMenuKind};
use crate::terminal_session::{TERMINAL_PURPOSES as PURPOSES, TerminalTabs};
use crate::theme;

const MENU_W: f32 = 224.0;
const PAD: f32 = 4.0;
const ITEM_H: f32 = 22.0;
const SEP_H: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    New,
    Split,
    Close,
    Others,
    Right,
    All,
}

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

fn close_targets(tabs: &TerminalTabs, target: u64, action: Action) -> Vec<u64> {
    let Some(index) = tabs.tabs().iter().position(|tab| tab.id == target) else {
        return Vec::new();
    };
    tabs.tabs()
        .iter()
        .enumerate()
        .filter_map(|(i, tab)| {
            match action {
                Action::New | Action::Split => false,
                Action::Close => tab.id == target,
                Action::Others => tab.group_id != tabs.tabs()[index].group_id,
                Action::Right => i > index && tab.group_id != tabs.tabs()[index].group_id,
                Action::All => true,
            }
            .then_some(tab.id)
        })
        .collect()
}

pub fn render(
    viewport: UiRect,
    state: State<AppState>,
    tabs: State<TerminalTabs>,
    application: Arc<ApplicationHandle>,
    terminal_focus: lgui::core::UiFocusHandle,
    editor_focus: lgui::core::UiFocusHandle,
) -> Element {
    let snapshot = state.get();
    let Some(menu) = snapshot.terminal_tab_context_menu else {
        return panel(viewport, VisualStyle::default());
    };
    let tab_snapshot = tabs.get();
    let Some(target_tab) = tab_snapshot.tabs().iter().find(|tab| tab.id == menu.target) else {
        return panel(viewport, VisualStyle::default());
    };
    if menu.kind != TerminalTabMenuKind::Actions {
        return render_purpose(viewport, state, tabs, menu, target_tab.purpose.clone());
    }
    let shell = target_tab.controller.shell();
    let cwd = target_tab
        .cwd
        .clone()
        .or_else(|| snapshot.workspace_folders.first().cloned());
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
            dismiss.update(|app| app.terminal_tab_context_menu = None);
        }
        cx.stop_propagation();
    })
    .on_input(|cx, _| cx.stop_propagation())
    .on_key_down(move |cx, event| {
        if event.key == LogicalKey::Named(NamedKey::Escape) {
            escape.update(|app| app.terminal_tab_context_menu = None);
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
            || !close_targets(&tab_snapshot, target, action).is_empty();
        let row = UiRect::new(card.left + 1.0, y, card.right - 1.0, y + ITEM_H);
        let action_state = state.clone();
        let action_tabs = tabs.clone();
        let action_application = application.clone();
        let action_terminal_focus = terminal_focus.clone();
        let action_editor_focus = editor_focus.clone();
        let cwd = cwd.clone();
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
                action_state.update(|app| app.terminal_tab_context_menu = None);
                // Resolve the target again: a terminal may have closed while the menu was open.
                if !action_tabs.get().tabs().iter().any(|tab| tab.id == target) {
                    return;
                }
                if action == Action::New {
                    let cwd = cwd.clone();
                    action_tabs.update(|tabs| {
                        let id = tabs.add_at(shell, cwd);
                        action_state.update(|app| {
                            if let Some(layout) = &mut app.application_layout {
                                layout.attach(id);
                            }
                        });
                    });
                } else if action == Action::Split {
                    action_tabs.update(|tabs| {
                        if let Some(id) = tabs.split(target) {
                            action_state.update(|app| {
                                if let Some(layout) = &mut app.application_layout {
                                    layout.attach(id);
                                }
                            });
                        }
                    });
                } else {
                    let targets = close_targets(&action_tabs.get(), target, action);
                    let mut controllers = Vec::new();
                    action_tabs.update(|tabs| {
                        for id in targets {
                            if let Some(controller) = tabs.close(id) {
                                controllers.push(controller);
                            }
                        }
                    });
                    for controller in controllers {
                        controller.terminate(&action_application);
                    }
                }
                if action_tabs.get().is_empty() {
                    action_state.update(|app| {
                        app.show_terminal = false;
                        app.terminal_shell_menu = false;
                        app.terminal_focused = false;
                    });
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

fn render_purpose(
    viewport: UiRect,
    state: State<AppState>,
    tabs: State<TerminalTabs>,
    menu: crate::state::TerminalTabContextMenuState,
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
            dismiss.update(|app| app.terminal_tab_context_menu = None);
        }
        cx.stop_propagation();
    })
    .on_input(|cx, _| cx.stop_propagation())
    .on_key_down(move |cx, event| {
        if event.key == LogicalKey::Named(NamedKey::Escape) {
            escape.update(|app| app.terminal_tab_context_menu = None);
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
                choose_tabs.update(|tabs| tabs.set_purpose(target, Some(purpose)));
                choose_state.update(|app| app.terminal_tab_context_menu = None);
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
            clear_tabs.update(|tabs| tabs.set_purpose(target, None));
            clear_state.update(|app| app.terminal_tab_context_menu = None);
        }),
    );
    overlay.child(surface)
}

fn purpose_row(row: UiRect, label: &str, selected: bool) -> Element {
    Element::new(move |cx| {
        let hovered = cx.context.interaction_flags(&cx.id).hovered;
        UiElement::panel(
            cx.id,
            row,
            if hovered {
                VisualStyle::filled(theme::c().selection)
            } else {
                VisualStyle::default()
            },
        )
        .children(cx.children)
    })
    .key(label.to_owned())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_session::ShellKind;

    #[test]
    fn actions_target_the_clicked_terminal_and_preserve_unclosed_sessions() {
        let mut tabs = TerminalTabs::new();
        let first = tabs.active_id().unwrap();
        let middle = tabs.add(ShellKind::Cmd);
        let last = tabs.add(ShellKind::Bash);
        assert_eq!(tabs.active_id(), Some(last));
        assert_eq!(close_targets(&tabs, middle, Action::Close), vec![middle]);
        assert_eq!(
            close_targets(&tabs, middle, Action::Others),
            vec![first, last]
        );
        assert_eq!(close_targets(&tabs, middle, Action::Right), vec![last]);
        assert!(close_targets(&tabs, last, Action::Right).is_empty());
        assert!(close_targets(&tabs, 999, Action::All).is_empty());
        for id in close_targets(&tabs, middle, Action::Others) {
            tabs.close(id);
        }
        assert_eq!(tabs.active_id(), Some(middle));
        assert!(close_targets(&tabs, middle, Action::Others).is_empty());
        let sibling = tabs.split(middle).unwrap();
        let other = tabs.add(ShellKind::Bash);
        assert_eq!(close_targets(&tabs, middle, Action::Others), vec![other]);
        assert_eq!(close_targets(&tabs, middle, Action::Right), vec![other]);
        assert_eq!(close_targets(&tabs, sibling, Action::Close), vec![sibling]);
    }
}
