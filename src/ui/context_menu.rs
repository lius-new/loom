//! Empty-space context menu overlay for the file drawer.
//!
//! Right-clicking the empty area of the drawer opens a small menu of the
//! actions that make sense in that context: create new items, open a terminal,
//! or add folders to the project. Items are grouped by hairline separators.

use std::path::{Path, PathBuf};
use std::process::Command;

use lgui::core::{CursorIcon, EventPolicy, UiEventKind};
use lgui::prelude::{Color, Element, ShadowStyle, State, UiRect, VisualStyle, panel, text};
use lgui::services::{Clipboard, ServicesContextExt};
use lgui::text::measure_width;
use unicode_width::UnicodeWidthStr;

use crate::state::{AppState, ExplorerCreateKind};
use crate::terminal_session::{ShellKind, TerminalTabs};
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
    AddFolder,
    RemoveFolder,
    RevealInFileExplorer,
    CopyPath,
    CopyRelativePath,
}

fn action_enabled(action: MenuAction, has_target: bool, can_remove: bool) -> bool {
    match action {
        MenuAction::AddFolder => true,
        MenuAction::RemoveFolder => can_remove,
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

const ENTRIES: &[Entry] = &[
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

fn context_target(app: &AppState) -> Option<PathBuf> {
    app.context_menu_target
        .clone()
        .or_else(|| context_root(app))
}

fn workspace_root_for_path(app: &AppState, path: &Path) -> Option<PathBuf> {
    app.workspace_folders
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
        .cloned()
}

fn copy_target_path(
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
            clipboard_path(relative)
        }
    } else {
        clipboard_path(target)
    };
    let label = if relative { "relative path" } else { "path" };
    match clipboard.write_text(&value) {
        Ok(()) => state.update(move |app| app.show_toast(format!("Copied {label}."))),
        Err(error) => state.update(move |app| app.show_toast(format!("Clipboard: {error}"))),
    }
}

#[cfg(target_os = "windows")]
fn clipboard_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(local) = value.strip_prefix(r"\\?\") {
        local.to_owned()
    } else {
        value.into_owned()
    }
}

#[cfg(not(target_os = "windows"))]
fn clipboard_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn reveal_target(state: &State<AppState>, target: &Path) {
    match reveal_in_file_manager(target) {
        Ok(()) => {}
        Err(error) => state.update(move |app| {
            app.show_toast(format!("Could not reveal folder: {error}"));
        }),
    }
}

#[cfg(target_os = "windows")]
fn reveal_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
    Command::new("explorer.exe").arg(path).spawn().map(|_| ())
}

#[cfg(target_os = "macos")]
fn reveal_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
    Command::new("open").arg(path).spawn().map(|_| ())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn reveal_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
    Command::new("xdg-open").arg(path).spawn().map(|_| ())
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
    let can_remove = target
        .as_ref()
        .is_some_and(|path| s.workspace_folders.contains(path));

    // Size the menu to its widest entry (label + optional shortcut).
    let content_w = ENTRIES
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
                    app.context_menu_target = None;
                    app.context_menu_hover = None;
                });
            }
        });

    // Menu surface. A capture-phase move clears the hover highlight whenever
    // the pointer is over the surface but not on an item (padding, separators).
    let st_clear = state.clone();
    let mut surface = theme::bordered(card, theme::SURFACE, theme::BORDER, 6.0, MENU_BORDER)
        .shadow(
            ShadowStyle::new(Color::BLACK)
                .alpha(80)
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
            Entry::Item {
                label,
                shortcut,
                action,
            } => {
                let enabled = action_enabled(*action, target.is_some(), can_remove);
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
                        VisualStyle::filled(theme::SELECTION)
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
                            if let Some(parent) = action_target.clone() {
                                let kind = if matches!(action, MenuAction::NewFile) {
                                    ExplorerCreateKind::File
                                } else {
                                    ExplorerCreateKind::Folder
                                };
                                workspace_actions::begin_explorer_create(&st_click, parent, kind);
                            }
                        }
                        MenuAction::OpenTerminal => {
                            if let Some(cwd) = action_target.clone() {
                                terminal_tabs_click.update(move |tabs| {
                                    tabs.add_at(ShellKind::default(), Some(cwd));
                                });
                                st_click.update(|app| {
                                    app.show_terminal = true;
                                    app.terminal_shell_menu = false;
                                });
                            }
                        }
                        MenuAction::AddFolder => {
                            workspace_actions::choose_folder_to_add(&st_click);
                        }
                        MenuAction::RemoveFolder => {
                            if let Some(root) = action_target.clone() {
                                workspace_actions::remove_folder_from_workspace(&st_click, root);
                            }
                        }
                        MenuAction::RevealInFileExplorer => {
                            if let Some(target) = action_target.as_deref() {
                                reveal_target(&st_click, target);
                            }
                        }
                        MenuAction::CopyPath => {
                            if let Some(target) = action_target.as_deref() {
                                copy_target_path(
                                    &st_click,
                                    cx.application().clipboard().as_ref(),
                                    target,
                                    false,
                                );
                            }
                        }
                        MenuAction::CopyRelativePath => {
                            if let Some(target) = action_target.as_deref() {
                                copy_target_path(
                                    &st_click,
                                    cx.application().clipboard().as_ref(),
                                    target,
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
                    theme::ZINC_100
                } else if !enabled {
                    theme::ZINC_500
                } else {
                    theme::ZINC_300
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
        assert!(action_enabled(MenuAction::AddFolder, false, false));
        assert!(!action_enabled(
            MenuAction::RevealInFileExplorer,
            false,
            false
        ));
        assert!(!action_enabled(MenuAction::CopyPath, false, false));
        assert!(!action_enabled(
            MenuAction::CopyRelativePath,
            false,
            false
        ));
        assert!(!action_enabled(MenuAction::NewFile, false, false));
        assert!(!action_enabled(MenuAction::NewFolder, false, false));
        assert!(!action_enabled(MenuAction::OpenTerminal, false, false));
        assert!(!action_enabled(MenuAction::RemoveFolder, true, false));
        assert!(action_enabled(MenuAction::RemoveFolder, true, true));
    }

    #[test]
    fn an_explicit_directory_target_takes_priority_over_the_background_root() {
        let mut app = AppState::new();
        app.workspace_folders = vec![PathBuf::from("workspace")];
        app.context_menu_target = Some(PathBuf::from("workspace/src"));

        assert_eq!(context_target(&app), Some(PathBuf::from("workspace/src")));
        assert_eq!(
            workspace_root_for_path(&app, Path::new("workspace/src")),
            Some(PathBuf::from("workspace"))
        );
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
            clipboard_path(Path::new(r"\\?\D:\Documents\project")),
            r"D:\Documents\project"
        );
        assert_eq!(
            clipboard_path(Path::new(r"\\?\UNC\server\share\project")),
            r"\\server\share\project"
        );
        assert_eq!(
            clipboard_path(Path::new(r"D:\Documents\project")),
            r"D:\Documents\project"
        );
    }
}
