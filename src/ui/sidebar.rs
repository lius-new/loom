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
    clip, CursorIcon, EventPolicy, IconStyle, PointerButton, UiElement, UiEventKind,
    UiEventPayload, UiId, WheelUnit, precompiled,
};
use lgui::dialogs::{system_file_dialogs, FileDialogOptions};
use lgui::prelude::{panel, text, Element, State, TextAlign, UiRect, VisualStyle};

use crate::state::{AppState, DirEntry};
use crate::theme;

const ROW_H: f32 = 20.0;
const INDENT: f32 = 12.0;

pub fn render(rect: UiRect, state: State<AppState>) -> Element {
    let s = state.get();

    let mut bar = panel(rect, VisualStyle::filled(theme::SIDEBAR));

    // Background right-click capture: covers the drawer at the lowest z-order.
    // Rows and the resize handle are drawn later and sit above it, so they win
    // hit-testing and this only receives events on empty space.
    let st_menu = state.clone();
    let capture = panel(rect, VisualStyle::default())
        .event_policy(EventPolicy::INTERACTIVE)
        .on_pointer_down_with_button(move |_cx, p, button| {
            if button == PointerButton::Right {
                st_menu.update(move |app| app.context_menu = Some((p.point.x, p.point.y)));
            }
        });
    bar = bar.child(capture);

    // Content: empty-state prompt, or the open folder's tree.
    match &s.open_dir {
        None => {
            for el in empty_state(rect, state.clone()) {
                bar = bar.child(el);
            }
        }
        Some(dir) => {
            let key = dir.to_string_lossy().into_owned();
            let root_name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| key.clone());

            let mut y = rect.top + 8.0;
            let indent = rect.left + INDENT;
            let is_expanded = s.expanded.contains(&key);

            // Collect the tree (root node + contents) into a flat element
            // list and remember where it ends so we can compute scroll range.
            let mut tree_els: Vec<Element> = Vec::new();
            tree_els.push(dir_row(&key, &root_name, indent, rect, y, is_expanded, &state));
            y += ROW_H;

            if is_expanded {
                // Indent guide: a faint 1px line under the root's icon, from
                // the row bottom to the bottom of its last child.
                let children_top = y;
                let guide_x = indent + 6.0;
                tree_els.extend(build_tree(&key, rect, &mut y, 1, &s, &state));
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
            let max_scroll = (content_bottom - rect.bottom).max(0.0);
            let scroll = s.tree_scroll.clamp(0.0, max_scroll);

            // Clip container: keeps rows inside the drawer while scrolling.
            // The content offset is the negated scroll (scrolling down moves
            // the content up). Wheel events land here because rows carry no
            // wheel handler, so hit-testing walks up to this node.
            let st = state.clone();
            let mut clip_el = clip(rect, 0.0, -scroll)
                .on_event(UiEventKind::Wheel, move |_cx, payload| {
                if let UiEventPayload::Wheel { delta } = payload {
                    let step = match delta.unit {
                        WheelUnit::Lines => delta.y * ROW_H * 3.0,
                        WheelUnit::Pixels => delta.y,
                    };
                    st.update(move |app| {
                        app.tree_scroll = (app.tree_scroll - step).clamp(0.0, max_scroll);
                    });
                }
            });
            for el in tree_els {
                clip_el = clip_el.child(el);
            }
            bar = bar.child(clip_el);

            // Scrollbar, drawn outside the clip so it stays fixed.
            if max_scroll > 0.0 {
                bar = bar.child(scrollbar(rect, content_bottom, scroll));
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
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::ResizeHorizontal)
        .on_pointer_down(move |_cx, _p| {
            st_down.update(move |app| app.resizing_sidebar = true);
        })
        .on_pointer_move(move |_cx, p| {
            st_move.update(move |app| {
                if app.resizing_sidebar {
                    app.sidebar_w = (win_right - p.point.x)
                        .clamp(theme::SIDEBAR_MIN_W, theme::SIDEBAR_MAX_W);
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

/// Empty state: a short prompt plus a compact "Open Folder" button, centered
/// in the drawer.
fn empty_state(rect: UiRect, state: State<AppState>) -> Vec<Element> {
    let cx = (rect.left + rect.right) / 2.0;
    let cy = (rect.top + rect.bottom) / 2.0;

    let title = text(
        UiRect::new(rect.left + 16.0, cy - 42.0, rect.right - 16.0, cy - 22.0),
        "No folder open",
        theme::sans_semibold(theme::ZINC_200, theme::UI_SIZE),
    );

    let desc = text(
        UiRect::new(rect.left + 16.0, cy - 20.0, rect.right - 16.0, cy - 4.0),
        "Open a folder to browse its files",
        theme::sans(theme::ZINC_500, theme::SMALL),
    );

    let btn_w = 110.0;
    let btn_h = 24.0;
    let btn_rect = UiRect::new(
        cx - btn_w / 2.0,
        cy + 8.0,
        cx + btn_w / 2.0,
        cy + 8.0 + btn_h,
    );
    let st = state.clone();
    let mut btn_style = theme::sans_semibold(theme::ZINC_200, theme::UI_SIZE);
    btn_style.align = TextAlign::Center;
    let btn = theme::bordered(btn_rect, theme::SURFACE, theme::BORDER, 4.0, 1.0)
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move || {
            let options = FileDialogOptions::new().title("Open Folder");
            if let Some(path) = system_file_dialogs().pick_folder(&options) {
                st.update(move |app| {
                    let key = path.to_string_lossy().into_owned();
                    let entries = read_entries(&path);
                    app.open_dir = Some(path);
                    app.dir_entries.clear();
                    app.dir_entries.insert(key.clone(), entries);
                    app.expanded.clear();
                    app.expanded.insert(key); // root starts expanded
                    app.tree_scroll = 0.0;
                });
            }
        })
        .child(text(
            UiRect::new(
                btn_rect.left,
                btn_rect.top + 4.0,
                btn_rect.right,
                btn_rect.bottom - 4.0,
            ),
            "Open Folder",
            btn_style,
        ));

    vec![title.into(), desc.into(), btn]
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
    v.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.cmp(&b.name))
    });
    v
}

/// A directory node row: icon (folder/folder-open), name, and a click handler
/// that toggles expansion (reading the directory on first expand).
fn dir_row(
    key: &str,
    name: &str,
    indent: f32,
    rect: UiRect,
    y: f32,
    is_expanded: bool,
    state: &State<AppState>,
) -> Element {
    let row_rect = UiRect::new(rect.left, y, rect.right, y + ROW_H);
    let icon = if is_expanded { "folder-open" } else { "folder" };
    let fid = UiId::owned(format!("tree-{key}"));

    let st = state.clone();
    let k = key.to_string();
    panel(row_rect, VisualStyle::default())
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move || {
            let key = k.clone();
            st.update(move |app| {
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
        .child(precompiled(UiElement::icon(
            fid,
            UiRect::new(indent, y + 4.0, indent + 12.0, y + 16.0),
            icon,
        ).icon_style(IconStyle::new(theme::ZINC_400))))
        .child(text(
            UiRect::new(indent + 16.0, y + 2.0, rect.right - 12.0, y + 18.0),
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
            els.push(dir_row(&key, &name, indent, rect, *y, is_expanded, state));
        } else {
            let row_rect = UiRect::new(rect.left, *y, rect.right, *y + ROW_H);
            let row = panel(row_rect, VisualStyle::default())
                .event_policy(EventPolicy::INTERACTIVE)
                .child(text(
                    UiRect::new(indent + 16.0, *y + 2.0, rect.right - 12.0, *y + 18.0),
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
            let guide_x = indent + 6.0;
            els.extend(build_tree(&key, rect, y, depth + 1, s, state));
            if *y > children_top {
                els.push(panel(
                    UiRect::new(guide_x, children_top, guide_x + 1.0, *y),
                    VisualStyle::filled(theme::BORDER),
                ));
            }
        }
    }

    els
}

/// Fixed vertical scrollbar for the file tree: a thin track plus a thumb whose
/// height and position reflect the visible/content ratio and the scroll offset.
fn scrollbar(rect: UiRect, content_bottom: f32, scroll: f32) -> Element {
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

    panel(track, VisualStyle::filled(theme::ZINC_800).radius(2.0)).child(panel(
        UiRect::new(track.left, thumb_top, track.right, thumb_top + thumb_h),
        VisualStyle::filled(theme::ZINC_600).radius(2.0),
    ))
}
