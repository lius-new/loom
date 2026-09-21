//! File drawer: the workspace file tree (folders + files).

use std::collections::HashSet;

use lgui::core::{ellipse, Color, CursorIcon, EventPolicy, IconStyle, UiElement, UiId, precompiled};
use lgui::prelude::{panel, text, Element, State, UiRect, VisualStyle};
use lgui::text::measure_width;

use crate::model::document::{meta, FileId, Folder, DIR_SRC};
use crate::state::AppState;
use crate::theme;

/// Horizontal indent per tree depth level.
const INDENT: f32 = 16.0;

/// A color-tinted, resolution-independent SVG icon (rasterized at physical pixels).
fn icon(id: &'static str, key: &'static str, rect: UiRect, color: Color) -> Element {
    precompiled(UiElement::icon(UiId::new(id), rect, key).icon_style(IconStyle::new(color)))
}

/// Natural width of `s` at the small (badge) text size, measured with the
/// installed text environment; falls back to a per-char estimate.
fn measure(s: &str) -> f32 {
    let bounds = UiRect::new(0.0, 0.0, 10_000.0, theme::SMALL);
    measure_width(s, bounds, theme::SMALL, 400).unwrap_or_else(|| {
        s.chars().count() as f32 * theme::CHAR_W * (theme::SMALL / theme::CODE_SIZE)
    })
}

/// Render one folder and, when expanded, its subfolders and files. Returns
/// the updated element tree and the next free vertical cursor.
fn tree(
    mut bar: Element,
    folder: &Folder,
    depth: u32,
    mut y: f32,
    rect: UiRect,
    state: State<AppState>,
    active: FileId,
    collapsed: &HashSet<&'static str>,
) -> (Element, f32) {
    let is_open = !collapsed.contains(folder.name);
    let indent = rect.left + 12.0 + depth as f32 * INDENT;

    // Folder row: chevron + name; clicking toggles open/closed.
    let st = state.clone();
    let name = folder.name;
    let row = UiRect::new(rect.left, y, rect.right, y + 20.0);
    let key = if is_open { "folder-open" } else { "folder" };
    let row_el = panel(row, VisualStyle::default())
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move || {
            st.update(move |app| {
                if app.collapsed.contains(name) {
                    app.collapsed.remove(name);
                } else {
                    app.collapsed.insert(name);
                }
            })
        })
        .child(icon(
            name,
            key,
            UiRect::new(indent, y + 4.0, indent + 12.0, y + 16.0),
            theme::ZINC_400,
        ))
        .child(text(
            UiRect::new(indent + 16.0, y + 2.0, rect.right - 12.0, y + 18.0),
            folder.name,
            theme::mono(theme::ZINC_300, theme::UI_SIZE),
        ));
    bar = bar.child(row_el);
    y += 20.0;

    if !is_open {
        return (bar, y);
    }

    // Indent guide: a faint vertical line under the folder's icon spans all
    // of its expanded children so the hierarchy is easy to trace.
    let children_top = y;
    let guide_x = indent + 6.0;

    for sub in folder.subdirs {
        let (b, ny) = tree(bar, sub, depth + 1, y, rect, state.clone(), active, collapsed);
        bar = b;
        y = ny;
    }

    let file_indent = rect.left + 12.0 + (depth + 1) as f32 * INDENT;
    for &id in folder.files {
        let m = meta(id);
        let row = UiRect::new(rect.left, y, rect.right, y + 22.0);
        let bg = if active == id {
            theme::SURFACE
        } else {
            theme::SIDEBAR
        };
        let st = state.clone();
        let badge_w = measure(m.lang.badge());
        let name_x = file_indent + badge_w + 6.0;
        let mut row_el = panel(row, VisualStyle::filled(bg))
            .event_policy(EventPolicy::INTERACTIVE)
            .on_click(move || st.update(move |app| app.workspace.set_active(id)))
            .child(text(
                UiRect::new(file_indent, y + 4.0, name_x, y + 20.0),
                m.lang.badge(),
                theme::mono_bold(m.lang.badge_color(), theme::SMALL),
            ))
            .child(text(
                UiRect::new(name_x, y + 4.0, rect.right - 24.0, y + 20.0),
                m.name,
                theme::mono(theme::ZINC_300, theme::UI_SIZE),
            ));
        if let Some(dot) = m.git_dot {
            row_el = row_el.child(ellipse(
                UiRect::new(rect.right - 22.0, y + 8.0, rect.right - 16.0, y + 14.0),
                VisualStyle::filled(dot.color()),
            ));
        }
        bar = bar.child(row_el);
        y += 22.0;
    }

    if y > children_top {
        bar = bar.child(panel(
            UiRect::new(guide_x, children_top, guide_x + 1.0, y),
            VisualStyle::filled(theme::BORDER),
        ));
    }

    (bar, y)
}

pub fn render(rect: UiRect, state: State<AppState>) -> Element {
    let s = state.get();
    let active = s.workspace.active();
    let collapsed = s.collapsed.clone();

    let bar = panel(rect, VisualStyle::filled(theme::SIDEBAR));

    // Workspace tree
    let (b, _) = tree(
        bar,
        &DIR_SRC,
        0,
        rect.top + 8.0,
        rect,
        state.clone(),
        active,
        &collapsed,
    );
    let mut bar = b;

    // Resize handle on the drawer's left edge. An 8px invisible hit strip
    // (drawn last, above the opaque file rows) carries the drag; the 1px
    // filled panel inside it is the visible divider.
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
