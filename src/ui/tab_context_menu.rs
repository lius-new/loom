//! Context menu for editor tabs.

use lgui::core::{CursorIcon, EventPolicy, UiEventKind};
use lgui::prelude::{Color, Element, ShadowStyle, State, UiRect, VisualStyle, panel, text};
use lgui::services::ServicesContextExt;

use crate::model::document::FileId;
use crate::model::pane_layout::PaneId;
use crate::state::{AppState, ExplorerContextTarget, ExplorerTargetKind};
use crate::theme;
use crate::workspace_actions::{self, TabCloseScope};

const MENU_W: f32 = 224.0;
const MENU_PAD: f32 = 4.0;
const ITEM_H: f32 = 22.0;
const ITEM_PAD_X: f32 = 10.0;
const SEP_H: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    KeepOpen,
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

enum Entry {
    Item(&'static str, Action),
    Separator,
}

const ENTRIES: &[Entry] = &[
    Entry::Item("Keep Open", Action::KeepOpen),
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

fn close_scope(action: Action) -> Option<TabCloseScope> {
    match action {
        Action::Close => Some(TabCloseScope::Tab),
        Action::CloseOthers => Some(TabCloseScope::Others),
        Action::CloseRight => Some(TabCloseScope::Right),
        Action::CloseSaved => Some(TabCloseScope::Saved),
        Action::CloseAll => Some(TabCloseScope::All),
        _ => None,
    }
}

fn action_enabled(app: &AppState, pane: PaneId, target: FileId, action: Action) -> bool {
    let Some(meta) = app.workspace.meta(target) else {
        return false;
    };
    if let Some(scope) = close_scope(action) {
        return !workspace_actions::tab_close_targets(app, pane, target, scope).is_empty();
    }
    // Built-in pages have no path to copy or reveal.
    if app.workspace.page(target).is_some() {
        return false;
    }
    match action {
        Action::KeepOpen => app
            .workspace
            .pane(pane)
            .is_some_and(|pane| pane.is_preview(target)),
        Action::ReloadFromDisk => {
            app.workspace.has_disk_conflict(target) && !app.workspace.is_missing_on_disk(target)
        }
        Action::KeepEditorVersion => app.workspace.has_disk_conflict(target),
        Action::CopyPath => true,
        Action::CopyRelativePath => {
            super::context_menu::workspace_root_for_path(app, &meta.path).is_some()
        }
        Action::RevealInTree => {
            !app.workspace.is_diff(target)
                && super::context_menu::workspace_root_for_path(app, &meta.path).is_some()
        }
        Action::RevealInFileExplorer => !app.workspace.is_diff(target),
        _ => false,
    }
}

pub fn render(viewport: UiRect, state: State<AppState>) -> Element {
    let snapshot = state.get();
    let Some(menu) = snapshot.tab_context_menu.as_ref() else {
        return panel(viewport, VisualStyle::default());
    };
    let target = menu.target;
    let pane = menu.pane;
    let Some(path) = snapshot
        .workspace
        .meta(target)
        .map(|meta| meta.path.clone())
    else {
        return panel(viewport, VisualStyle::default());
    };
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
                dismiss_state.update(|app| app.tab_context_menu = None);
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
            clear_hover.try_update(|app| {
                let Some(menu) = app.tab_context_menu.as_mut() else {
                    return false;
                };
                let changed = menu.hovered.is_some();
                menu.hovered = None;
                changed
            });
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
                let enabled = action_enabled(&snapshot, pane, target, *action);
                let hovered = enabled && menu.hovered == Some(item_index);
                let row = UiRect::new(card.left + 1.0, row_top, card.right - 1.0, row_top + ITEM_H);
                let hover_state = state.clone();
                let click_state = state.clone();
                let index = item_index;
                let action = *action;
                let action_path = path.clone();
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
                    hover_state.try_update(move |app| {
                        let Some(menu) = app.tab_context_menu.as_mut() else {
                            return false;
                        };
                        let next = enabled.then_some(index);
                        let changed = menu.hovered != next;
                        menu.hovered = next;
                        changed
                    });
                })
                .on_click(move |cx: &mut lgui::core::UiEventContext| {
                    if !enabled {
                        return;
                    }
                    click_state.update(|app| app.tab_context_menu = None);
                    if let Some(scope) = close_scope(action) {
                        workspace_actions::request_close_tabs(&click_state, pane, target, scope);
                        return;
                    }
                    match action {
                        Action::KeepOpen => {
                            click_state.update(|app| {
                                app.workspace.promote_preview(pane, target);
                            });
                        }
                        Action::ReloadFromDisk => {
                            click_state.update(|app| {
                                if let Err(error) = app.workspace.accept_disk_version(target) {
                                    app.show_error(error);
                                }
                            });
                        }
                        Action::KeepEditorVersion => {
                            click_state.update(|app| {
                                app.workspace.keep_editor_version(target);
                            });
                        }
                        Action::CopyPath | Action::CopyRelativePath => {
                            super::context_menu::copy_target_path(
                                &click_state,
                                cx.application().clipboard().as_ref(),
                                &action_path,
                                action == Action::CopyRelativePath,
                            );
                        }
                        Action::RevealInTree => {
                            workspace_actions::reveal_file_in_tree(&click_state, target);
                        }
                        Action::RevealInFileExplorer => {
                            super::context_menu::reveal_target(
                                &click_state,
                                &ExplorerContextTarget {
                                    path: action_path.clone(),
                                    kind: ExplorerTargetKind::File,
                                },
                            );
                        }
                        _ => {}
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn close_actions_disable_when_their_scope_is_empty() {
        let mut app = AppState::new();
        let only = app
            .workspace
            .open_path(PathBuf::from("workspace/only.rs"), String::new());

        assert!(action_enabled(
            &app,
            app.workspace.active_pane(),
            only,
            Action::Close
        ));
        assert!(!action_enabled(
            &app,
            app.workspace.active_pane(),
            only,
            Action::CloseOthers
        ));
        assert!(!action_enabled(
            &app,
            app.workspace.active_pane(),
            only,
            Action::CloseRight
        ));
        assert!(action_enabled(
            &app,
            app.workspace.active_pane(),
            only,
            Action::CloseSaved
        ));
        assert!(action_enabled(
            &app,
            app.workspace.active_pane(),
            only,
            Action::CloseAll
        ));
    }

    #[test]
    fn keep_open_is_available_only_for_the_preview_tab() {
        let mut app = AppState::new();
        let permanent = app
            .workspace
            .open_path(PathBuf::from("workspace/permanent.rs"), String::new());
        let preview = app
            .workspace
            .preview_path(PathBuf::from("workspace/preview.rs"), String::new());

        assert!(!action_enabled(
            &app,
            app.workspace.active_pane(),
            permanent,
            Action::KeepOpen
        ));
        assert!(action_enabled(
            &app,
            app.workspace.active_pane(),
            preview,
            Action::KeepOpen
        ));
    }

    #[test]
    fn relative_path_and_tree_actions_require_a_containing_workspace() {
        let mut app = AppState::new();
        let file = app
            .workspace
            .open_path(PathBuf::from("workspace/src/main.rs"), String::new());

        assert!(!action_enabled(
            &app,
            app.workspace.active_pane(),
            file,
            Action::CopyRelativePath
        ));
        assert!(!action_enabled(
            &app,
            app.workspace.active_pane(),
            file,
            Action::RevealInTree
        ));

        app.workspace_folders.push(PathBuf::from("workspace"));
        assert!(action_enabled(
            &app,
            app.workspace.active_pane(),
            file,
            Action::CopyRelativePath
        ));
        assert!(action_enabled(
            &app,
            app.workspace.active_pane(),
            file,
            Action::RevealInTree
        ));
    }

    #[test]
    fn disk_resolution_actions_enable_only_for_a_conflict() {
        let root = std::env::temp_dir().join(format!("loom-tab-conflict-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("note.txt");
        std::fs::write(&path, "before").unwrap();
        let mut app = AppState::new();
        let target = app.workspace.open_path(path.clone(), "before".into());
        assert!(!action_enabled(
            &app,
            app.workspace.active_pane(),
            target,
            Action::ReloadFromDisk
        ));
        assert!(!action_enabled(
            &app,
            app.workspace.active_pane(),
            target,
            Action::KeepEditorVersion
        ));

        app.workspace.active_editor_mut().unwrap().move_end();
        app.workspace.active_editor_mut().unwrap().insert(" editor");
        std::fs::write(&path, "after and longer").unwrap();
        app.workspace.reconcile_document(target);
        assert!(action_enabled(
            &app,
            app.workspace.active_pane(),
            target,
            Action::ReloadFromDisk
        ));
        assert!(action_enabled(
            &app,
            app.workspace.active_pane(),
            target,
            Action::KeepEditorVersion
        ));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn every_menu_action_is_enabled_when_its_target_is_available() {
        let mut app = AppState::new();
        app.workspace_folders.push(PathBuf::from("workspace"));
        let target = app
            .workspace
            .open_path(PathBuf::from("workspace/first.rs"), String::new());
        app.workspace
            .open_path(PathBuf::from("workspace/second.rs"), String::new());

        for action in [
            Action::Close,
            Action::CloseOthers,
            Action::CloseRight,
            Action::CloseSaved,
            Action::CloseAll,
            Action::CopyPath,
            Action::CopyRelativePath,
            Action::RevealInTree,
            Action::RevealInFileExplorer,
        ] {
            assert!(
                action_enabled(&app, app.workspace.active_pane(), target, action),
                "{action:?}"
            );
        }
    }
}
