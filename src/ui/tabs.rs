//! Editor tab strip: buffer tabs with language badges, a close button, and
//! the collapsed drawer control on the right.

use std::cell::RefCell;
use std::collections::HashMap;

use lgui::core::{
    Color, EventPolicy, IconStyle, PointerButton, UiElement, UiFocusHandle, UiId, WheelUnit, clip,
    precompiled,
};
use lgui::prelude::{Element, State, TextStyle, UiRect, VisualStyle, panel, text};
use lgui::text::{self, TextLayoutRequest};

use crate::model::document::{FileId, FileMeta};
use crate::state::{AppState, MainSurface, TabContextMenuState, TabDragState};
use crate::theme;
use crate::ui::tab_layout::{self, TabLayoutInput, TabLayoutResult};

/// Horizontal padding inside each tab pill (px-3 in the mockup).
pub(super) const PAD: f32 = 12.0;
/// Gap between badge, name and close icon (gap-2 in the mockup).
pub(super) const GAP: f32 = 8.0;
/// Gap between adjacent tab pills.
pub(super) const TAB_GAP: f32 = 4.0;
/// Extra right margin inside text rects so the last glyph is not clipped.
pub(super) const TEXT_MARGIN: f32 = 6.0;
/// Left padding of the tab cluster. The strip itself has no padding — each
/// cluster (tabs / controls) applies its own, so parent padding never leaks
/// out as an outer margin.
pub(super) const TABS_PAD: f32 = 8.0;
/// Vertical inset of each tab pill inside the strip.
pub(super) const PILL_INSET: f32 = 4.0;
/// Upper bound for inactive tabs. The active tab uses its full natural width.
pub(super) const MAX_TAB_W: f32 = 220.0;
/// Keeps the badge, an ellipsis and the close button usable when tabs shrink.
pub(super) const MIN_TAB_W: f32 = 96.0;
const EDITOR_MIN_TAB_W: f32 = 120.0;
const CLUSTER_PAD: f32 = 8.0;
const ICON: f32 = 14.0;
const DIRTY_MARK: &str = "●";
const FADE_W: f32 = 16.0;
const DRAG_THRESHOLD: f32 = 5.0;
const DRAG_EDGE_W: f32 = 28.0;
const DRAG_SCROLL_STEP: f32 = 18.0;
const TAB_SCROLLBAR_H: f32 = 4.0;
const TAB_SCROLLBAR_HIT_H: f32 = 8.0;
const TAB_SCROLLBAR_MIN_THUMB_W: f32 = 28.0;
/// Extra distance beyond each viewport edge whose tabs are still built, so a
/// wheel step never reveals an empty gap before the next frame.
const RENDER_OVERSCAN: f32 = 240.0;

/// Upper bound for cached text widths; ellipsis probing adds short-lived
/// candidates, so the cache is reset rather than allowed to grow unbounded.
const MEASURE_CACHE_LIMIT: usize = 8_192;

thread_local! {
    /// Shaped text widths. The tab strip lays out every
    /// open tab on each pointer move, so repeated shaping would dominate with
    /// many tabs. Font families are fixed and widths are in logical pixels, so
    /// the key changes whenever a name, size or weight does.
    static MEASURE_CACHE: RefCell<MeasureCache> = RefCell::new(MeasureCache::default());
}

/// Text widths grouped by `(size bits, weight)` so hits can look up by `&str`
/// without allocating.
#[derive(Default)]
struct MeasureCache {
    widths: HashMap<(u32, i32), HashMap<String, f32>>,
    len: usize,
}

/// Natural text width measured with the renderer's text system, falling back
/// to a per-character estimate when no text system is installed.
pub(super) fn measure(s: &str, size: f32, weight: i32) -> f32 {
    let style = (size.to_bits(), weight);
    let cached = MEASURE_CACHE.with(|cache| {
        cache
            .borrow()
            .widths
            .get(&style)
            .and_then(|widths| widths.get(s))
            .copied()
    });
    if let Some(width) = cached {
        return width;
    }

    let bounds = UiRect::new(0.0, 0.0, 10_000.0, size);
    let mut request = TextLayoutRequest::single_line(s, bounds, size, weight);
    request.font_families = theme::MONO_FAMILIES;
    match text::layout(&request) {
        Some(layout) => {
            // Only shaped widths are cached: the estimate below is a stand-in
            // until a text system is installed and must not outlive it.
            MEASURE_CACHE.with(|cache| {
                let mut cache = cache.borrow_mut();
                if cache.len >= MEASURE_CACHE_LIMIT {
                    *cache = MeasureCache::default();
                }
                cache.len += 1;
                cache
                    .widths
                    .entry(style)
                    .or_default()
                    .insert(s.to_owned(), layout.width);
            });
            layout.width
        }
        None => s.chars().count() as f32 * theme::CHAR_W * (size / theme::CODE_SIZE),
    }
}

pub(super) fn ellipsize(value: &str, max_width: f32, size: f32, weight: i32) -> String {
    if measure(value, size, weight) <= max_width {
        return value.to_owned();
    }

    const ELLIPSIS: &str = "…";
    if max_width <= measure(ELLIPSIS, size, weight) {
        return ELLIPSIS.to_owned();
    }

    let suffix = value
        .rfind('.')
        .filter(|index| *index > 0)
        .map(|index| &value[index..])
        .filter(|suffix| {
            measure(suffix, size, weight) + measure(ELLIPSIS, size, weight) < max_width
        })
        .unwrap_or("");
    let stem = &value[..value.len() - suffix.len()];
    let chars: Vec<char> = stem.chars().collect();
    let mut low = 0;
    let mut high = chars.len();
    while low < high {
        let middle = (low + high + 1) / 2;
        let candidate = chars[..middle]
            .iter()
            .copied()
            .chain(ELLIPSIS.chars())
            .chain(suffix.chars())
            .collect::<String>();
        if measure(&candidate, size, weight) <= max_width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }

    chars[..low]
        .iter()
        .copied()
        .chain(ELLIPSIS.chars())
        .chain(suffix.chars())
        .collect()
}

/// A color-tinted, resolution-independent SVG icon (rasterized at physical pixels).
fn icon(id: &'static str, key: &'static str, rect: UiRect, color: Color) -> Element {
    precompiled(UiElement::icon(UiId::new(id), rect, key).icon_style(IconStyle::new(color)))
}

fn controls_left(rect: UiRect, app: &AppState) -> f32 {
    if app.show_drawer {
        rect.right
    } else {
        rect.right - CLUSTER_PAD - ICON - CLUSTER_PAD - 1.0
    }
}

/// Size of an SVG tab badge, matched to the cap height of the text badges.
const BADGE_ICON: f32 = 12.0;

/// Leading tab marker: a language/diff label, or an SVG icon.
#[derive(Clone, Copy)]
enum TabBadge {
    Text(&'static str, Color),
    Icon(&'static str, Color),
}

impl TabBadge {
    fn width(self) -> f32 {
        match self {
            Self::Text(label, _) => measure(label, theme::SMALL, 700),
            Self::Icon(..) => BADGE_ICON,
        }
    }

    fn render(self, left: f32, pill: UiRect) -> Element {
        match self {
            Self::Text(label, color) => text(
                UiRect::new(
                    left,
                    pill.top,
                    left + self.width() + TEXT_MARGIN,
                    pill.bottom,
                ),
                label,
                theme::mono_bold(color, theme::SMALL),
            )
            .into(),
            Self::Icon(key, color) => {
                let top = pill.top + (pill.height() - BADGE_ICON) / 2.0;
                icon(
                    "tabs.badge",
                    key,
                    UiRect::new(left, top, left + BADGE_ICON, top + BADGE_ICON),
                    color,
                )
            }
        }
    }
}

fn tab_badge(app: &AppState, id: FileId, meta: &FileMeta) -> TabBadge {
    if app.workspace.is_settings(id) {
        TabBadge::Icon("settings", theme::c().text_muted)
    } else if app.workspace.is_diff(id) {
        TabBadge::Text("Δ", theme::c().badge.diff)
    } else {
        TabBadge::Text(meta.lang.badge(), meta.lang.badge_color())
    }
}

fn tab_name_style(active: bool, preview: bool) -> TextStyle {
    let style = if active {
        theme::mono(theme::c().text_bright, theme::UI_SIZE)
    } else {
        theme::mono(theme::c().text_muted, theme::UI_SIZE)
    };
    if preview { style.italic() } else { style }
}

fn natural_tab_widths(app: &AppState) -> Vec<f32> {
    let labels = app.workspace.tab_labels();
    app.workspace
        .open_files()
        .iter()
        .filter_map(|id| {
            let meta = app.workspace.meta(*id)?;
            let badge = tab_badge(app, *id, meta);
            let dirty_slot = if app.workspace.is_dirty(*id) {
                measure(DIRTY_MARK, theme::SMALL, 400) + GAP
            } else {
                0.0
            };
            let fixed_width = PAD
                + badge.width()
                + GAP
                + GAP
                + dirty_slot
                + measure("✕", theme::SMALL, 400)
                + PAD;
            let label = labels.get(id).map(String::as_str).unwrap_or(&meta.name);
            Some(fixed_width + measure(label, theme::UI_SIZE, 400) + TEXT_MARGIN)
        })
        .collect()
}

fn tab_layout(rect: UiRect, app: &AppState, reveal_active: bool) -> TabLayoutResult {
    let widths = natural_tab_widths(app);
    let active_index = app.workspace.active().and_then(|active| {
        app.workspace
            .open_files()
            .iter()
            .position(|id| *id == active)
    });
    let base_viewport_width = (controls_left(rect, app) - rect.left).max(0.0);
    tab_layout::calculate(TabLayoutInput {
        natural_widths: &widths,
        viewport_width: base_viewport_width,
        scroll_x: app.tab_scroll_x,
        active_index,
        reveal_active,
        min_tab_width: EDITOR_MIN_TAB_W,
        max_tab_width: MAX_TAB_W,
        gap: TAB_GAP,
        padding: TABS_PAD,
    })
}

pub fn active_visible_scroll(rect: UiRect, app: &AppState) -> f32 {
    tab_layout(rect, app, true).scroll_x
}

#[derive(Clone, Copy, Debug)]
struct TabScrollbarGeometry {
    track: UiRect,
    thumb_left: f32,
    thumb_width: f32,
    travel: f32,
    max_scroll_x: f32,
}

fn scrollbar_geometry(rect: UiRect, layout: &TabLayoutResult) -> Option<TabScrollbarGeometry> {
    if layout.max_scroll_x <= 0.0 || layout.content_width <= 0.0 {
        return None;
    }
    let track = UiRect::new(
        rect.left + TABS_PAD,
        rect.bottom - TAB_SCROLLBAR_HIT_H,
        rect.left + layout.viewport_width - TABS_PAD,
        rect.bottom,
    );
    if track.width() <= 0.0 {
        return None;
    }
    let thumb_width = (track.width() * layout.viewport_width / layout.content_width)
        .max(TAB_SCROLLBAR_MIN_THUMB_W)
        .min(track.width());
    let travel = (track.width() - thumb_width).max(0.0);
    let thumb_left = if layout.max_scroll_x <= 0.0 {
        track.left
    } else {
        track.left + travel * (layout.scroll_x / layout.max_scroll_x)
    };
    Some(TabScrollbarGeometry {
        track,
        thumb_left,
        thumb_width,
        travel,
        max_scroll_x: layout.max_scroll_x,
    })
}

fn scroll_from_pointer(pointer_x: f32, offset: f32, geometry: TabScrollbarGeometry) -> f32 {
    if geometry.travel <= 0.0 {
        return 0.0;
    }
    let thumb_left = (pointer_x - geometry.track.left - offset).clamp(0.0, geometry.travel);
    thumb_left / geometry.travel * geometry.max_scroll_x
}

fn point_inside(rect: UiRect, x: f32, y: f32) -> bool {
    x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
}

/// Track tab hover and, only for an actual `PointerDrag`, advance scrollbar or
/// tab reorder gestures. Ordinary hover movement must never promote a pending
/// click into a drag.
pub fn update_pointer(
    app: &mut AppState,
    rect: UiRect,
    pointer_x: f32,
    pointer_y: f32,
    is_pointer_drag: bool,
) -> bool {
    let inside = point_inside(rect, pointer_x, pointer_y);
    let mut changed = app.tab_strip_hovered != inside;
    app.tab_strip_hovered = inside;

    let mut layout = tab_layout(rect, app, false);
    if is_pointer_drag {
        if app.tab_scrollbar_dragging
            && let Some(geometry) = scrollbar_geometry(rect, &layout)
        {
            let next = scroll_from_pointer(pointer_x, app.tab_scrollbar_drag_offset, geometry);
            if (app.tab_scroll_x - next).abs() > f32::EPSILON {
                app.tab_scroll_x = next;
                changed = true;
                layout = tab_layout(rect, app, false);
            }
        }

        if let Some(mut drag) = app.tab_drag.take() {
            if !drag.active && (pointer_x - drag.pointer_origin_x).abs() >= DRAG_THRESHOLD {
                drag.active = true;
                changed = true;
            }
            if drag.active {
                let viewport_left = rect.left;
                let viewport_right = rect.left + layout.viewport_width;
                let previous_scroll = app.tab_scroll_x;
                if pointer_x <= viewport_left + DRAG_EDGE_W {
                    app.tab_scroll_x = (app.tab_scroll_x - DRAG_SCROLL_STEP).max(0.0);
                } else if pointer_x >= viewport_right - DRAG_EDGE_W {
                    app.tab_scroll_x =
                        (app.tab_scroll_x + DRAG_SCROLL_STEP).min(layout.max_scroll_x);
                }
                if (app.tab_scroll_x - previous_scroll).abs() > f32::EPSILON {
                    changed = true;
                    layout = tab_layout(rect, app, false);
                }

                let content_x = pointer_x - rect.left + layout.scroll_x;
                if let Some(target_index) = tab_layout::reorder_target(&layout.items, content_x)
                    && target_index != drag.target_index
                {
                    drag.target_index = target_index;
                    changed = true;
                }
            }
            app.tab_drag = Some(drag);
        }
    }

    let viewport_right = rect.left + layout.viewport_width;
    let hovered = if inside && pointer_x <= viewport_right {
        let content_x = pointer_x - rect.left + layout.scroll_x;
        layout
            .items
            .iter()
            .find(|item| content_x >= item.left && content_x <= item.right)
            .and_then(|item| app.workspace.open_files().get(item.index))
            .copied()
    } else {
        None
    };
    if app.tab_hovered != hovered {
        app.tab_hovered = hovered;
        changed = true;
    }

    changed
}

/// Commit a completed reorder and release all tab pointer state.
pub fn finish_pointer_interaction(app: &mut AppState) -> bool {
    let mut changed = app.tab_scrollbar_dragging;
    app.tab_scrollbar_dragging = false;
    if let Some(drag) = app.tab_drag.take() {
        changed = true;
        if drag.active {
            changed |= app.workspace.move_tab(drag.source, drag.target_index);
        }
    }
    changed
}

/// Cancel transient pointer state without applying a pending reorder.
pub fn cancel_pointer_interaction(app: &mut AppState) -> bool {
    let changed = app.tab_scrollbar_dragging
        || app.tab_drag.is_some()
        || app.tab_strip_hovered
        || app.tab_hovered.is_some();
    app.tab_scrollbar_dragging = false;
    app.tab_drag = None;
    app.tab_strip_hovered = false;
    app.tab_hovered = None;
    changed
}

pub fn render_tooltip(viewport: UiRect, rect: UiRect, app: &AppState) -> Option<Element> {
    let hovered = app.tab_hovered?;
    let path = app.workspace.meta(hovered)?.path.clone();
    let path = super::display_path(&path);
    let index = app
        .workspace
        .open_files()
        .iter()
        .position(|id| *id == hovered)?;
    let layout = tab_layout(rect, app, false);
    let item = layout.items.get(index)?;
    let natural_width = measure(&path, theme::SMALL, 400) + 20.0;
    let width = natural_width.min((viewport.width() - 16.0).max(80.0));
    let item_left = rect.left + item.left - layout.scroll_x;
    let left = item_left.clamp(viewport.left + 8.0, viewport.right - width - 8.0);
    let tooltip_rect = UiRect::new(left, rect.bottom + 4.0, left + width, rect.bottom + 28.0);
    Some(
        theme::bordered(
            tooltip_rect,
            theme::c().surface,
            theme::c().border,
            4.0,
            1.0,
        )
        .child(text(
            UiRect::new(
                tooltip_rect.left + 9.0,
                tooltip_rect.top + 4.0,
                tooltip_rect.right - 9.0,
                tooltip_rect.bottom - 3.0,
            ),
            path,
            theme::mono(theme::c().text_soft, theme::SMALL),
        )),
    )
}

pub fn render(rect: UiRect, state: State<AppState>, editor_focus: UiFocusHandle) -> Element {
    let s = state.get();
    let active = s.workspace.active();
    let labels = s.workspace.tab_labels();

    let mut bar = panel(rect, VisualStyle::filled(theme::c().sidebar));
    let controls_left = controls_left(rect, &s);
    let layout = tab_layout(rect, &s, false);
    let tab_viewport = UiRect::new(
        rect.left,
        rect.top,
        rect.left + layout.viewport_width,
        rect.bottom,
    );
    let mut tab_layer = clip(tab_viewport, -layout.scroll_x, 0.0);

    if layout.max_scroll_x > 0.0 {
        let wheel_state = state.clone();
        let max_scroll_x = layout.max_scroll_x;
        bar = bar.on_wheel(move |cx, delta| {
            let primary_delta = if delta.x.abs() > delta.y.abs() {
                delta.x
            } else {
                delta.y
            };
            let step = match delta.unit {
                WheelUnit::Lines => primary_delta * 48.0,
                WheelUnit::Pixels => primary_delta,
            };
            wheel_state.update(move |app| {
                app.tab_scroll_x = (app.tab_scroll_x - step).clamp(0.0, max_scroll_x);
            });
            cx.stop_propagation();
        });
    }

    // Tabs are content-sized rounded pills, inset PILL_INSET vertically,
    // separated by TAB_GAP. The strip has no padding; this cluster's left
    // padding is TABS_PAD.
    if s.main_surface() == MainSurface::Welcome {
        let label = "welcome";
        let label_w = measure(label, theme::UI_SIZE, 400);
        let tab_w = (PAD * 2.0 + label_w + TEXT_MARGIN).clamp(MIN_TAB_W, MAX_TAB_W);
        let pill = UiRect::new(
            rect.left + TABS_PAD,
            rect.top + PILL_INSET,
            rect.left + TABS_PAD + tab_w,
            rect.bottom - PILL_INSET,
        );
        tab_layer = tab_layer.child(
            theme::bordered(pill, theme::c().bg, theme::c().border, 2.0, 1.0).child(text(
                UiRect::new(pill.left + PAD, pill.top, pill.right - PAD, pill.bottom),
                label,
                theme::mono(theme::c().text_bright, theme::UI_SIZE),
            )),
        );
    }
    // Only tabs near the viewport become elements. The drag source is kept
    // even when edge auto-scroll moves it away so its keyed element survives
    // until the gesture ends.
    let open_files = s.workspace.open_files();
    let visible = tab_layout::visible_range(
        &layout.items,
        layout.scroll_x,
        layout.viewport_width,
        RENDER_OVERSCAN,
    );
    let drag_source_index = s
        .tab_drag
        .as_ref()
        .and_then(|drag| open_files.iter().position(|id| *id == drag.source))
        .filter(|index| !visible.contains(index));
    for index in visible.chain(drag_source_index) {
        let Some(&id) = open_files.get(index) else {
            continue;
        };
        let Some(m) = s.workspace.meta(id) else {
            continue;
        };
        let Some(layout_item) = layout.items.get(index) else {
            continue;
        };
        let badge = tab_badge(&s, id, m);
        let badge_w = badge.width();
        let dirty = s.workspace.is_dirty(id);
        let dirty_w = if dirty {
            measure(DIRTY_MARK, theme::SMALL, 400)
        } else {
            0.0
        };
        let close_w = measure("✕", theme::SMALL, 400);
        let tab_w = layout_item.width;
        let x = rect.left + layout_item.left;

        let pill = UiRect::new(
            x,
            rect.top + PILL_INSET,
            x + tab_w,
            rect.bottom - PILL_INSET,
        );
        let name_style = tab_name_style(Some(id) == active, s.workspace.is_preview(id));

        let badge_left = pill.left + PAD;
        let name_left = badge_left + badge_w + GAP;
        let close_left = pill.right - PAD - close_w;
        let dirty_left = close_left - GAP - dirty_w;
        let name_right = if dirty {
            dirty_left - GAP
        } else {
            close_left - GAP
        };
        let name_slot_w = (name_right - name_left).max(0.0);
        let label = labels.get(&id).cloned().unwrap_or_else(|| m.name.clone());
        let display_name = if Some(id) == active {
            label
        } else {
            ellipsize(
                &label,
                (name_slot_w - TEXT_MARGIN).max(0.0),
                theme::UI_SIZE,
                400,
            )
        };

        let st = state.clone();
        let focus = editor_focus.clone();
        // Active tab gets a crisp 1px border; see theme::bordered for why a
        // filled ring is used instead of a stroked outline.
        let mut tab_el = if Some(id) == active {
            theme::bordered(pill, theme::c().bg, theme::c().border, 2.0, 1.0)
        } else {
            panel(pill, VisualStyle::default().radius(2.0))
        }
        .key(format!("editor-tab-{id:?}"))
        .event_policy(EventPolicy::INTERACTIVE)
        .on_pointer_down_with_button(move |cx, pointer, button| {
            match button {
                PointerButton::Left => {
                    let pointer_x = pointer.point.x;
                    let pointer_y = pointer.point.y;
                    st.update(move |app| {
                        let clicks = app.editor.tab_click_count(id, pointer_x, pointer_y);
                        app.workspace.set_active(id);
                        app.tab_context_menu = None;
                        if clicks > 1 {
                            app.workspace.promote_preview(id);
                            app.tab_drag = None;
                        } else {
                            app.tab_drag = Some(TabDragState {
                                source: id,
                                pointer_origin_x: pointer_x,
                                target_index: index,
                                active: false,
                            });
                        }
                    });
                    focus.focus();
                }
                PointerButton::Middle => {
                    crate::workspace_actions::request_close_tab(&st, id);
                    focus.focus();
                }
                PointerButton::Right => {
                    let position = (pointer.point.x, pointer.point.y);
                    st.update(move |app| {
                        app.tab_drag = None;
                        app.editor.menu = None;
                        app.context_menu = None;
                        app.context_menu_target = None;
                        app.context_menu_hover = None;
                        app.tab_context_menu = Some(TabContextMenuState {
                            position,
                            target: id,
                            hovered: None,
                        });
                    });
                }
                _ => return,
            }
            cx.stop_propagation();
        });

        tab_el = tab_el.child(badge.render(badge_left, pill));
        tab_el = tab_el.child(text(
            UiRect::new(name_left, pill.top, name_left + name_slot_w, pill.bottom),
            display_name,
            name_style,
        ));
        if dirty {
            tab_el = tab_el.child(text(
                UiRect::new(
                    dirty_left,
                    pill.top,
                    dirty_left + dirty_w + TEXT_MARGIN,
                    pill.bottom,
                ),
                DIRTY_MARK,
                theme::mono(theme::c().accent, theme::SMALL),
            ));
        }

        // The close slot is always reserved by the layout, but the control is
        // only painted and hit-testable for the active or hovered tab.
        if Some(id) == active || s.tab_hovered == Some(id) {
            let st = state.clone();
            let focus = editor_focus.clone();
            tab_el = tab_el.child(
                panel(
                    UiRect::new(close_left, pill.top, pill.right - PAD, pill.bottom),
                    VisualStyle::default(),
                )
                .key(format!("editor-tab-close-{id:?}"))
                .event_policy(EventPolicy::INTERACTIVE)
                .on_pointer_down_with_button(move |cx, _pointer, button| {
                    if button == PointerButton::Left {
                        cx.stop_propagation();
                    }
                })
                .on_click(move || {
                    crate::workspace_actions::request_close_tab(&st, id);
                    focus.focus();
                })
                .child(text(
                    UiRect::new(
                        close_left,
                        pill.top,
                        close_left + close_w + TEXT_MARGIN,
                        pill.bottom,
                    ),
                    "✕",
                    theme::mono(theme::c().text_dim, theme::SMALL),
                )),
            );
        }

        tab_layer = tab_layer.child(tab_el);
    }

    bar = bar.child(tab_layer);

    if let Some(drag) = s.tab_drag.as_ref().filter(|drag| drag.active)
        && let Some(source_index) = s
            .workspace
            .open_files()
            .iter()
            .position(|id| *id == drag.source)
        && drag.target_index != source_index
        && let Some(target) = layout.items.get(drag.target_index)
    {
        let content_x = if drag.target_index < source_index {
            target.left
        } else {
            target.right
        };
        let screen_x = rect.left + content_x - layout.scroll_x;
        if screen_x >= tab_viewport.left && screen_x <= tab_viewport.right {
            bar = bar.child(panel(
                UiRect::new(
                    screen_x - 1.0,
                    rect.top + PILL_INSET,
                    screen_x + 1.0,
                    rect.bottom - PILL_INSET,
                ),
                VisualStyle::filled(theme::c().accent).radius(1.0),
            ));
        }
    }

    const FADE_ALPHA: [u8; 4] = [210, 150, 90, 35];
    if layout.scroll_x > 0.0 {
        let strip_w = FADE_W / FADE_ALPHA.len() as f32;
        for (index, alpha) in FADE_ALPHA.into_iter().enumerate() {
            let left = tab_viewport.left + index as f32 * strip_w;
            bar = bar.child(panel(
                UiRect::new(left, rect.top, left + strip_w, rect.bottom),
                VisualStyle::filled(theme::c().sidebar).alpha(alpha),
            ));
        }
    }
    if layout.scroll_x < layout.max_scroll_x {
        let strip_w = FADE_W / FADE_ALPHA.len() as f32;
        for (index, alpha) in FADE_ALPHA.into_iter().enumerate() {
            let right = tab_viewport.right - index as f32 * strip_w;
            bar = bar.child(panel(
                UiRect::new(right - strip_w, rect.top, right, rect.bottom),
                VisualStyle::filled(theme::c().sidebar).alpha(alpha),
            ));
        }
    }

    if (s.tab_strip_hovered || s.tab_scrollbar_dragging)
        && let Some(geometry) = scrollbar_geometry(rect, &layout)
    {
        let visual_track = UiRect::new(
            geometry.track.left,
            geometry.track.bottom - TAB_SCROLLBAR_H - 1.0,
            geometry.track.right,
            geometry.track.bottom - 1.0,
        );
        let thumb_rect = UiRect::new(
            geometry.thumb_left,
            visual_track.top,
            geometry.thumb_left + geometry.thumb_width,
            visual_track.bottom,
        );
        let track_state = state.clone();
        let thumb_state = state.clone();
        let scrollbar = panel(geometry.track, VisualStyle::default())
            .key("tabs-horizontal-scrollbar")
            .event_policy(EventPolicy::INTERACTIVE)
            .on_pointer_down_with_button(move |cx, pointer, button| {
                if button != PointerButton::Left {
                    return;
                }
                let offset = geometry.thumb_width / 2.0;
                let next = scroll_from_pointer(pointer.point.x, offset, geometry);
                track_state.update(move |app| {
                    app.tab_scroll_x = next;
                    app.tab_scrollbar_dragging = true;
                    app.tab_scrollbar_drag_offset = offset;
                    app.tab_drag = None;
                });
                cx.stop_propagation();
            })
            .child(panel(
                visual_track,
                VisualStyle::filled(theme::c().scrollbar).radius(TAB_SCROLLBAR_H / 2.0),
            ))
            .child(
                panel(
                    thumb_rect,
                    VisualStyle::filled(theme::c().text_dim).radius(TAB_SCROLLBAR_H / 2.0),
                )
                .key("tabs-horizontal-scrollbar-thumb")
                .event_policy(EventPolicy::INTERACTIVE)
                .on_pointer_down_with_button(move |cx, pointer, button| {
                    if button != PointerButton::Left {
                        return;
                    }
                    let offset = pointer.point.x - geometry.thumb_left;
                    thumb_state.update(move |app| {
                        app.tab_scrollbar_dragging = true;
                        app.tab_scrollbar_drag_offset = offset;
                        app.tab_drag = None;
                    });
                    cx.stop_propagation();
                }),
            );
        bar = bar.child(scrollbar);
    }

    // Show the drawer toggle here only when the drawer is collapsed.
    if !s.show_drawer {
        let top = rect.top + (rect.height() - ICON) / 2.0;
        let icon_r = UiRect::new(
            rect.right - CLUSTER_PAD - ICON,
            top,
            rect.right - CLUSTER_PAD,
            top + ICON,
        );
        bar = bar.child(panel(
            UiRect::new(controls_left, rect.top, controls_left + 1.0, rect.bottom),
            VisualStyle::filled(theme::c().border),
        ));
        bar = bar.child(
            panel(
                UiRect::new(controls_left + 1.0, rect.top, rect.right, rect.bottom),
                VisualStyle::filled(theme::c().sidebar),
            )
            .event_policy(EventPolicy::INTERACTIVE)
            .on_click(move || state.update(|app| app.show_drawer = true))
            .child(icon(
                "tabs.drawer",
                "panel-left",
                icon_r,
                theme::c().text_muted,
            )),
        );
    }

    // Hairline bottom border (matches the title bar).
    bar = bar.child(panel(
        UiRect::new(rect.left, rect.bottom - 1.0, rect.right, rect.bottom),
        VisualStyle::filled(theme::c().border),
    ));

    bar
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgui::text::TextFontSlant;
    use std::path::PathBuf;

    #[test]
    fn preview_tab_names_are_italic() {
        assert_eq!(tab_name_style(true, true).font_slant, TextFontSlant::Italic);
        assert_eq!(
            tab_name_style(true, false).font_slant,
            TextFontSlant::Upright
        );
    }

    #[test]
    fn long_file_names_preserve_the_extension_and_fit_the_budget() {
        let budget = measure("very-long….rs", theme::UI_SIZE, 400);
        let result = ellipsize(
            "very-long-file-name-that-keeps-going.rs",
            budget,
            theme::UI_SIZE,
            400,
        );

        assert!(result.contains('…'));
        assert!(result.ends_with(".rs"));
        assert!(measure(&result, theme::UI_SIZE, 400) <= budget);
    }

    #[test]
    fn short_file_names_remain_unchanged() {
        assert_eq!(ellipsize("main.rs", 200.0, theme::UI_SIZE, 400), "main.rs");
    }

    #[test]
    fn scrollbar_pointer_mapping_reaches_both_ends() {
        let geometry = TabScrollbarGeometry {
            track: UiRect::new(10.0, 0.0, 210.0, 8.0),
            thumb_left: 10.0,
            thumb_width: 40.0,
            travel: 160.0,
            max_scroll_x: 800.0,
        };

        assert_eq!(scroll_from_pointer(10.0, 0.0, geometry), 0.0);
        assert_eq!(scroll_from_pointer(170.0, 0.0, geometry), 800.0);
        assert_eq!(scroll_from_pointer(90.0, 0.0, geometry), 400.0);
    }

    #[test]
    fn tab_reorder_waits_for_the_drag_threshold_and_commits_on_release() {
        let mut app = AppState::new();
        let first = app
            .workspace
            .open_path(PathBuf::from("first.rs"), String::new());
        let second = app
            .workspace
            .open_path(PathBuf::from("second.rs"), String::new());
        let third = app
            .workspace
            .open_path(PathBuf::from("third.rs"), String::new());
        app.workspace.set_active(first);
        app.tab_drag = Some(TabDragState {
            source: first,
            pointer_origin_x: 20.0,
            target_index: 0,
            active: false,
        });
        let rect = UiRect::new(0.0, 0.0, 700.0, theme::TABS_H);

        assert!(update_pointer(&mut app, rect, 22.0, 10.0, true));
        assert!(!app.tab_drag.as_ref().unwrap().active);
        assert!(finish_pointer_interaction(&mut app));
        assert_eq!(app.workspace.open_files(), &[first, second, third]);

        app.tab_drag = Some(TabDragState {
            source: first,
            pointer_origin_x: 20.0,
            target_index: 0,
            active: false,
        });
        assert!(update_pointer(&mut app, rect, 650.0, 10.0, true));
        assert!(app.tab_drag.as_ref().unwrap().active);
        assert!(finish_pointer_interaction(&mut app));
        assert_eq!(app.workspace.open_files(), &[second, third, first]);
    }

    /// Manual probe for Phase 6: `cargo test --release -- --ignored --nocapture
    /// pointer_updates_with_many_tabs`.
    #[test]
    #[ignore]
    fn pointer_updates_with_many_tabs() {
        for count in [100, 500] {
            let mut app = AppState::new();
            for index in 0..count {
                app.workspace.open_path(
                    PathBuf::from(format!("dir{}/file-{index}.rs", index % 7)),
                    String::new(),
                );
            }
            let rect = UiRect::new(0.0, 0.0, 1200.0, theme::TABS_H);
            let started = std::time::Instant::now();
            for step in 0..200 {
                update_pointer(&mut app, rect, (step * 5) as f32, 10.0, false);
            }
            println!(
                "{count} tabs: {:?} per pointer update",
                started.elapsed() / 200
            );
        }
    }

    #[test]
    fn ordinary_hover_never_promotes_a_pending_click_to_tab_drag() {
        let mut app = AppState::new();
        let first = app
            .workspace
            .open_path(PathBuf::from("first.rs"), String::new());
        let second = app
            .workspace
            .open_path(PathBuf::from("second.rs"), String::new());
        app.workspace.set_active(first);
        app.tab_drag = Some(TabDragState {
            source: first,
            pointer_origin_x: 20.0,
            target_index: 0,
            active: false,
        });
        let rect = UiRect::new(0.0, 0.0, 700.0, theme::TABS_H);

        assert!(update_pointer(&mut app, rect, 500.0, 10.0, false));
        assert!(!app.tab_drag.as_ref().unwrap().active);
        assert!(finish_pointer_interaction(&mut app));
        assert_eq!(app.workspace.open_files(), &[first, second]);
    }
}
