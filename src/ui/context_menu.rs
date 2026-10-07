//! Context menu overlay for the file drawer and its directory rows.
//!
//! Directory rows carry an explicit target; an empty-space invocation falls
//! back to the active workspace root. Items are grouped by separators.

use std::path::{Path, PathBuf};
use std::process::Command;

use lgui::core::{CursorIcon, EventPolicy, UiEventKind};
use lgui::prelude::{Color, Element, ShadowStyle, State, UiRect, VisualStyle, panel, text};
use lgui::services::{Clipboard, ServicesContextExt};
use lgui::text::measure_width;
use unicode_width::UnicodeWidthStr;

use crate::state::{AppState, ExplorerContextTarget, ExplorerCreateKind, ExplorerTargetKind};
use crate::terminal_session::TerminalTabs;
use crate::theme;
use crate::workspace_actions;

const MENU_PAD: f32 = 4.0; // padding above/below the item list
const ITEM_H: f32 = 22.0; // item row height
const ITEM_PAD_X: f32 = 10.0; // horizontal padding inside an item
const SEP_H: f32 = 5.0; // separator row height
const SHORTCUT_GAP: f32 = 18.0; // min gap between a label and its shortcut
const TEXT_MARGIN: f32 = 6.0; // right margin so the last glyph is not clipped
const MENU_BORDER: f32 = 1.0; // menu border width

/// One row of the menu; `Separator` renders a hairline divider.
#[derive(Clone, Copy)]
enum MenuAction {
    NewFile,
    NewFolder,
    OpenTerminal,
    Cut,
    Copy,
    Rename,
    Delete,
    AddFolder,
    RemoveFolder,
    RevealInFileExplorer,
    CopyPath,
    CopyRelativePath,
}

fn action_enabled(
    action: MenuAction,
    has_target: bool,
    has_item_target: bool,
    can_remove: bool,
) -> bool {
    match action {
        MenuAction::AddFolder => true,
        MenuAction::RemoveFolder => can_remove,
        MenuAction::Cut | MenuAction::Copy | MenuAction::Rename | MenuAction::Delete => {
            has_item_target
        }
        _ => has_target,
    }
}

enum Entry {
    Item {
        label: &'static str,
        shortcut: Option<&'static str>,
        action: MenuAction,
    },
    Separator,
}

const DIRECTORY_ENTRIES: &[Entry] = &[
    Entry::Item {
        label: "New File",
        shortcut: None,
        action: MenuAction::NewFile,
    },
    Entry::Item {
        label: "New Folder",
        shortcut: None,
        action: MenuAction::NewFolder,
    },
    Entry::Separator,
    Entry::Item {
        label: "Reveal in File Explorer",
        shortcut: None,
        action: MenuAction::RevealInFileExplorer,
    },
    Entry::Item {
        label: "Open in Terminal",
        shortcut: None,
        action: MenuAction::OpenTerminal,
    },
    Entry::Separator,
    Entry::Item {
        label: "Cut",
        shortcut: None,
        action: MenuAction::Cut,
    },
    Entry::Item {
        label: "Copy",
        shortcut: None,
        action: MenuAction::Copy,
    },
    Entry::Separator,
    Entry::Item {
        label: "Add Folder From Workspace",
        shortcut: None,
        action: MenuAction::AddFolder,
    },
    Entry::Item {
        label: "Remove Folder From Workspace",
        shortcut: None,
        action: MenuAction::RemoveFolder,
    },
    Entry::Separator,
    Entry::Item {
        label: "Copy Path",
        shortcut: None,
        action: MenuAction::CopyPath,
    },
    Entry::Item {
        label: "Copy Relative Path",
        shortcut: None,
        action: MenuAction::CopyRelativePath,
    },
    Entry::Separator,
    Entry::Item {
        label: "Rename",
        shortcut: None,
        action: MenuAction::Rename,
    },
    Entry::Item {
        label: "Delete",
        shortcut: None,
        action: MenuAction::Delete,
    },
];

const FILE_ENTRIES: &[Entry] = &[
    Entry::Item {
        label: "Reveal in File Explorer",
        shortcut: None,
        action: MenuAction::RevealInFileExplorer,
    },
    Entry::Item {
        label: "Open in Terminal",
        shortcut: None,
        action: MenuAction::OpenTerminal,
    },
    Entry::Separator,
    Entry::Item {
        label: "Cut",
        shortcut: None,
        action: MenuAction::Cut,
    },
    Entry::Item {
        label: "Copy",
        shortcut: None,
        action: MenuAction::Copy,
    },
    Entry::Separator,
    Entry::Item {
        label: "Copy Path",
        shortcut: None,
        action: MenuAction::CopyPath,
    },
    Entry::Item {
        label: "Copy Relative Path",
        shortcut: None,
        action: MenuAction::CopyRelativePath,
    },
    Entry::Separator,
    Entry::Item {
        label: "Rename",
        shortcut: None,
        action: MenuAction::Rename,
    },
    Entry::Item {
        label: "Delete",
        shortcut: None,
        action: MenuAction::Delete,
    },
];

/// The empty-space menu acts on the root containing the active file. Falling
/// back to the first root keeps its behavior deterministic before a file is
/// opened. The longest match wins when workspace roots are nested.
fn context_root(app: &AppState) -> Option<PathBuf> {
    app.workspace
        .active_path()
        .and_then(|active| {
            app.workspace_folders
                .iter()
                .filter(|root| active.starts_with(root))
                .max_by_key(|root| root.components().count())
        })
        .or_else(|| app.workspace_folders.first())
        .cloned()
}

fn context_target(app: &AppState) -> Option<ExplorerContextTarget> {
    app.context_menu_target.clone().or_else(|| {
        context_root(app).map(|path| ExplorerContextTarget {
            path,
            kind: ExplorerTargetKind::Directory,
        })
    })
}

pub(crate) fn workspace_root_for_path(app: &AppState, path: &Path) -> Option<PathBuf> {
    app.workspace_folders
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
        .cloned()
}

pub(crate) fn copy_target_path(
    state: &State<AppState>,
    clipboard: &dyn Clipboard,
    target: &Path,
    relative: bool,
) {
    let value = if relative {
        let Some(root) = workspace_root_for_path(&state.get(), target) else {
            state.update(|app| app.show_toast("Path is outside the workspace."));
            return;
        };
        let relative = target.strip_prefix(root).unwrap_or(target);
        if relative.as_os_str().is_empty() {
            ".".to_owned()
        } else {
            super::display_path(relative)
        }
    } else {
        super::display_path(target)
    };
    let label = if relative { "relative path" } else { "path" };
    match clipboard.write_text(&value) {
        Ok(()) => state.update(move |app| app.show_toast(format!("Copied {label}."))),
        Err(error) => state.update(move |app| app.show_toast(format!("Clipboard: {error}"))),
    }
}

pub(crate) fn reveal_target(state: &State<AppState>, target: &ExplorerContextTarget) {
    match reveal_in_file_manager(&target.path, target.kind) {
        Ok(()) => {}
        Err(error) => state.update(move |app| {
            app.show_toast(format!("Could not reveal item: {error}"));
        }),
    }
}

#[cfg(target_os = "windows")]
fn reveal_in_file_manager(path: &Path, kind: ExplorerTargetKind) -> std::io::Result<()> {
    let mut command = Command::new("explorer.exe");
    if kind == ExplorerTargetKind::File {
        command.arg(format!("/select,{}", super::display_path(path)));
    } else {
        command.arg(super::display_path(path));
    }
    command.spawn().map(|_| ())
}

#[cfg(target_os = "macos")]
fn reveal_in_file_manager(path: &Path, kind: ExplorerTargetKind) -> std::io::Result<()> {
    let mut command = Command::new("open");
    if kind == ExplorerTargetKind::File {
        command.arg("-R");
    }
    command.arg(path).spawn().map(|_| ())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn reveal_in_file_manager(path: &Path, kind: ExplorerTargetKind) -> std::io::Result<()> {
    let target = if kind == ExplorerTargetKind::File {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    Command::new("xdg-open").arg(target).spawn().map(|_| ())
}

/// Natural width of `s` at the menu text size. The renderer's generic text
/// measurement does not select the menu's monospace family, so keep the
/// monospace column width as a floor to avoid clipping long labels.
fn measure(s: &str) -> f32 {
    let bounds = UiRect::new(0.0, 0.0, 10_000.0, theme::UI_SIZE);
    let mono_width =
        UnicodeWidthStr::width(s) as f32 * theme::CHAR_W * (theme::UI_SIZE / theme::CODE_SIZE);
    measure_width(s, bounds, theme::UI_SIZE, 400).map_or(mono_width, |width| width.max(mono_width))
}

/// Renders the right-click menu anchored at `pos` (screen coords), flipped and
/// clamped so it stays inside `rect` (the viewport).
pub fn render(
    rect: UiRect,
    pos: (f32, f32),
    state: State<AppState>,
    terminal_tabs: State<TerminalTabs>,
) -> Element {
    let s = state.get();
    let target = context_target(&s);
    let has_item_target = s.context_menu_target.is_some();
    let entries = if target
        .as_ref()
        .is_some_and(|target| target.kind == ExplorerTargetKind::File)
    {
        FILE_ENTRIES
    } else {
        DIRECTORY_ENTRIES
    };
    let can_remove = target.as_ref().is_some_and(|target| {
        target.kind == ExplorerTargetKind::Directory && s.workspace_folders.contains(&target.path)
    });

    // Size the menu to its widest entry (label + optional shortcut).
    let content_w = entries
        .iter()
        .map(|e| match e {
            Entry::Item {
                label, shortcut, ..
            } => {
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
        + entries
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
                    app.context_menu_target = None;
                    app.context_menu_hover = None;
                });
            }
        });

    // Menu surface. A capture-phase move clears the hover highlight whenever
    // the pointer is over the surface but not on an item (padding, separators).
    let st_clear = state.clone();
    let mut surface = theme::bordered(
        card,
        theme::c().surface,
        theme::c().border,
        6.0,
        MENU_BORDER,
    )
    .shadow(
        ShadowStyle::new(Color::BLACK)
            .alpha(theme::c().shadow_alpha)
            .offset(0.0, 2.0)
            .blur(3.0),
    )
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
    for entry in entries {
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
                    VisualStyle::filled(theme::c().border),
                ));
                y += SEP_H;
            }
            Entry::Item {
                label,
                shortcut,
                action,
            } => {
                let enabled =
                    action_enabled(*action, target.is_some(), has_item_target, can_remove);
                let item_rect = UiRect::new(
                    card.left + MENU_BORDER,
                    y,
                    card.right - MENU_BORDER,
                    y + ITEM_H,
                );
                let hovered = enabled && s.context_menu_hover == Some(index);
                let idx = index;
                let label = *label;
                let action = *action;

                let st_hover = state.clone();
                let st_click = state.clone();
                let terminal_tabs_click = terminal_tabs.clone();
                let action_target = target.clone();
                let mut item = panel(
                    item_rect,
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
                .on_event(UiEventKind::PointerMove, move |_cx, _p| {
                    st_hover.try_update(move |app| {
                        let changed = if enabled {
                            app.context_menu_hover != Some(idx)
                        } else {
                            app.context_menu_hover.is_some()
                        };
                        if enabled {
                            app.context_menu_hover = Some(idx);
                        } else {
                            app.context_menu_hover = None;
                        }
                        changed
                    });
                })
                .on_click(move |cx: &mut lgui::core::UiEventContext| {
                    if !enabled {
                        return;
                    }
                    st_click.update(move |app| {
                        app.context_menu = None;
                        app.context_menu_target = None;
                        app.context_menu_hover = None;
                    });
                    match action {
                        MenuAction::NewFile | MenuAction::NewFolder => {
                            if let Some(parent) =
                                action_target.as_ref().map(|target| target.path.clone())
                            {
                                let kind = if matches!(action, MenuAction::NewFile) {
                                    ExplorerCreateKind::File
                                } else {
                                    ExplorerCreateKind::Folder
                                };
                                workspace_actions::begin_explorer_create(&st_click, parent, kind);
                            }
                        }
                        MenuAction::OpenTerminal => {
                            let cwd = action_target.as_ref().and_then(|target| match target.kind {
                                ExplorerTargetKind::File => {
                                    target.path.parent().map(Path::to_path_buf)
                                }
                                ExplorerTargetKind::Directory => Some(target.path.clone()),
                            });
                            if let Some(cwd) = cwd {
                                let shell = st_click.get().default_shell;
                                terminal_tabs_click.update(move |tabs| {
                                    tabs.add_at(shell, Some(cwd));
                                });
                                st_click.update(|app| {
                                    app.show_terminal = true;
                                    app.terminal_shell_menu = false;
                                });
                            }
                        }
                        MenuAction::Cut | MenuAction::Copy => {
                            if let Some(target) = action_target.as_ref() {
                                workspace_actions::copy_explorer_item(
                                    &st_click,
                                    &target.path,
                                    matches!(action, MenuAction::Cut),
                                );
                            }
                        }
                        MenuAction::Rename => {
                            if let Some(target) = action_target.clone() {
                                workspace_actions::begin_explorer_rename(
                                    &st_click,
                                    target.path,
                                    target.kind,
                                );
                            }
                        }
                        MenuAction::Delete => {
                            if let Some(target) = action_target.clone() {
                                workspace_actions::delete_explorer_item(&st_click, target.path);
                            }
                        }
                        MenuAction::AddFolder => {
                            workspace_actions::choose_folder_to_add(&st_click);
                        }
                        MenuAction::RemoveFolder => {
                            if let Some(root) = action_target.clone() {
                                workspace_actions::remove_folder_from_workspace(
                                    &st_click, root.path,
                                );
                            }
                        }
                        MenuAction::RevealInFileExplorer => {
                            if let Some(target) = action_target.as_ref() {
                                reveal_target(&st_click, target);
                            }
                        }
                        MenuAction::CopyPath => {
                            if let Some(target) = action_target.as_ref() {
                                copy_target_path(
                                    &st_click,
                                    cx.application().clipboard().as_ref(),
                                    &target.path,
                                    false,
                                );
                            }
                        }
                        MenuAction::CopyRelativePath => {
                            if let Some(target) = action_target.as_ref() {
                                copy_target_path(
                                    &st_click,
                                    cx.application().clipboard().as_ref(),
                                    &target.path,
                                    true,
                                );
                            }
                        }
                    }
                });

                // Label (left-aligned).
                let label_rect = UiRect::new(
                    item_rect.left + ITEM_PAD_X,
                    item_rect.top,
                    item_rect.right - ITEM_PAD_X,
                    item_rect.bottom,
                );
                let label_color = if hovered {
                    theme::c().text_bright
                } else if !enabled {
                    theme::c().text_dim
                } else {
                    theme::c().text_soft
                };
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
                        theme::mono_right(theme::c().text_dim, theme::UI_SIZE),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_root_falls_back_to_the_first_workspace_folder() {
        let mut app = AppState::new();
        app.workspace_folders = vec![PathBuf::from("first"), PathBuf::from("second")];

        assert_eq!(context_root(&app), Some(PathBuf::from("first")));
    }

    #[test]
    fn context_root_prefers_the_deepest_root_containing_the_active_file() {
        let mut app = AppState::new();
        app.workspace_folders = vec![
            PathBuf::from("workspace"),
            PathBuf::from("workspace/nested"),
        ];
        app.workspace
            .open_path(PathBuf::from("workspace/nested/src/main.rs"), String::new());

        assert_eq!(context_root(&app), Some(PathBuf::from("workspace/nested")));
    }

    #[test]
    fn only_add_folder_is_enabled_without_a_workspace() {
        assert!(action_enabled(MenuAction::AddFolder, false, false, false));
        assert!(!action_enabled(
            MenuAction::RevealInFileExplorer,
            false,
            false,
            false
        ));
        assert!(!action_enabled(MenuAction::CopyPath, false, false, false));
        assert!(!action_enabled(
            MenuAction::CopyRelativePath,
            false,
            false,
            false
        ));
        assert!(!action_enabled(MenuAction::NewFile, false, false, false));
        assert!(!action_enabled(MenuAction::NewFolder, false, false, false));
        assert!(!action_enabled(
            MenuAction::OpenTerminal,
            false,
            false,
            false
        ));
        assert!(!action_enabled(
            MenuAction::RemoveFolder,
            true,
            false,
            false
        ));
        assert!(action_enabled(MenuAction::RemoveFolder, true, true, true));
        assert!(!action_enabled(MenuAction::Rename, true, false, false));
        assert!(action_enabled(MenuAction::Rename, true, true, false));
    }

    #[test]
    fn an_explicit_directory_target_takes_priority_over_the_background_root() {
        let mut app = AppState::new();
        app.workspace_folders = vec![PathBuf::from("workspace")];
        app.context_menu_target = Some(ExplorerContextTarget {
            path: PathBuf::from("workspace/src"),
            kind: ExplorerTargetKind::Directory,
        });

        assert_eq!(
            context_target(&app),
            Some(ExplorerContextTarget {
                path: PathBuf::from("workspace/src"),
                kind: ExplorerTargetKind::Directory,
            })
        );
        assert_eq!(
            workspace_root_for_path(&app, Path::new("workspace/src")),
            Some(PathBuf::from("workspace"))
        );
    }

    #[test]
    fn file_menu_contains_only_file_actions() {
        let labels = FILE_ENTRIES
            .iter()
            .filter_map(|entry| match entry {
                Entry::Item { label, .. } => Some(*label),
                Entry::Separator => None,
            })
            .collect::<Vec<_>>();

        assert!(labels.contains(&"Reveal in File Explorer"));
        assert!(labels.contains(&"Cut"));
        assert!(labels.contains(&"Copy"));
        assert!(labels.contains(&"Rename"));
        assert!(labels.contains(&"Delete"));
        assert!(!labels.contains(&"New File"));
        assert!(!labels.contains(&"Add Folder From Workspace"));
        assert!(!labels.contains(&"Remove Folder From Workspace"));
    }

    #[test]
    fn long_menu_labels_reserve_at_least_their_monospace_width() {
        let label = "Remove Folder From Workspace";
        let mono_width = UnicodeWidthStr::width(label) as f32
            * theme::CHAR_W
            * (theme::UI_SIZE / theme::CODE_SIZE);

        assert!(measure(label) >= mono_width);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn copied_windows_paths_hide_the_verbatim_prefix() {
        assert_eq!(
            crate::ui::display_path(Path::new(r"\\?\D:\Documents\project")),
            r"D:\Documents\project"
        );
        assert_eq!(
            crate::ui::display_path(Path::new(r"\\?\UNC\server\share\project")),
            r"\\server\share\project"
        );
        assert_eq!(
            crate::ui::display_path(Path::new(r"D:\Documents\project")),
            r"D:\Documents\project"
        );
    }
}
