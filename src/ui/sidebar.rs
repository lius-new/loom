//! File drawer: an empty-state prompt when no folder is open, or the folder's
//! file tree when one is. Also owns the right-click capture and resize handle.
//!
//! The tree is lazy: opening a folder reads only its top level, and a
//! sub-directory is read from disk only when first expanded. Renders read from
//! the cached listings, so no filesystem I/O happens per frame. The opened
//! folder itself is shown as a root node (expanded by default) with its
//! contents indented beneath it.

use std::fs;
use std::path::Path;

use lgui::core::{
    CursorIcon, EventPolicy, IconStyle, PointerButton, UiElement, UiEventKind, UiEventPayload,
    UiFocusHandle, UiId, WheelUnit, clip, precompiled,
};
use lgui::prelude::{Element, State, TextAlign, UiRect, VisualStyle, panel, text};

use crate::state::{AppState, DirEntry};
use crate::theme;
use crate::workspace_actions;

const HEADER_H: f32 = theme::TABS_H;
const ROW_H: f32 = 20.0;
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

pub fn render(rect: UiRect, state: State<AppState>, editor_focus: UiFocusHandle) -> Element {
    let s = state.get();
    let header_rect = UiRect::new(rect.left, rect.top, rect.right, rect.top + HEADER_H);
    let content_rect = UiRect::new(rect.left, header_rect.bottom, rect.right, rect.bottom);

    let mut bar = panel(rect, VisualStyle::filled(theme::SIDEBAR))
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
                st_menu.update(move |app| app.context_menu = Some((p.point.x, p.point.y)));
            }
        });
    bar = bar.child(capture);

    // Content: empty-state prompt, or the open folder's tree.
    match &s.open_dir {
        None => {
            for el in empty_state(content_rect, state.clone()) {
                bar = bar.child(el);
            }
        }
        Some(dir) => {
            let key = dir.to_string_lossy().into_owned();
            let root_name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| key.clone());
            let content_right =
                tree_content_right(&key, &root_name, &s, content_rect.left).max(content_rect.right);
            let max_scroll_x = (content_right - content_rect.right).max(0.0);
            let scroll_x = s.tree_scroll_x.clamp(0.0, max_scroll_x);

            let mut y = content_rect.top + 8.0;
            let indent = content_rect.left + INDENT;
            let is_expanded = s.expanded.contains(&key);

            let root = StickyDirectory {
                key: key.clone(),
                name: root_name.clone(),
                depth: 0,
                source_y: y,
                is_expanded,
            };
            let mut path = vec![root.clone()];
            let mut rows = vec![TreeRowMeta {
                y,
                depth: 0,
                sticky_path: path.clone(),
            }];

            // Collect the tree (root node + contents) into a flat element
            // list and retain each row's directory ancestry for sticky rows.
            let mut tree_els: Vec<Element> = Vec::new();
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
                &state,
            ));
            y += ROW_H;

            if is_expanded {
                // Indent guide: a faint 1px line under the root's icon, from
                // the row bottom to the bottom of its last child.
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
                    &editor_focus,
                ));
                if y > children_top {
                    tree_els.push(panel(
                        UiRect::new(guide_x, children_top, guide_x + 1.0, y),
                        VisualStyle::filled(theme::BORDER),
                    ));
                }
            }
            let content_bottom = y;

            // How far the content can be pulled up before its last row reaches
            // the drawer's bottom edge.
            let max_scroll = (content_bottom - content_rect.bottom).max(0.0);
            let scroll = s.tree_scroll.clamp(0.0, max_scroll);

            // Keep wheel scrolling on the tree root so sticky rows, the
            // scrollbar track and ordinary rows all participate equally.
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

            // Clip container: keeps rows inside the drawer while scrolling.
            // The content offset is the negated scroll (scrolling down moves
            // the content up).
            let mut clip_el = clip(content_rect, -scroll_x, -scroll);
            for el in tree_els {
                clip_el = clip_el.child(el);
            }
            bar = bar.child(clip_el);

            // Draw deepest sticky rows first. Parents are added last so a
            // departing child slides underneath its fixed ancestor.
            for sticky in sticky_rows(&rows, content_rect.top, scroll)
                .into_iter()
                .rev()
            {
                let directory = sticky.directory;
                let indent = content_rect.left + INDENT + directory.depth as f32 * INDENT;
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
                    &state,
                ));
            }

            // Scrollbar, drawn outside the clip so it stays fixed.
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
            VisualStyle::filled(theme::BORDER),
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
    let mut header = panel(rect, VisualStyle::filled(theme::SIDEBAR));
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
        theme::ZINC_500,
    ));
    header = header.child(text(
        UiRect::new(
            explorer_icon.right + 7.0,
            rect.top,
            rect.right - 40.0,
            rect.bottom,
        ),
        "EXPLORER",
        theme::mono_bold(theme::ZINC_300, theme::SMALL).tracking(0.8),
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
                "chevron-right",
                collapse_icon,
                theme::ZINC_500,
            )),
    );

    header.child(panel(
        UiRect::new(rect.left, rect.bottom - 1.0, rect.right, rect.bottom),
        VisualStyle::filled(theme::BORDER),
    ))
}

/// Empty state from the Explorer mockup, without its recent-workspace and
/// bottom workspace-switching sections.
fn empty_state(rect: UiRect, state: State<AppState>) -> Vec<Element> {
    let cx = (rect.left + rect.right) / 2.0;
    let icon_rect = UiRect::new(cx - 24.0, rect.top + 28.0, cx + 24.0, rect.top + 76.0);
    let icon =
        theme::bordered(icon_rect, theme::SURFACE, theme::BORDER, 10.0, 1.0).child(drawer_icon(
            "sidebar.empty.folder",
            crate::file_icons::FOLDER_ICON,
            UiRect::new(
                icon_rect.left + 12.0,
                icon_rect.top + 12.0,
                icon_rect.right - 12.0,
                icon_rect.bottom - 12.0,
            ),
            theme::ZINC_500,
        ));

    let mut centered_title = theme::sans_semibold(theme::ZINC_200, theme::UI_SIZE);
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

    let mut centered_desc = theme::sans(theme::ZINC_500, theme::SMALL);
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
    let btn = theme::bordered(btn_rect, theme::SURFACE, theme::BORDER, 4.0, 1.0)
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
            theme::ACCENT,
        ))
        .child(text(
            UiRect::new(
                btn_rect.left + 34.0,
                btn_rect.top,
                btn_rect.right - 12.0,
                btn_rect.bottom,
            ),
            "Open Folder",
            theme::sans_semibold(theme::ZINC_200, theme::UI_SIZE),
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
            theme::ZINC_600,
        ))
        .child(text(
            UiRect::new(
                clone_rect.left + 34.0,
                clone_rect.top,
                clone_rect.right - 92.0,
                clone_rect.bottom,
            ),
            "Clone Repository...",
            theme::sans(theme::ZINC_400, theme::UI_SIZE),
        ))
        .child(text(
            UiRect::new(
                clone_rect.right - 86.0,
                clone_rect.top,
                clone_rect.right - 12.0,
                clone_rect.bottom,
            ),
            "Ctrl+Shift+G",
            theme::mono_right(theme::ZINC_600, 9.0),
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

/// Read and sort one directory's entries (directories first, then name).
fn read_entries(dir: &Path) -> Vec<DirEntry> {
    let mut v: Vec<DirEntry> = match fs::read_dir(dir) {
        Ok(iter) => iter
            .flatten()
            .map(|e| DirEntry {
                is_dir: e.file_type().map(|t| t.is_dir()).unwrap_or(false),
                name: e.file_name().to_string_lossy().into_owned(),
                path: e.path(),
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    v.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    v
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

/// A directory node row: icon (folder/folder-open), name, and a click handler
/// that toggles expansion (reading the directory on first expand).
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
        VisualStyle::filled(theme::SIDEBAR)
    } else if hovered {
        VisualStyle::filled(theme::SURFACE)
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
    row.on_click(move || {
        let key = click_key.clone();
        click_state.update(move |app| {
            if app.expanded.contains(&key) {
                app.expanded.remove(&key);
            } else {
                app.expanded.insert(key.clone());
                if !app.dir_entries.contains_key(&key) {
                    let entries = read_entries(Path::new(&key));
                    app.dir_entries.insert(key.clone(), entries);
                }
            }
        });
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
        .icon_style(IconStyle::new(theme::ZINC_400)),
    ))
    .child(text(
        UiRect::new(indent + TREE_LABEL_OFFSET, y + 2.0, text_right, y + 18.0),
        name.to_string(),
        theme::mono(theme::ZINC_300, theme::UI_SIZE),
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
    editor_focus: &UiFocusHandle,
) -> Vec<Element> {
    let mut els = Vec::new();

    let Some(entries) = s.dir_entries.get(dir_key) else {
        return els;
    };

    for entry in entries {
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
                state,
            ));
        } else {
            rows.push(TreeRowMeta {
                y: *y,
                depth,
                sticky_path: path.clone(),
            });
            let row_rect = UiRect::new(rect.left, *y, content_right, *y + ROW_H);
            let icon_id = UiId::owned(format!("tree-file-icon-{key}"));
            let icon = crate::file_icons::icon_for_file(&name);
            let file_path = entry.path.clone();
            let open_id = s.workspace.file_id_for_path(&file_path);
            let row_style = if s.workspace.active_path() == Some(file_path.as_path()) {
                VisualStyle::filled(theme::ACTIVE_LINE)
            } else if s.tree_hovered_path.as_deref() == Some(key.as_str()) {
                VisualStyle::filled(theme::SURFACE)
            } else {
                VisualStyle::default()
            };
            let hover_state = state.clone();
            let hover_key = key.clone();
            let st = state.clone();
            let focus = editor_focus.clone();
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
                .on_click(move || {
                    if let Some(id) = open_id {
                        st.update(move |app| app.workspace.set_active(id));
                        focus.focus();
                        return;
                    }

                    let path = file_path.clone();
                    match read_text_file(&path) {
                        Ok(contents) => {
                            st.update(move |app| {
                                app.workspace.open_path(path, contents);
                                app.toast = None;
                            });
                            focus.focus();
                        }
                        Err(message) => st.update(move |app| app.show_toast(message)),
                    }
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
                    .icon_style(IconStyle::new(theme::ZINC_400)),
                ))
                .child(text(
                    UiRect::new(
                        indent + TREE_LABEL_OFFSET,
                        *y + 2.0,
                        content_right - 12.0,
                        *y + 18.0,
                    ),
                    name,
                    theme::mono(theme::ZINC_400, theme::UI_SIZE),
                ));
            els.push(row);
        }

        *y += ROW_H;

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
                editor_focus,
            ));
            if *y > children_top {
                els.push(panel(
                    UiRect::new(guide_x, children_top, guide_x + 1.0, *y),
                    VisualStyle::filled(theme::BORDER),
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

    for entry in entries {
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
        rect.right - 7.0,
        rect.top + 8.0,
        rect.right - 3.0,
        rect.bottom - 8.0,
    );
    let thumb_h = (track.height() * viewport_h / content_h)
        .max(24.0)
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
    let thumb = panel(
        UiRect::new(track.left, thumb_top, track.right, thumb_top + thumb_h),
        VisualStyle::filled(theme::ZINC_600).radius(2.0),
    )
    .key("file-tree-vertical-scrollbar-thumb")
    .event_policy(EventPolicy::INTERACTIVE)
    .on_pointer_down(move |_cx, p| {
        st_down.update(move |app| {
            app.scrollbar_dragging = true;
            app.scrollbar_drag_offset = p.point.y - thumb_top;
        });
    })
    .on_pointer_move(move |_cx, p| {
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
        rect.bottom - 7.0,
        rect.right - 8.0,
        rect.bottom - 3.0,
    );
    let thumb_w = (track.width() * viewport_w / content_w)
        .max(24.0)
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
    panel(
        UiRect::new(thumb_left, track.top, thumb_left + thumb_w, track.bottom),
        VisualStyle::filled(theme::ZINC_600).radius(2.0),
    )
    .key("file-tree-horizontal-scrollbar-thumb")
    .event_policy(EventPolicy::INTERACTIVE)
    .on_pointer_down(move |_cx, p| {
        st_down.update(move |app| {
            app.horizontal_scrollbar_dragging = true;
            app.horizontal_scrollbar_drag_offset = p.point.x - thumb_left;
        });
    })
    .on_pointer_move(move |_cx, p| {
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
            }],
        );
        state.dir_entries.insert(
            nested_key.to_string(),
            vec![DirEntry {
                name: long_name.to_string(),
                path: format!("{nested_key}/{long_name}").into(),
                is_dir: false,
            }],
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
