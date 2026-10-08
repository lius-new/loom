//! File drawer: an empty-state prompt when no folder is open, or the folders'
//! file tree when one is. Also owns the right-click capture and resize handle.
//!
//! The tree is lazy: opening a folder reads only its top level, and a
//! sub-directory is read from disk only when first expanded. Renders read from
//! the cached listings, so no filesystem I/O happens per frame. The opened
//! folders themselves are shown as root nodes (expanded by default) with their
//! contents indented beneath it.

use std::fs;
use std::path::{Path, PathBuf};

use lgui::core::{
    CursorIcon, EventPolicy, IconStyle, PointerButton, UiElement, UiEventKind, UiEventPayload,
    UiFocusHandle, UiId, WheelUnit, clip, precompiled,
};
use lgui::prelude::{Color, Element, State, TextAlign, UiRect, VisualStyle, panel, text};

use crate::file_tree::read_directory;
use crate::git::{GitStoreSnapshot, PathDecoration};
#[cfg(test)]
use crate::state::DirEntry;
use crate::state::{
    AppState, ExplorerContextTarget, ExplorerCreateKind, ExplorerTargetKind, FileDragState,
};
use crate::theme;
use crate::ui::components::input::{self, InputBinding, InputOptions, InputState, InputStyle};
use crate::ui::components::scrollbar;
use crate::workspace_actions;

const HEADER_H: f32 = theme::TABS_H;
const ROW_H: f32 = 20.0;
const CREATE_ROW_H: f32 = 28.0;
const INDENT: f32 = 12.0;
const TREE_ICON_SIZE: f32 = 16.0;
const TREE_ICON_GAP: f32 = 4.0;
const TREE_LABEL_OFFSET: f32 = TREE_ICON_SIZE + TREE_ICON_GAP;
const MAX_EDITABLE_FILE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
struct StickyDirectory {
    key: String,
    name: String,
    depth: usize,
    source_y: f32,
    is_expanded: bool,
}

#[derive(Clone, Debug)]
struct TreeRowMeta {
    y: f32,
    depth: usize,
    sticky_path: Vec<StickyDirectory>,
}

#[derive(Clone, Debug)]
struct StickyRow {
    directory: StickyDirectory,
    top: f32,
}

pub fn render(
    rect: UiRect,
    state: State<AppState>,
    git_store: State<GitStoreSnapshot>,
    editor_focus: UiFocusHandle,
    create_input_focus: UiFocusHandle,
    create_input_id: UiId,
) -> Element {
    let s = state.get();
    let git = git_store.get();
    let header_rect = UiRect::new(rect.left, rect.top, rect.right, rect.top + HEADER_H);
    let content_rect = UiRect::new(rect.left, header_rect.bottom, rect.right, rect.bottom);

    let mut bar = panel(rect, VisualStyle::filled(theme::c().sidebar))
        .child(drawer_header(header_rect, state.clone()));

    // Background right-click capture: covers the drawer at the lowest z-order.
    // Rows and the resize handle are drawn later and sit above it, so they win
    // hit-testing and this only receives events on empty space.
    let st_menu = state.clone();
    let clear_hover = state.clone();
    let capture = panel(content_rect, VisualStyle::default())
        .event_policy(EventPolicy::INTERACTIVE)
        .on_event_capture(UiEventKind::PointerMove, move |_cx, _payload| {
            clear_hover.try_update(|app| {
                let changed = app.tree_hovered_path.is_some();
                app.tree_hovered_path = None;
                changed
            });
        })
        .on_pointer_down_with_button(move |_cx, p, button| {
            if button == PointerButton::Right {
                st_menu.update(move |app| {
                    app.tab_context_menu = None;
                    app.context_menu = Some((p.point.x, p.point.y));
                    app.context_menu_target = None;
                });
            }
        });
    bar = bar.child(capture);

    // Content: empty-state prompt, or all root folders in the current project.
    if s.workspace_folders.is_empty() {
        for el in empty_state(content_rect, state.clone()) {
            bar = bar.child(el);
        }
    } else {
        let roots = s
            .workspace_folders
            .iter()
            .map(|dir| {
                let key = dir.to_string_lossy().into_owned();
                let name = dir
                    .file_name()
                    .map(|value| value.to_string_lossy().into_owned())
                    .unwrap_or_else(|| key.clone());
                (dir.clone(), key, name)
            })
            .collect::<Vec<_>>();
        let content_right = roots
            .iter()
            .map(|(_, key, name)| tree_content_right(key, name, &s, content_rect.left))
            .fold(content_rect.right, f32::max);
        let max_scroll_x = (content_right - content_rect.right).max(0.0);
        let scroll_x = s.tree_scroll_x.clamp(0.0, max_scroll_x);

        let mut y = content_rect.top;
        let indent = content_rect.left + INDENT;
        let mut rows = Vec::new();
        let mut tree_els = Vec::new();

        // Collect every root and its descendants into one scrollable list.
        // Each root starts a fresh sticky ancestry so adjacent workspaces push
        // one another out at the top edge instead of nesting visually.
        for (root_path, key, root_name) in roots {
            let is_expanded = s.expanded.contains(&key);
            let root = StickyDirectory {
                key: key.clone(),
                name: root_name.clone(),
                depth: 0,
                source_y: y,
                is_expanded,
            };
            let mut path = vec![root];
            rows.push(TreeRowMeta {
                y,
                depth: 0,
                sticky_path: path.clone(),
            });
            if s.explorer_rename
                .as_ref()
                .is_some_and(|request| request.path == root_path)
            {
                tree_els.push(rename_row(
                    content_rect,
                    y,
                    indent,
                    is_expanded,
                    state.clone(),
                    create_input_focus.clone(),
                    create_input_id.clone(),
                ));
            } else {
                tree_els.push(dir_row(
                    &key,
                    &root_name,
                    indent,
                    content_rect,
                    y,
                    content_right,
                    0.0,
                    is_expanded,
                    false,
                    s.tree_hovered_path.as_deref() == Some(key.as_str()),
                    theme::c().text_soft,
                    &state,
                ));
            }
            y += ROW_H;

            if s.explorer_create
                .as_ref()
                .is_some_and(|request| request.parent == root_path)
            {
                tree_els.push(create_row(
                    content_rect,
                    y,
                    indent,
                    state.clone(),
                    create_input_focus.clone(),
                    create_input_id.clone(),
                ));
                y += CREATE_ROW_H;
            }

            if is_expanded {
                let children_top = y;
                let guide_x = indent + TREE_ICON_SIZE / 2.0;
                tree_els.extend(build_tree(
                    &key,
                    content_rect,
                    &mut y,
                    1,
                    &s,
                    &state,
                    &mut path,
                    &mut rows,
                    content_right,
                    &git,
                    &editor_focus,
                    &create_input_focus,
                    &create_input_id,
                ));
                if y > children_top {
                    tree_els.push(panel(
                        UiRect::new(guide_x, children_top, guide_x + 1.0, y),
                        VisualStyle::filled(theme::c().border),
                    ));
                }
            }
        }
        let content_bottom = y;

        let max_scroll = (content_bottom - content_rect.bottom).max(0.0);
        let scroll = s.tree_scroll.clamp(0.0, max_scroll);

        let st = state.clone();
        bar = bar.on_event(UiEventKind::Wheel, move |_cx, payload| {
            if let UiEventPayload::Wheel { delta } = payload {
                let (step_x, step_y) = match delta.unit {
                    WheelUnit::Lines => (delta.x * ROW_H * 3.0, delta.y * ROW_H * 3.0),
                    WheelUnit::Pixels => (delta.x, delta.y),
                };
                st.update(move |app| {
                    app.tree_scroll = (app.tree_scroll - step_y).clamp(0.0, max_scroll);
                    app.tree_scroll_x = (app.tree_scroll_x - step_x).clamp(0.0, max_scroll_x);
                });
            }
        });

        let mut clip_el = clip(content_rect, -scroll_x, -scroll);
        for el in tree_els {
            clip_el = clip_el.child(el);
        }
        bar = bar.child(clip_el);

        for sticky in sticky_rows(&rows, content_rect.top, scroll)
            .into_iter()
            .rev()
        {
            let directory = sticky.directory;
            let indent = content_rect.left + INDENT + directory.depth as f32 * INDENT;
            // The workspace root keeps its plain color, as in the tree itself.
            let label_color = if directory.depth == 0 {
                theme::c().text_soft
            } else {
                tree_label_color(&git, Path::new(&directory.key), theme::c().text_soft)
            };
            bar = bar.child(dir_row(
                &directory.key,
                &directory.name,
                indent,
                content_rect,
                sticky.top,
                content_right,
                -scroll_x,
                directory.is_expanded,
                true,
                false,
                label_color,
                &state,
            ));
        }

        if max_scroll > 0.0 && (s.sidebar_hovered || s.scrollbar_dragging) {
            bar = bar.child(vertical_scrollbar(
                content_rect,
                content_bottom,
                scroll,
                state.clone(),
            ));
        }
        if max_scroll_x > 0.0 && (s.sidebar_hovered || s.horizontal_scrollbar_dragging) {
            bar = bar.child(horizontal_scrollbar(
                content_rect,
                content_right,
                scroll_x,
                state.clone(),
            ));
        }
    }

    // Resize handle on the drawer's left edge. An 8px invisible hit strip
    // (drawn last, above the file rows) carries the drag; the 1px filled panel
    // inside it is the visible divider.
    let handle_rect = UiRect::new(rect.left, rect.top, rect.left + 8.0, rect.bottom);
    let win_right = rect.right;
    let st_down = state.clone();
    let st_move = state.clone();
    let st_up = state.clone();
    let handle = panel(handle_rect, VisualStyle::default())
        .key("file-tree-resize-handle")
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::ResizeHorizontal)
        .on_pointer_down_with_button(move |_cx, _p, button| {
            if button == PointerButton::Left {
                st_down.update(move |app| app.resizing_sidebar = true);
            }
        })
        .on_pointer_drag(move |_cx, p| {
            st_move.update(move |app| {
                if app.resizing_sidebar {
                    app.sidebar_w = resized_sidebar_width(win_right, p.point.x);
                }
            });
        })
        .on_pointer_up(move |_cx, _p| {
            st_up.update(move |app| app.resizing_sidebar = false);
        })
        .child(panel(
            UiRect::new(rect.left, rect.top, rect.left + 1.0, rect.bottom),
            VisualStyle::filled(if s.resizing_sidebar {
                theme::c().accent
            } else {
                theme::c().border
            }),
        ));
    bar = bar.child(handle);

    bar
}

fn resized_sidebar_width(window_right: f32, pointer_x: f32) -> f32 {
    (window_right - pointer_x).clamp(theme::SIDEBAR_MIN_W, theme::SIDEBAR_MAX_W)
}

pub(super) fn drawer_icon(
    id: &'static str,
    key: &'static str,
    rect: UiRect,
    color: lgui::prelude::Color,
) -> Element {
    precompiled(UiElement::icon(UiId::new(id), rect, key).icon_style(IconStyle::new(color)))
}

fn drawer_header(rect: UiRect, state: State<AppState>) -> Element {
    let mut header = panel(rect, VisualStyle::filled(theme::c().sidebar));
    let icon_top = rect.top + (rect.height() - 14.0) / 2.0;
    let explorer_icon = UiRect::new(
        rect.left + 12.0,
        icon_top,
        rect.left + 26.0,
        icon_top + 14.0,
    );
    header = header.child(drawer_icon(
        "sidebar.explorer",
        "explorer",
        explorer_icon,
        theme::c().text_dim,
    ));
    header = header.child(text(
        UiRect::new(
            explorer_icon.right + 7.0,
            rect.top,
            rect.right - 40.0,
            rect.bottom,
        ),
        "EXPLORER",
        theme::mono_bold(theme::c().text_soft, theme::SMALL).tracking(0.8),
    ));

    let collapse_hit = UiRect::new(
        rect.right - 32.0,
        rect.top + 2.0,
        rect.right - 4.0,
        rect.bottom,
    );
    let collapse_icon = UiRect::new(
        collapse_hit.left + 7.0,
        icon_top,
        collapse_hit.left + 21.0,
        icon_top + 14.0,
    );
    let collapse_state = state;
    header = header.child(
        panel(collapse_hit, VisualStyle::default().radius(3.0))
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(CursorIcon::Pointer)
            .on_click(move || {
                collapse_state.update(|app| {
                    app.show_drawer = false;
                    app.sidebar_hovered = false;
                    app.tree_hovered_path = None;
                });
            })
            .child(drawer_icon(
                "sidebar.collapse.icon",
                "panel-left",
                collapse_icon,
                theme::c().text_dim,
            )),
    );

    header.child(panel(
        UiRect::new(rect.left, rect.bottom - 1.0, rect.right, rect.bottom),
        VisualStyle::filled(theme::c().border),
    ))
}

/// Empty state from the Explorer mockup, without its recent-workspace and
/// bottom workspace-switching sections.
fn empty_state(rect: UiRect, state: State<AppState>) -> Vec<Element> {
    let cx = (rect.left + rect.right) / 2.0;
    let icon_rect = UiRect::new(cx - 24.0, rect.top + 28.0, cx + 24.0, rect.top + 76.0);
    let icon = theme::bordered(icon_rect, theme::c().surface, theme::c().border, 10.0, 1.0).child(
        drawer_icon(
            "sidebar.empty.folder",
            crate::file_icons::FOLDER_ICON,
            UiRect::new(
                icon_rect.left + 12.0,
                icon_rect.top + 12.0,
                icon_rect.right - 12.0,
                icon_rect.bottom - 12.0,
            ),
            theme::c().text_dim,
        ),
    );

    let mut centered_title = theme::sans_semibold(theme::c().text, theme::UI_SIZE);
    centered_title.align = TextAlign::Center;
    let title = text(
        UiRect::new(
            rect.left + 14.0,
            rect.top + 88.0,
            rect.right - 14.0,
            rect.top + 108.0,
        ),
        "No Folder Opened",
        centered_title,
    );

    let mut centered_desc = theme::sans(theme::c().text_dim, theme::SMALL);
    centered_desc.align = TextAlign::Center;
    let desc_line_one = text(
        UiRect::new(
            rect.left + 14.0,
            rect.top + 111.0,
            rect.right - 14.0,
            rect.top + 128.0,
        ),
        "Open a directory to explore files,",
        centered_desc,
    );
    let desc_line_two = text(
        UiRect::new(
            rect.left + 14.0,
            rect.top + 127.0,
            rect.right - 14.0,
            rect.top + 144.0,
        ),
        "track changes, and start editing.",
        centered_desc,
    );

    let btn_rect = UiRect::new(
        rect.left + 14.0,
        rect.top + 164.0,
        rect.right - 14.0,
        rect.top + 198.0,
    );
    let st = state.clone();
    let btn = theme::bordered(btn_rect, theme::c().surface, theme::c().border, 4.0, 1.0)
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            workspace_actions::choose_folder(&st);
        })
        .child(drawer_icon(
            "sidebar.empty.open-folder",
            crate::file_icons::FOLDER_OPEN_ICON,
            UiRect::new(
                btn_rect.left + 12.0,
                btn_rect.top + 10.0,
                btn_rect.left + 26.0,
                btn_rect.top + 24.0,
            ),
            theme::c().accent,
        ))
        .child(text(
            UiRect::new(
                btn_rect.left + 34.0,
                btn_rect.top,
                btn_rect.right - 12.0,
                btn_rect.bottom,
            ),
            "Open Folder",
            theme::sans_semibold(theme::c().text, theme::UI_SIZE),
        ));

    let clone_rect = UiRect::new(
        rect.left + 14.0,
        rect.top + 202.0,
        rect.right - 14.0,
        rect.top + 232.0,
    );
    let clone_state = state;
    let clone_btn = panel(clone_rect, VisualStyle::default().radius(4.0))
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            clone_state.update(|app| {
                app.show_clone_dialog = true;
                app.clone_repository_error = None;
            });
        })
        .child(drawer_icon(
            "sidebar.empty.clone",
            "git-branch",
            UiRect::new(
                clone_rect.left + 12.0,
                clone_rect.top + 8.0,
                clone_rect.left + 26.0,
                clone_rect.top + 22.0,
            ),
            theme::c().text_faint,
        ))
        .child(text(
            UiRect::new(
                clone_rect.left + 34.0,
                clone_rect.top,
                clone_rect.right - 92.0,
                clone_rect.bottom,
            ),
            "Clone Repository...",
            theme::sans(theme::c().text_muted, theme::UI_SIZE),
        ))
        .child(text(
            UiRect::new(
                clone_rect.right - 86.0,
                clone_rect.top,
                clone_rect.right - 12.0,
                clone_rect.bottom,
            ),
            "Ctrl+Shift+G",
            theme::mono_right(theme::c().text_faint, 9.0),
        ));

    vec![
        icon,
        title.into(),
        desc_line_one.into(),
        desc_line_two.into(),
        btn,
        clone_btn,
    ]
}

fn read_text_file(path: &Path) -> Result<String, String> {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let metadata = fs::metadata(path).map_err(|error| format!("Could not open {name}: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("Could not open {name}: not a regular file"));
    }
    if metadata.len() > MAX_EDITABLE_FILE_BYTES {
        return Err(format!("Could not open {name}: file is larger than 4 MiB"));
    }
    fs::read_to_string(path).map_err(|error| format!("Could not open {name} as UTF-8: {error}"))
}

fn create_row(
    rect: UiRect,
    y: f32,
    indent: f32,
    state: State<AppState>,
    focus: UiFocusHandle,
    id: UiId,
) -> Element {
    let snapshot = state.get();
    let kind = snapshot
        .explorer_create
        .as_ref()
        .map_or(ExplorerCreateKind::File, |request| request.kind);
    let border = if snapshot.explorer_create_error.is_some() {
        theme::c().error
    } else if snapshot.explorer_create_input.focused {
        theme::c().accent
    } else {
        theme::c().border
    };
    let icon = if kind == ExplorerCreateKind::Folder {
        crate::file_icons::FOLDER_ICON
    } else {
        crate::file_icons::DEFAULT_FILE_ICON
    };
    let input_rect = UiRect::new(
        indent + TREE_LABEL_OFFSET,
        y + 2.0,
        rect.right - 8.0,
        y + CREATE_ROW_H - 2.0,
    );
    let field_rect = UiRect::new(
        input_rect.left + 1.0,
        input_rect.top + 1.0,
        input_rect.right - 1.0,
        input_rect.bottom - 1.0,
    );
    let input = input::render(
        field_rect,
        id,
        focus,
        InputBinding::new(state, explorer_create_input, explorer_create_input_mut),
        InputOptions {
            label: if kind == ExplorerCreateKind::Folder {
                "New folder name"
            } else {
                "New file name"
            },
            placeholder: "",
            style: InputStyle {
                text: theme::mono(theme::c().text, theme::UI_SIZE),
                placeholder: theme::mono(theme::c().text_faint, theme::UI_SIZE),
                caret: theme::c().text,
                selection: theme::c().accent,
            },
            on_submit: Some(workspace_actions::finish_explorer_create),
            on_cancel: Some(workspace_actions::cancel_explorer_create),
            on_blur: Some(workspace_actions::cancel_explorer_create),
        },
    );

    panel(
        UiRect::new(rect.left, y, rect.right, y + CREATE_ROW_H),
        VisualStyle::filled(theme::c().surface),
    )
    .child(precompiled(
        UiElement::icon(
            UiId::owned("tree-create-icon"),
            UiRect::new(
                indent,
                y + 6.0,
                indent + TREE_ICON_SIZE,
                y + 6.0 + TREE_ICON_SIZE,
            ),
            icon,
        )
        .icon_style(IconStyle::new(theme::c().text_muted)),
    ))
    .child(theme::bordered(input_rect, theme::c().bg, border, 3.0, 1.0))
    .child(input)
}

fn rename_row(
    rect: UiRect,
    y: f32,
    indent: f32,
    is_expanded: bool,
    state: State<AppState>,
    focus: UiFocusHandle,
    id: UiId,
) -> Element {
    let snapshot = state.get();
    let border = if snapshot.explorer_create_error.is_some() {
        theme::c().error
    } else if snapshot.explorer_create_input.focused {
        theme::c().accent
    } else {
        theme::c().border
    };
    let input_rect = UiRect::new(
        indent + TREE_LABEL_OFFSET,
        y + 2.0,
        rect.right - 8.0,
        y + ROW_H - 2.0,
    );
    let field_rect = UiRect::new(
        input_rect.left + 1.0,
        input_rect.top + 1.0,
        input_rect.right - 1.0,
        input_rect.bottom - 1.0,
    );
    let (label, icon) = match snapshot
        .explorer_rename
        .as_ref()
        .map(|request| request.kind)
    {
        Some(ExplorerTargetKind::File) => {
            let icon = snapshot
                .explorer_rename
                .as_ref()
                .and_then(|request| request.path.file_name())
                .map(|name| crate::file_icons::icon_for_file(&name.to_string_lossy()))
                .unwrap_or(crate::file_icons::DEFAULT_FILE_ICON);
            ("Rename file", icon)
        }
        _ => (
            "Rename folder",
            if is_expanded {
                crate::file_icons::FOLDER_OPEN_ICON
            } else {
                crate::file_icons::FOLDER_ICON
            },
        ),
    };
    let input = input::render(
        field_rect,
        id,
        focus,
        InputBinding::new(state, explorer_create_input, explorer_create_input_mut),
        InputOptions {
            label,
            placeholder: "",
            style: InputStyle {
                text: theme::mono(theme::c().text, theme::UI_SIZE),
                placeholder: theme::mono(theme::c().text_faint, theme::UI_SIZE),
                caret: theme::c().text,
                selection: theme::c().accent,
            },
            on_submit: Some(workspace_actions::finish_explorer_rename),
            on_cancel: Some(workspace_actions::cancel_explorer_create),
            on_blur: Some(workspace_actions::cancel_explorer_create),
        },
    );
    panel(
        UiRect::new(rect.left, y, rect.right, y + ROW_H),
        VisualStyle::filled(theme::c().surface),
    )
    .child(precompiled(
        UiElement::icon(
            UiId::owned("tree-rename-icon"),
            UiRect::new(
                indent,
                y + 2.0,
                indent + TREE_ICON_SIZE,
                y + 2.0 + TREE_ICON_SIZE,
            ),
            icon,
        )
        .icon_style(IconStyle::new(theme::c().text_muted)),
    ))
    .child(theme::bordered(input_rect, theme::c().bg, border, 3.0, 1.0))
    .child(input)
}

fn explorer_create_input(app: &AppState) -> &InputState {
    &app.explorer_create_input
}

fn explorer_create_input_mut(app: &mut AppState) -> &mut InputState {
    &mut app.explorer_create_input
}

/// Explorer label color for `path`: git changes and ignored paths override
/// `default`, using the same precedence as Zed's project panel.
fn tree_label_color(git: &GitStoreSnapshot, path: &Path, default: Color) -> Color {
    let colors = theme::c().git;
    match git.decoration_for_absolute_path(path) {
        Some(PathDecoration::Conflict | PathDecoration::Deleted) => colors.deleted,
        Some(PathDecoration::Modified) => colors.modified,
        Some(PathDecoration::Created) => colors.added,
        Some(PathDecoration::Ignored) => colors.ignored,
        None => default,
    }
}

/// A directory node row: left-click toggles expansion; right-click opens the
/// context menu for this directory.
fn dir_row(
    key: &str,
    name: &str,
    indent: f32,
    rect: UiRect,
    y: f32,
    content_right: f32,
    content_offset_x: f32,
    is_expanded: bool,
    sticky: bool,
    hovered: bool,
    label_color: Color,
    state: &State<AppState>,
) -> Element {
    let row_right = if sticky { rect.right } else { content_right };
    let row_rect = UiRect::new(rect.left, y, row_right, y + ROW_H);
    let indent = indent + content_offset_x;
    let text_right = if sticky {
        rect.right - 12.0
    } else {
        content_right - 12.0
    };
    let icon = if is_expanded {
        crate::file_icons::FOLDER_OPEN_ICON
    } else {
        crate::file_icons::FOLDER_ICON
    };
    let layer = if sticky { "sticky" } else { "content" };
    let fid = UiId::owned(format!("tree-{layer}-{key}"));
    let style = if sticky {
        VisualStyle::filled(theme::c().sidebar)
    } else if hovered {
        VisualStyle::filled(theme::c().surface)
    } else {
        VisualStyle::default()
    };

    let mut row = panel(row_rect, style).event_policy(EventPolicy::INTERACTIVE);
    if !sticky {
        let hover_state = state.clone();
        let hover_key = key.to_string();
        row = row.on_pointer_move(move |_cx, _pointer| {
            let key = hover_key.clone();
            hover_state.try_update(move |app| {
                let changed = app.tree_hovered_path.as_deref() != Some(key.as_str());
                app.tree_hovered_path = Some(key);
                changed
            });
        });
    }

    let click_state = state.clone();
    let click_key = key.to_string();
    row.on_pointer_down_with_button(move |cx, pointer, button| {
        let key = click_key.clone();
        match button {
            PointerButton::Left => {
                click_state.update(move |app| {
                    if app.expanded.contains(&key) {
                        app.expanded.remove(&key);
                    } else {
                        app.expanded.insert(key.clone());
                        if !app.dir_entries.contains_key(&key) {
                            let entries = read_directory(Path::new(&key));
                            app.dir_entries.insert(key.clone(), entries);
                        }
                    }
                });
            }
            PointerButton::Right => {
                click_state.update(move |app| {
                    app.tab_context_menu = None;
                    app.context_menu = Some((pointer.point.x, pointer.point.y));
                    app.context_menu_target = Some(ExplorerContextTarget {
                        path: PathBuf::from(key),
                        kind: ExplorerTargetKind::Directory,
                    });
                    app.context_menu_hover = None;
                });
            }
            _ => return,
        }
        cx.stop_propagation();
    })
    .child(precompiled(
        UiElement::icon(
            fid,
            UiRect::new(
                indent,
                y + 2.0,
                indent + TREE_ICON_SIZE,
                y + 2.0 + TREE_ICON_SIZE,
            ),
            icon,
        )
        .icon_style(IconStyle::new(theme::c().text_muted)),
    ))
    .child(text(
        UiRect::new(indent + TREE_LABEL_OFFSET, y + 2.0, text_right, y + 18.0),
        name.to_string(),
        theme::mono(label_color, theme::UI_SIZE),
    ))
}

/// Renders the cached entries for `dir_key`, advancing `y` by one row per
/// entry. Child directories render only when present in `expanded`.
fn build_tree(
    dir_key: &str,
    rect: UiRect,
    y: &mut f32,
    depth: usize,
    s: &AppState,
    state: &State<AppState>,
    path: &mut Vec<StickyDirectory>,
    rows: &mut Vec<TreeRowMeta>,
    content_right: f32,
    git: &GitStoreSnapshot,
    editor_focus: &UiFocusHandle,
    create_input_focus: &UiFocusHandle,
    create_input_id: &UiId,
) -> Vec<Element> {
    let mut els = Vec::new();

    let Some(entries) = s.dir_entries.get(dir_key) else {
        return els;
    };

    for entry in entries.iter() {
        let is_dir = entry.is_dir;
        let name = entry.name.clone();
        let key = entry.path.to_string_lossy().into_owned();
        let indent = rect.left + INDENT + depth as f32 * INDENT;

        if is_dir {
            let is_expanded = s.expanded.contains(&key);
            path.push(StickyDirectory {
                key: key.clone(),
                name: name.clone(),
                depth,
                source_y: *y,
                is_expanded,
            });
            rows.push(TreeRowMeta {
                y: *y,
                depth,
                sticky_path: path.clone(),
            });
            if s.explorer_rename
                .as_ref()
                .is_some_and(|request| request.path == entry.path)
            {
                els.push(rename_row(
                    rect,
                    *y,
                    indent,
                    is_expanded,
                    state.clone(),
                    create_input_focus.clone(),
                    create_input_id.clone(),
                ));
            } else {
                els.push(dir_row(
                    &key,
                    &name,
                    indent,
                    rect,
                    *y,
                    content_right,
                    0.0,
                    is_expanded,
                    false,
                    s.tree_hovered_path.as_deref() == Some(key.as_str()),
                    tree_label_color(git, &entry.path, theme::c().text_soft),
                    state,
                ));
            }
        } else {
            rows.push(TreeRowMeta {
                y: *y,
                depth,
                sticky_path: path.clone(),
            });
            let row_rect = UiRect::new(rect.left, *y, content_right, *y + ROW_H);
            let icon_id = UiId::owned(format!("tree-file-icon-{key}"));
            let icon = crate::file_icons::icon_for_file(&name);
            if s.explorer_rename
                .as_ref()
                .is_some_and(|request| request.path == entry.path)
            {
                els.push(rename_row(
                    rect,
                    *y,
                    indent,
                    false,
                    state.clone(),
                    create_input_focus.clone(),
                    create_input_id.clone(),
                ));
            } else {
                let file_path = entry.path.clone();
                let open_id = s.workspace.file_id_for_path(&file_path);
                let git_indicator = git
                    .repository_for_path(&file_path)
                    .and_then(|repository| repository.state_for_absolute_path(&file_path))
                    .map(|state| state.display_kind().indicator())
                    .unwrap_or_default();
                let label_color = tree_label_color(git, &file_path, theme::c().text_muted);
                let row_style = if s.workspace.active_path() == Some(file_path.as_path()) {
                    VisualStyle::filled(theme::c().active_line)
                } else if s.tree_hovered_path.as_deref() == Some(key.as_str()) {
                    VisualStyle::filled(theme::c().surface)
                } else {
                    VisualStyle::default()
                };
                let hover_state = state.clone();
                let hover_key = key.clone();
                let st = state.clone();
                let focus = editor_focus.clone();
                let action_path = file_path.clone();
                let row = panel(row_rect, row_style)
                    .event_policy(EventPolicy::INTERACTIVE)
                    .on_pointer_move(move |_cx, _pointer| {
                        let key = hover_key.clone();
                        hover_state.try_update(move |app| {
                            let changed = app.tree_hovered_path.as_deref() != Some(key.as_str());
                            app.tree_hovered_path = Some(key);
                            changed
                        });
                    })
                    .on_pointer_down_with_button(move |cx, pointer, button| {
                        match button {
                            PointerButton::Left => {
                                // Opening waits for the release: a press that moves
                                // becomes a drag into the editor (see `app.rs`).
                                let point = (pointer.point.x, pointer.point.y);
                                let path = action_path.clone();
                                st.update(move |app| {
                                    let clicks =
                                        app.editor.tree_click_count(&path, point.0, point.1);
                                    app.file_drag = Some(FileDragState {
                                        path,
                                        clicks,
                                        origin: point,
                                        point,
                                        active: false,
                                        drop: None,
                                    });
                                });
                            }
                            PointerButton::Middle => {
                                if let Some(id) = open_id {
                                    st.update(move |app| {
                                        app.workspace.set_active(id);
                                        app.workspace.promote_active_preview();
                                    });
                                    focus.focus();
                                } else {
                                    let path = action_path.clone();
                                    match read_text_file(&path) {
                                        Ok(contents) => {
                                            st.update(move |app| {
                                                app.workspace.open_path(path, contents);
                                                app.toast = None;
                                            });
                                            focus.focus();
                                        }
                                        Err(message) => {
                                            st.update(move |app| app.show_error(message));
                                        }
                                    }
                                }
                            }
                            PointerButton::Right => {
                                let path = action_path.clone();
                                st.update(move |app| {
                                    app.tab_context_menu = None;
                                    app.context_menu = Some((pointer.point.x, pointer.point.y));
                                    app.context_menu_target = Some(ExplorerContextTarget {
                                        path,
                                        kind: ExplorerTargetKind::File,
                                    });
                                    app.context_menu_hover = None;
                                });
                            }
                            _ => return,
                        }
                        cx.stop_propagation();
                    })
                    .child(precompiled(
                        UiElement::icon(
                            icon_id,
                            UiRect::new(
                                indent,
                                *y + 2.0,
                                indent + TREE_ICON_SIZE,
                                *y + 2.0 + TREE_ICON_SIZE,
                            ),
                            icon,
                        )
                        .icon_style(IconStyle::new(theme::c().text_muted)),
                    ))
                    .child(text(
                        UiRect::new(
                            indent + TREE_LABEL_OFFSET,
                            *y + 2.0,
                            content_right - 30.0,
                            *y + 18.0,
                        ),
                        name,
                        theme::mono(label_color, theme::UI_SIZE),
                    ))
                    .child(text(
                        UiRect::new(
                            content_right - 25.0,
                            *y + 2.0,
                            content_right - 8.0,
                            *y + 18.0,
                        ),
                        git_indicator,
                        theme::mono(theme::c().accent, theme::SMALL),
                    ));
                els.push(row);
            }
        }

        *y += ROW_H;

        if is_dir
            && s.explorer_create
                .as_ref()
                .is_some_and(|request| request.parent.as_path() == entry.path.as_path())
        {
            els.push(create_row(
                rect,
                *y,
                indent,
                state.clone(),
                create_input_focus.clone(),
                create_input_id.clone(),
            ));
            *y += CREATE_ROW_H;
        }

        if is_dir && s.expanded.contains(&key) {
            // Indent guide under an expanded folder's icon: spans from the
            // folder row bottom to the bottom of its last child.
            let children_top = *y;
            let guide_x = indent + TREE_ICON_SIZE / 2.0;
            els.extend(build_tree(
                &key,
                rect,
                y,
                depth + 1,
                s,
                state,
                path,
                rows,
                content_right,
                git,
                editor_focus,
                create_input_focus,
                create_input_id,
            ));
            if *y > children_top {
                els.push(panel(
                    UiRect::new(guide_x, children_top, guide_x + 1.0, *y),
                    VisualStyle::filled(theme::c().border),
                ));
            }
        }
        if is_dir {
            path.pop();
        }
    }

    els
}

/// Right edge required by the currently visible tree rows. File-tree labels
/// use the editor's monospace UI font, so the shared character advance gives
/// a stable scroll range without measuring text during every frame.
fn tree_content_right(root_key: &str, root_name: &str, s: &AppState, left: f32) -> f32 {
    let mut right = row_content_right(left, 0, root_name);
    if s.expanded.contains(root_key) {
        extend_content_right(root_key, 1, s, left, &mut right);
    }
    right
}

fn extend_content_right(dir_key: &str, depth: usize, s: &AppState, left: f32, right: &mut f32) {
    let Some(entries) = s.dir_entries.get(dir_key) else {
        return;
    };

    for entry in entries.iter() {
        *right = right.max(row_content_right(left, depth, &entry.name));
        if entry.is_dir {
            let key = entry.path.to_string_lossy();
            if s.expanded.contains(key.as_ref()) {
                extend_content_right(key.as_ref(), depth + 1, s, left, right);
            }
        }
    }
}

fn row_content_right(left: f32, depth: usize, name: &str) -> f32 {
    let label_width = name.chars().count() as f32 * theme::CHAR_W;
    left + INDENT + depth as f32 * INDENT + TREE_LABEL_OFFSET + label_width + 12.0
}

/// Resolves the directory ancestry pinned above the scrolling content.
///
/// A row becomes active when it reaches the slot immediately below its
/// ancestors. The first later row outside a directory's subtree then pushes
/// that directory upward until the replacement takes over the same slot.
fn sticky_rows(rows: &[TreeRowMeta], viewport_top: f32, scroll: f32) -> Vec<StickyRow> {
    if scroll <= 0.0 {
        return Vec::new();
    }

    let mut active_path = Vec::new();
    for row in rows {
        let slot_top = viewport_top + row.depth as f32 * ROW_H;
        if row.y - scroll <= slot_top {
            active_path = row.sticky_path.clone();
        }
    }

    active_path
        .into_iter()
        .map(|directory| {
            let slot_top = viewport_top + directory.depth as f32 * ROW_H;
            let boundary_top = rows
                .iter()
                .find(|row| {
                    row.y > directory.source_y
                        && !row
                            .sticky_path
                            .iter()
                            .any(|ancestor| ancestor.key == directory.key)
                })
                .map(|row| row.y - scroll - ROW_H)
                .unwrap_or(slot_top);
            StickyRow {
                directory,
                top: slot_top.min(boundary_top),
            }
        })
        .collect()
}

/// Fixed vertical scrollbar for the file tree. Only the draggable thumb is
/// painted; the track remains invisible like the scrollbars in modern editors.
fn vertical_scrollbar(
    rect: UiRect,
    content_bottom: f32,
    scroll: f32,
    state: State<AppState>,
) -> Element {
    let content_top = rect.top + 8.0;
    let content_h = (content_bottom - content_top).max(1.0);
    let viewport_h = rect.height();
    let max_scroll = (content_bottom - rect.bottom).max(0.0);

    let track = UiRect::new(
        rect.right - scrollbar::SIZE,
        rect.top + 8.0,
        rect.right,
        rect.bottom - 8.0,
    );
    let thumb_h = (track.height() * viewport_h / content_h)
        .max(scrollbar::MIN_THUMB_LENGTH)
        .min(track.height());
    let travel = track.height() - thumb_h;
    let thumb_top = if max_scroll == 0.0 {
        track.top
    } else {
        track.top + travel * (scroll / max_scroll)
    };

    // Draggable thumb: on pointer-down we remember how far above the thumb's
    // top the cursor is, then map the pointer's vertical movement through the
    // track's travel range onto the scroll range. Track geometry and the
    // scroll range are fixed during a drag (content height can't change), so
    // capturing them here is safe.
    let track_top = track.top;
    let st_down = state.clone();
    let st_move = state.clone();
    let st_up = state.clone();
    let thumb = scrollbar::thumb(
        UiRect::new(
            track.left + scrollbar::INSET,
            thumb_top,
            track.right - scrollbar::INSET,
            thumb_top + thumb_h,
        ),
        0xb8,
    )
    .key("file-tree-vertical-scrollbar-thumb")
    .event_policy(EventPolicy::INTERACTIVE)
    .on_pointer_down(move |_cx, p| {
        st_down.update(move |app| {
            app.scrollbar_dragging = true;
            app.scrollbar_drag_offset = p.point.y - thumb_top;
        });
    })
    .on_pointer_drag(move |_cx, p| {
        st_move.update(move |app| {
            if app.scrollbar_dragging && travel > 0.0 {
                let t = (p.point.y - track_top - app.scrollbar_drag_offset) / travel * max_scroll;
                app.tree_scroll = t.clamp(0.0, max_scroll);
            }
        });
    })
    .on_pointer_up(move |_cx, _p| {
        st_up.update(move |app| app.scrollbar_dragging = false);
    });

    thumb
}

/// Fixed horizontal scrollbar for wide or deeply nested tree rows. As with
/// the vertical scrollbar, only the draggable thumb is painted.
fn horizontal_scrollbar(
    rect: UiRect,
    content_right: f32,
    scroll: f32,
    state: State<AppState>,
) -> Element {
    let content_w = (content_right - rect.left).max(1.0);
    let viewport_w = rect.width();
    let max_scroll = (content_right - rect.right).max(0.0);
    let track = UiRect::new(
        rect.left + 8.0,
        rect.bottom - scrollbar::SIZE,
        rect.right - 8.0,
        rect.bottom,
    );
    let thumb_w = (track.width() * viewport_w / content_w)
        .max(scrollbar::MIN_THUMB_LENGTH)
        .min(track.width());
    let travel = track.width() - thumb_w;
    let thumb_left = if max_scroll == 0.0 {
        track.left
    } else {
        track.left + travel * (scroll / max_scroll)
    };

    let track_left = track.left;
    let st_down = state.clone();
    let st_move = state.clone();
    let st_up = state.clone();
    scrollbar::thumb(
        UiRect::new(
            thumb_left,
            track.top + scrollbar::INSET,
            thumb_left + thumb_w,
            track.bottom - scrollbar::INSET,
        ),
        0xb8,
    )
    .key("file-tree-horizontal-scrollbar-thumb")
    .event_policy(EventPolicy::INTERACTIVE)
    .on_pointer_down(move |_cx, p| {
        st_down.update(move |app| {
            app.horizontal_scrollbar_dragging = true;
            app.horizontal_scrollbar_drag_offset = p.point.x - thumb_left;
        });
    })
    .on_pointer_drag(move |_cx, p| {
        st_move.update(move |app| {
            if app.horizontal_scrollbar_dragging && travel > 0.0 {
                let t = (p.point.x - track_left - app.horizontal_scrollbar_drag_offset) / travel
                    * max_scroll;
                app.tree_scroll_x = t.clamp(0.0, max_scroll);
            }
        });
    })
    .on_pointer_up(move |_cx, _p| {
        st_up.update(move |app| app.horizontal_scrollbar_dragging = false);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(key: &str, depth: usize, source_y: f32) -> StickyDirectory {
        StickyDirectory {
            key: key.to_string(),
            name: key.to_string(),
            depth,
            source_y,
            is_expanded: true,
        }
    }

    fn row(y: f32, depth: usize, sticky_path: &[StickyDirectory]) -> TreeRowMeta {
        TreeRowMeta {
            y,
            depth,
            sticky_path: sticky_path.to_vec(),
        }
    }

    #[test]
    fn pins_the_current_directory_and_its_ancestors() {
        let root = directory("root", 0, 8.0);
        let src = directory("src", 1, 28.0);
        let rows = vec![
            row(8.0, 0, std::slice::from_ref(&root)),
            row(28.0, 1, &[root.clone(), src.clone()]),
            row(48.0, 2, &[root.clone(), src.clone()]),
            row(68.0, 2, &[root.clone(), src.clone()]),
        ];

        let sticky = sticky_rows(&rows, 0.0, 48.0);

        assert_eq!(sticky.len(), 2);
        assert_eq!(sticky[0].directory.key, "root");
        assert_eq!(sticky[0].top, 0.0);
        assert_eq!(sticky[1].directory.key, "src");
        assert_eq!(sticky[1].top, ROW_H);
    }

    #[test]
    fn next_sibling_pushes_the_current_directory_out() {
        let root = directory("root", 0, 8.0);
        let src = directory("src", 1, 28.0);
        let tests = directory("tests", 1, 88.0);
        let rows = vec![
            row(8.0, 0, std::slice::from_ref(&root)),
            row(28.0, 1, &[root.clone(), src.clone()]),
            row(48.0, 2, &[root.clone(), src.clone()]),
            row(68.0, 2, &[root.clone(), src.clone()]),
            row(88.0, 1, &[root.clone(), tests.clone()]),
        ];

        let being_pushed = sticky_rows(&rows, 0.0, 55.0);
        assert_eq!(being_pushed[1].directory.key, "src");
        assert_eq!(being_pushed[1].top, 13.0);

        let replaced = sticky_rows(&rows, 0.0, 68.0);
        assert_eq!(replaced[1].directory.key, "tests");
        assert_eq!(replaced[1].top, ROW_H);
    }

    #[test]
    fn waits_until_a_directory_reaches_its_sticky_slot() {
        let root = directory("root", 0, 8.0);
        let rows = vec![row(8.0, 0, std::slice::from_ref(&root))];

        assert!(sticky_rows(&rows, 0.0, 7.0).is_empty());
        assert_eq!(sticky_rows(&rows, 0.0, 8.0).len(), 1);
    }

    #[test]
    fn horizontal_range_uses_only_visible_tree_rows() {
        let root_key = "root";
        let nested_key = "root/src";
        let long_name = "a_very_long_nested_file_name.rs";
        let mut state = AppState::new();
        state.expanded.insert(root_key.to_string());
        state.expanded.insert(nested_key.to_string());
        state.dir_entries.insert(
            root_key.to_string(),
            vec![DirEntry {
                name: "src".to_string(),
                path: nested_key.into(),
                is_dir: true,
            }]
            .into(),
        );
        state.dir_entries.insert(
            nested_key.to_string(),
            vec![DirEntry {
                name: long_name.to_string(),
                path: format!("{nested_key}/{long_name}").into(),
                is_dir: false,
            }]
            .into(),
        );

        assert_eq!(
            tree_content_right(root_key, "root", &state, 100.0),
            row_content_right(100.0, 2, long_name)
        );

        state.expanded.remove(nested_key);
        assert_eq!(
            tree_content_right(root_key, "root", &state, 100.0),
            row_content_right(100.0, 1, "src")
        );
    }

    #[test]
    fn sidebar_resize_tracks_both_drag_directions_and_clamps() {
        assert_eq!(resized_sidebar_width(1_000.0, 750.0), 250.0);
        assert_eq!(resized_sidebar_width(1_000.0, 900.0), theme::SIDEBAR_MIN_W);
        assert_eq!(resized_sidebar_width(1_000.0, 500.0), theme::SIDEBAR_MAX_W);
    }
}
