//! The miniature itself is the drop surface. Hit tests and fills share rectangles.
use crate::model::application_layout::{
    ApplicationLayout, Content, DropPlan, Placement, RegionId, Visibility,
};
use crate::model::pane_layout::{Axis, Direction};
use crate::state::AppState;
use crate::terminal_session::TerminalTabs;
use crate::theme;
use lgui::core::{
    CursorIcon, LogicalKey, NamedKey, Observable, ObservableListener, Point, UiEventKind,
    UiEventPayload,
};
use lgui::prelude::{Element, State, Stroke, UiRect, VisualStyle, component, panel};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Share of the content area the miniature may occupy on each axis.
const MAP_FRACTION: f32 = 0.56;
/// Window-edge drop bands sit just outside the miniature's silhouette.
const OUTER_BAND: f32 = 18.0;
/// Half-width of the gutter reserved for each internal divider.
const DIVIDER_HALF: f32 = 5.0;
/// Depth of a cell's side drop bands before its centre takes over.
const EDGE_BAND: f32 = 22.0;
/// Cell corner radius; also used by the centre-drop highlight.
const CELL_RADIUS: f32 = 6.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub rect: UiRect,
    pub placement: Placement,
    /// A cell's body, as opposed to its edge, divider or window bands.
    pub center: bool,
    pub plan: Option<DropPlan>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct MapScene {
    pub version: u64,
    pub viewport: UiRect,
    pub visibility: Visibility,
    pub sessions: Vec<u64>,
    pub cells: Vec<(RegionId, Content, UiRect)>,
    pub zones: Vec<Zone>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalDrag {
    pub session: u64,
    pub origin: Point,
    pub point: Point,
    pub active: bool,
    pub scene: Option<MapScene>,
    pub hover: Option<usize>,
    /// Validate the framework's hover state on the release frame. Its cancel
    /// event is also delivered as PointerUp, but clears all hovered nodes.
    pub release: Option<Point>,
}
/// The miniature's drag state, kept outside `AppState`. Changing it re-renders
/// only its subscribers (the overlay and the tab cursors), never the app.
#[derive(Clone, Default)]
pub struct MapStore(Arc<MapInner>);
#[derive(Default)]
struct MapInner {
    drag: Mutex<Option<TerminalDrag>>,
    revision: AtomicU64,
    listeners: Mutex<Vec<(u64, ObservableListener)>>,
    next_listener: AtomicU64,
}
/// What subscribers observe: a change counter and the cursor it implies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapSignal {
    pub revision: u64,
    pub cursor: CursorIcon,
}
impl MapStore {
    pub fn with<R>(&self, f: impl FnOnce(Option<&TerminalDrag>) -> R) -> R {
        f(self.0.drag.lock().expect("map store poisoned").as_ref())
    }
    /// Mutate the drag; subscribers are notified only when `f` reports a
    /// visible change. Plain pointer bookkeeping returns false.
    pub fn update(&self, f: impl FnOnce(&mut Option<TerminalDrag>) -> bool) -> bool {
        let changed = f(&mut self.0.drag.lock().expect("map store poisoned"));
        if changed {
            self.0.revision.fetch_add(1, Ordering::AcqRel);
            let listeners = self
                .0
                .listeners
                .lock()
                .expect("map listeners poisoned")
                .iter()
                .map(|(_, l)| l.clone())
                .collect::<Vec<_>>();
            for listener in listeners {
                listener();
            }
        }
        changed
    }
    pub fn snapshot(&self) -> Option<TerminalDrag> {
        self.with(|d| d.cloned())
    }
    pub fn is_dragging(&self) -> bool {
        self.with(|d| d.is_some())
    }
    pub fn is_active(&self) -> bool {
        self.with(|d| d.is_some_and(|d| d.active))
    }
    pub fn cancel(&self) -> bool {
        self.update(|d| d.take().is_some())
    }
    pub fn observable(&self) -> Observable<MapSignal> {
        let read = self.clone();
        let subscribe = self.clone();
        Observable::new(
            Arc::as_ptr(&self.0) as u64,
            move || MapSignal {
                revision: read.0.revision.load(Ordering::Acquire),
                cursor: drag_cursor(&read),
            },
            move |listener| {
                let inner = subscribe.0.clone();
                let id = inner.next_listener.fetch_add(1, Ordering::Relaxed);
                inner
                    .listeners
                    .lock()
                    .expect("map listeners poisoned")
                    .push((id, listener));
                Box::new(move || {
                    inner
                        .listeners
                        .lock()
                        .expect("map listeners poisoned")
                        .retain(|(i, _)| *i != id);
                })
            },
        )
    }
}
/// Borrow the app state without invalidating it.
fn with_app<R>(state: &State<AppState>, f: impl FnOnce(&mut AppState) -> R) -> R {
    let mut out = None;
    state.try_update(|app| {
        out = Some(f(app));
        false
    });
    out.expect("state update ran")
}
pub fn visibility(app: &AppState) -> Visibility {
    Visibility {
        git: app.show_source_control,
        files: app.show_drawer,
        terminal: app.show_terminal,
    }
}
pub fn begin(app: &mut AppState, map: &MapStore, session: u64, point: Point) {
    map.update(|d| {
        *d = Some(TerminalDrag {
            session,
            origin: point,
            point,
            active: false,
            scene: None,
            hover: None,
            release: None,
        });
        true
    });
    app.terminal_tab_context_menu = None;
    app.terminal_shell_menu = false;
}
fn contains(rect: UiRect, point: Point) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}
impl MapScene {
    pub fn hit(&self, point: Point) -> Option<usize> {
        self.zones.iter().position(|z| contains(z.rect, point))
    }
    fn build(
        layout: &ApplicationLayout,
        session: u64,
        viewport: UiRect,
        visible: Visibility,
        sessions: Vec<u64>,
    ) -> Self {
        let snapshot = layout.snapshot(viewport, visible);
        let width = viewport.width() * MAP_FRACTION;
        let height = viewport.height() * MAP_FRACTION;
        let scale = ((width - 2.0 * OUTER_BAND).max(1.0) / viewport.width().max(1.0))
            .min((height - 2.0 * OUTER_BAND).max(1.0) / viewport.height().max(1.0));
        let left = (viewport.left + viewport.right - viewport.width() * scale) / 2.0;
        let top = (viewport.top + viewport.bottom - viewport.height() * scale) / 2.0;
        let project = |r: UiRect| {
            UiRect::new(
                left + (r.left - viewport.left) * scale,
                top + (r.top - viewport.top) * scale,
                left + (r.right - viewport.left) * scale,
                top + (r.bottom - viewport.top) * scale,
            )
        };
        let silhouette = project(viewport);
        let mut zones = vec![];
        // Internal boundaries reserve a gutter, ordered by smaller scope.
        let mut dividers = snapshot.dividers.clone();
        dividers.sort_by(|a, b| {
            (a.parent.width() * a.parent.height())
                .total_cmp(&(b.parent.width() * b.parent.height()))
        });
        let mut divider_rects = vec![];
        for divider in &dividers {
            let scope = project(divider.parent);
            let r = match divider.axis {
                Axis::Horizontal => {
                    let x = left + (divider.position - viewport.left) * scale;
                    UiRect::new(x - DIVIDER_HALF, scope.top, x + DIVIDER_HALF, scope.bottom)
                }
                Axis::Vertical => {
                    let y = top + (divider.position - viewport.top) * scale;
                    UiRect::new(scope.left, y - DIVIDER_HALF, scope.right, y + DIVIDER_HALF)
                }
            };
            divider_rects.push((divider.axis, r));
            zones.push(Zone {
                rect: r,
                placement: Placement::Between {
                    split: divider.split,
                    before: divider.before,
                    after: divider.after,
                },
                center: false,
                plan: None,
            });
        }
        for (dir, r) in [
            (
                Direction::Left,
                UiRect::new(
                    silhouette.left - OUTER_BAND,
                    silhouette.top,
                    silhouette.left,
                    silhouette.bottom,
                ),
            ),
            (
                Direction::Right,
                UiRect::new(
                    silhouette.right,
                    silhouette.top,
                    silhouette.right + OUTER_BAND,
                    silhouette.bottom,
                ),
            ),
            (
                Direction::Up,
                UiRect::new(
                    silhouette.left,
                    silhouette.top - OUTER_BAND,
                    silhouette.right,
                    silhouette.top,
                ),
            ),
            (
                Direction::Down,
                UiRect::new(
                    silhouette.left,
                    silhouette.bottom,
                    silhouette.right,
                    silhouette.bottom + OUTER_BAND,
                ),
            ),
        ] {
            zones.push(Zone {
                rect: r,
                placement: Placement::Outer(dir),
                center: false,
                plan: None,
            });
        }
        let mut cells = vec![];
        for (id, content, real) in snapshot.regions {
            let mut r = project(real);
            // The cell cannot claim its neighbour's divider band.
            for (axis, band) in &divider_rects {
                match axis {
                    Axis::Horizontal if r.top < band.bottom && r.bottom > band.top => {
                        if (r.left - (band.left + band.right) / 2.0).abs() < 0.1 {
                            r.left = band.right;
                        }
                        if (r.right - (band.left + band.right) / 2.0).abs() < 0.1 {
                            r.right = band.left;
                        }
                    }
                    Axis::Vertical if r.left < band.right && r.right > band.left => {
                        if (r.top - (band.top + band.bottom) / 2.0).abs() < 0.1 {
                            r.top = band.bottom;
                        }
                        if (r.bottom - (band.top + band.bottom) / 2.0).abs() < 0.1 {
                            r.bottom = band.top;
                        }
                    }
                    _ => {}
                }
            }
            let ew = EDGE_BAND.min(r.width() * 0.26);
            let eh = EDGE_BAND.min(r.height() * 0.26);
            for (dir, zone) in [
                (
                    Direction::Left,
                    UiRect::new(r.left, r.top, r.left + ew, r.bottom),
                ),
                (
                    Direction::Right,
                    UiRect::new(r.right - ew, r.top, r.right, r.bottom),
                ),
                (
                    Direction::Up,
                    UiRect::new(r.left + ew, r.top, r.right - ew, r.top + eh),
                ),
                (
                    Direction::Down,
                    UiRect::new(r.left + ew, r.bottom - eh, r.right - ew, r.bottom),
                ),
            ] {
                zones.push(Zone {
                    rect: zone,
                    placement: Placement::Side(id, dir),
                    center: false,
                    plan: None,
                });
            }
            zones.push(Zone {
                rect: UiRect::new(r.left + ew, r.top + eh, r.right - ew, r.bottom - eh),
                placement: Placement::Side(id, Direction::Right),
                center: true,
                plan: None,
            });
            cells.push((id, content, r));
        }
        for zone in &mut zones {
            zone.plan = layout.plan(
                session,
                &zone.placement,
                viewport,
                visible,
                sessions.clone(),
            );
        }
        Self {
            version: layout.version,
            viewport,
            visibility: visible,
            sessions,
            cells,
            zones,
        }
    }
}
pub fn refresh(app: &AppState, map: &MapStore, viewport: UiRect, sessions: Vec<u64>) -> bool {
    let visible = visibility(app);
    let Some(layout) = &app.application_layout else {
        return false;
    };
    map.update(|slot| {
        let Some(drag) = slot else {
            return false;
        };
        if !visible.terminal || layout.terminal_region(drag.session).is_none() {
            *slot = None;
            return true;
        }
        let stale = drag.scene.as_ref().is_none_or(|s| {
            s.version != layout.version
                || s.viewport != viewport
                || s.visibility != visible
                || s.sessions != sessions
        });
        if stale {
            drag.scene = Some(MapScene::build(
                layout,
                drag.session,
                viewport,
                visible,
                sessions,
            ));
        }
        let hover = drag.scene.as_ref().and_then(|s| s.hit(drag.point));
        let changed = drag.hover != hover;
        drag.hover = hover;
        stale || changed
    })
}
pub fn update_pointer(
    app: &AppState,
    map: &MapStore,
    point: Point,
    viewport: UiRect,
    sessions: Vec<u64>,
) -> bool {
    // Only a visible change notifies the overlay; plain movement inside the
    // same zone is recorded silently.
    let activated = map.update(|slot| {
        let Some(drag) = slot.as_mut().filter(|d| d.release.is_none()) else {
            return false;
        };
        drag.point = point;
        let activated =
            !drag.active && (point.x - drag.origin.x).hypot(point.y - drag.origin.y) >= 5.0;
        drag.active |= activated;
        activated
    });
    let pending = map.with(|d| d.is_some_and(|d| d.release.is_none()));
    pending && refresh(app, map, viewport, sessions) | activated
}
/// Return whether a drag was consumed and which session should receive focus.
pub fn finish(
    app: &mut AppState,
    map: &MapStore,
    point: Point,
    viewport: UiRect,
    sessions: &[u64],
) -> (bool, Option<u64>) {
    let mut taken = None;
    map.update(|slot| {
        taken = slot.take();
        taken.is_some()
    });
    let Some(drag) = taken else {
        return (false, None);
    };
    if !drag.active {
        return (false, Some(drag.session));
    }
    let visible = visibility(app);
    let plan = drag
        .scene
        .as_ref()
        .filter(|s| s.hit(point).is_some())
        .and_then(|s| drag.hover.and_then(|i| s.zones[i].plan.clone()));
    if let Some(plan) = plan
        && let Some(layout) = &mut app.application_layout
        && layout.commit(plan, viewport, visible, sessions)
    {
        return (true, Some(drag.session));
    }
    (true, None)
}
pub fn drag_cursor(map: &MapStore) -> CursorIcon {
    map.with(|d| match d.filter(|d| d.active) {
        Some(d)
            if d.scene
                .as_ref()
                .and_then(|s| d.hover.map(|i| &s.zones[i]))
                .is_some_and(|z| z.plan.is_some()) =>
        {
            CursorIcon::Move
        }
        Some(_) => CursorIcon::NotAllowed,
        None => CursorIcon::Pointer,
    })
}
/// Apply a finished drag's effect on the terminal tabs and focus.
fn settle(
    result: (bool, Option<u64>),
    tabs: &State<TerminalTabs>,
    focus: &lgui::core::UiFocusHandle,
) {
    if let Some(id) = result.1 {
        tabs.update(|tabs| {
            if result.0 {
                tabs.detach_group(id);
            }
            tabs.select(id);
        });
        focus.focus();
    }
}
pub fn attach(
    mut root: Element,
    state: State<AppState>,
    map: MapStore,
    tabs: State<TerminalTabs>,
    viewport: UiRect,
    focus: lgui::core::UiFocusHandle,
) -> Element {
    let cancel = map.clone();
    root = root.on_event_capture(UiEventKind::KeyDown, move |cx, payload| {
        if let UiEventPayload::Keyboard { event } = payload
            && event.key == LogicalKey::Named(NamedKey::Escape)
            && cancel.cancel()
        {
            cx.prevent_default();
            cx.stop_propagation();
        }
    });
    let stale = map.clone();
    root = root.on_event_capture(UiEventKind::PointerDown, move |_, _| {
        stale.cancel();
    });
    for kind in [UiEventKind::PointerMove, UiEventKind::PointerDrag] {
        let state = state.clone();
        let map = map.clone();
        let tabs = tabs.clone();
        root =
            root.on_event_capture(kind, move |cx, payload| {
                if let UiEventPayload::PointerMove { pointer }
                | UiEventPayload::PointerDrag { pointer } = payload
                    && map.is_dragging()
                {
                    let sessions = tabs.get().tabs().iter().map(|t| t.id).collect();
                    with_app(&state, |app| {
                        update_pointer(app, &map, pointer.point, viewport, sessions)
                    });
                    if map.is_active() {
                        cx.stop_propagation();
                    }
                }
            });
    }
    root.on_event_capture(UiEventKind::PointerUp, move |cx, payload| {
        if let UiEventPayload::PointerUp { pointer } = payload
            && map.is_dragging()
        {
            let result = if map.is_active() {
                // The overlay settles it next frame, once hover reflects
                // whether this was a real release or a cancellation.
                map.update(|d| {
                    if let Some(d) = d {
                        d.release = Some(pointer.point);
                    }
                    true
                });
                (true, None)
            } else {
                let sessions = tabs.get().tabs().iter().map(|t| t.id).collect::<Vec<_>>();
                with_app(&state, |app| {
                    finish(app, &map, pointer.point, viewport, &sessions)
                })
            };
            settle(result, &tabs, &focus);
            if result.0 {
                cx.prevent_default();
                cx.stop_propagation();
            }
        }
    })
}
/// The overlay is its own component: map changes re-render only it.
pub fn render(
    viewport: UiRect,
    state: State<AppState>,
    map: MapStore,
    tabs: State<TerminalTabs>,
    focus: lgui::core::UiFocusHandle,
) -> Element {
    component(viewport, move |cx, viewport| {
        cx.use_observable(map.observable(), |s: &MapSignal| s.revision);
        render_overlay(*viewport, state, map, tabs, focus)
    })
    .key("terminal-layout-map-component")
}
fn render_overlay(
    viewport: UiRect,
    state: State<AppState>,
    map: MapStore,
    tabs: State<TerminalTabs>,
    focus: lgui::core::UiFocusHandle,
) -> Element {
    use lgui::core::{EventPolicy, UiElement, UiId};
    let release_map = map.clone();
    let mut overlay = Element::new(move |cx| {
        let release = release_map.with(|d| {
            let drag = d?;
            let point = drag.release?;
            let hovered = drag.scene.as_ref().is_some_and(|scene| {
                (0..scene.zones.len()).any(|i| {
                    cx.context
                        .interaction_flags(&UiId::owned(format!("terminal-map-zone-{i}")))
                        .hovered
                })
            });
            Some((point, hovered))
        });
        if let Some((point, hovered)) = release {
            if hovered {
                let sessions = tabs.get().tabs().iter().map(|t| t.id).collect::<Vec<_>>();
                let mut result = (true, None);
                state.try_update(|app| {
                    result = finish(app, &release_map, point, viewport, &sessions);
                    result.1.is_some()
                });
                settle(result, &tabs, &focus);
            } else {
                release_map.cancel();
            }
        }
        UiElement::panel(cx.id, viewport, VisualStyle::default()).children(cx.children)
    })
    .key("terminal-layout-map");
    let Some(drag) = map.snapshot() else {
        return overlay;
    };
    let Some(scene) = &drag.scene else {
        return overlay;
    };
    // Invisible during the pending click. This lets the very first movement
    // into the map establish hover before the release/cancel distinction.
    for (i, zone) in scene.zones.iter().enumerate() {
        let rect = zone.rect;
        overlay = overlay.child(
            Element::new(move |cx| {
                UiElement::panel(
                    UiId::owned(format!("terminal-map-zone-{i}")),
                    rect,
                    VisualStyle::default(),
                )
                .children(cx.children)
            })
            .key(format!("map-hit-{i}"))
            .event_policy(EventPolicy {
                hover: true,
                press: false,
                focus: false,
            })
            .on_pointer_move(|_, _| {}),
        );
    }
    if !drag.active {
        return overlay;
    }
    let c = theme::c();
    // Only the miniature responds while dragging; the real window is left
    // untouched until release.
    let zone = drag.hover.map(|i| &scene.zones[i]);
    let moving = |content: &Content| matches!(content, Content::Terminal { tabs, .. } if tabs.contains(&drag.session));
    for (_, content, rect) in &scene.cells {
        let fill = match content {
            Content::Editor => c.surface,
            Content::Git | Content::Files => c.sidebar,
            Content::Terminal { .. } => c.bg,
        };
        let border = if moving(content) { c.accent } else { c.border };
        overlay = overlay.child(theme::bordered(
            inset(*rect, 1.0),
            fill,
            border,
            CELL_RADIUS,
            1.0,
        ));
    }
    // Window-edge bands have no cell to suggest them; mark each with a rail.
    for zone in &scene.zones {
        if let Placement::Outer(dir) = zone.placement {
            let r = zone.rect;
            let (mx, my) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
            let rail = match dir {
                Direction::Left | Direction::Right => {
                    UiRect::new(mx - 1.5, r.top + 8.0, mx + 1.5, r.bottom - 8.0)
                }
                Direction::Up | Direction::Down => {
                    UiRect::new(r.left + 8.0, my - 1.5, r.right - 8.0, my + 1.5)
                }
            };
            overlay = overlay.child(panel(
                rail,
                VisualStyle::filled(c.text_faint).alpha(110).radius(1.5),
            ));
        }
    }
    if let Some(zone) = zone {
        let color = if zone.plan.is_some() {
            c.accent
        } else {
            c.warning
        };
        // A body drop outlines the cell; bands are narrow enough to fill solidly.
        let style = if zone.center {
            VisualStyle::filled(color)
                .alpha(40)
                .stroked(Stroke::new(color, 2.0, 230))
                .radius(CELL_RADIUS - 2.0)
        } else {
            VisualStyle::filled(color).alpha(200).radius(3.0)
        };
        overlay = overlay.child(panel(zone.rect, style));
    }
    overlay
}

fn inset(r: UiRect, by: f32) -> UiRect {
    UiRect::new(r.left + by, r.top + by, r.right - by, r.bottom - by)
}

pub fn render_divider(
    divider: crate::model::application_layout::Divider,
    state: State<AppState>,
) -> Element {
    use lgui::core::{EventPolicy, PointerButton};
    let p = divider.position;
    let (line, hit, cursor) = match divider.axis {
        Axis::Horizontal => (
            UiRect::new(p - 0.5, divider.parent.top, p + 0.5, divider.parent.bottom),
            UiRect::new(p - 4.0, divider.parent.top, p + 4.0, divider.parent.bottom),
            CursorIcon::ResizeHorizontal,
        ),
        Axis::Vertical => (
            UiRect::new(divider.parent.left, p - 0.5, divider.parent.right, p + 0.5),
            UiRect::new(divider.parent.left, p - 4.0, divider.parent.right, p + 4.0),
            CursorIcon::ResizeVertical,
        ),
    };
    let active = state.get().application_sash_drag.as_ref().is_some_and(|d| {
        d.split == divider.split && d.before == divider.before && d.after == divider.after
    });
    let key = format!(
        "application-sash-{}-{}-{}",
        divider.split, divider.before, divider.after
    );
    let down = state.clone();
    let moving = state.clone();
    let up = state;
    panel(hit, VisualStyle::default())
        .key(key)
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(cursor)
        .on_pointer_down_with_button(move |cx, _, button| {
            if button == PointerButton::Left {
                down.update(|app| app.application_sash_drag = Some(divider.clone()));
                cx.stop_propagation();
            }
        })
        .on_pointer_drag(move |cx, pointer| {
            moving.try_update(|app| {
                let Some(drag) = &app.application_sash_drag else {
                    return false;
                };
                let visible = visibility(app);
                let Some(layout) = &mut app.application_layout else {
                    return false;
                };
                layout.resize(
                    drag,
                    match drag.axis {
                        Axis::Horizontal => pointer.point.x,
                        Axis::Vertical => pointer.point.y,
                    },
                    visible,
                )
            });
            cx.stop_propagation();
        })
        .on_pointer_up(move |cx, _| {
            up.update(|app| app.application_sash_drag = None);
            cx.stop_propagation();
        })
        .child(panel(
            line,
            VisualStyle::filled(if active {
                theme::c().accent
            } else {
                theme::c().border
            }),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn center(r: UiRect) -> Point {
        Point::new((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
    }
    fn setup() -> (AppState, MapStore, UiRect) {
        let r = UiRect::new(0.0, 0.0, 1200.0, 800.0);
        let mut app = AppState::new();
        app.show_terminal = true;
        app.show_source_control = true;
        let mut layout = ApplicationLayout::initial(r, 220.0, 220.0, 240.0);
        layout.sync(&[(1, 1), (2, 2)], Some(1));
        // The default strip remains region 4; a deliberately detached tab is 7.
        let plan = layout
            .plan(
                2,
                &Placement::Side(4, Direction::Right),
                r,
                visibility(&app),
                vec![1, 2],
            )
            .unwrap();
        layout = plan.layout;
        app.application_layout = Some(layout);
        let map = MapStore::default();
        begin(&mut app, &map, 1, Point::new(10.0, 600.0));
        update_pointer(&app, &map, center(r), r, vec![1, 2]);
        (app, map, r)
    }
    fn scene_of(map: &MapStore) -> MapScene {
        map.snapshot().unwrap().scene.unwrap()
    }
    #[test]
    fn all_map_cells_and_bands_have_explicit_targets_and_matching_geometry() {
        let (_, map, _) = setup();
        let scene = scene_of(&map);
        for (id, _, rect) in &scene.cells {
            let hit = &scene.zones[scene.hit(center(*rect)).unwrap()];
            assert_eq!(hit.placement, Placement::Side(*id, Direction::Right));
            assert!(contains(hit.rect, center(*rect)));
        }
        for (index, zone) in scene.zones.iter().enumerate() {
            let i = scene.hit(center(zone.rect)).unwrap();
            let hit = &scene.zones[i];
            // A larger divider's midpoint can itself be a T junction. Its
            // smaller-scope divider wins there; both use the same fill/hit.
            if matches!(zone.placement, Placement::Between { .. })
                && matches!(hit.placement, Placement::Between { .. })
            {
                assert!(i <= index);
            } else {
                assert_eq!(hit.placement, zone.placement);
            }
            assert!(contains(hit.rect, center(zone.rect)));
        }
        assert!(scene.hit(Point::new(50.0, 50.0)).is_none());
        let sole = scene
            .zones
            .iter()
            .find(|z| z.placement == Placement::Side(4, Direction::Right))
            .unwrap();
        assert!(sole.plan.is_none());
    }
    #[test]
    fn movement_inside_a_zone_notifies_nobody_and_zone_changes_notify_only_the_map() {
        let (app, map, r) = setup();
        let revision = || map.observable().read().revision;
        let scene = scene_of(&map);
        let body = center(scene.zones.iter().find(|z| z.center).unwrap().rect);
        update_pointer(&app, &map, body, r, vec![1, 2]);
        let before = revision();
        let nudge = Point::new(body.x + 1.0, body.y + 1.0);
        assert!(!update_pointer(&app, &map, nudge, r, vec![1, 2]));
        assert_eq!(revision(), before);
        assert_eq!(map.snapshot().unwrap().point, nudge);
        let edge = scene
            .zones
            .iter()
            .find(|z| z.placement == Placement::Outer(Direction::Left))
            .unwrap()
            .rect;
        assert!(update_pointer(&app, &map, center(edge), r, vec![1, 2]));
        assert_eq!(revision(), before + 1);
        assert_eq!(drag_cursor(&map), CursorIcon::Move);
    }
    #[test]
    fn release_commits_cached_preview_even_when_release_point_moves_to_other_zone() {
        let (mut app, map, r) = setup();
        let original = app.application_layout.clone();
        let target = scene_of(&map)
            .zones
            .iter()
            .find(|z| z.placement == Placement::Side(3, Direction::Right))
            .unwrap()
            .clone();
        update_pointer(&app, &map, center(target.rect), r, vec![1, 2]);
        assert_eq!(app.application_layout, original);
        let elsewhere = scene_of(&map)
            .zones
            .iter()
            .find(|z| z.placement == Placement::Outer(Direction::Left))
            .unwrap()
            .rect;
        assert_eq!(
            finish(&mut app, &map, center(elsewhere), r, &[1, 2]),
            (true, Some(1))
        );
        assert_eq!(app.application_layout, Some(target.plan.unwrap().layout));
    }
    #[test]
    fn outside_invalid_and_stale_release_cancel_without_mutation() {
        for mode in 0..3 {
            let (mut app, map, r) = setup();
            let original = app.application_layout.clone();
            let scene = scene_of(&map);
            let zone = scene
                .zones
                .iter()
                .find(|z| {
                    z.placement
                        == if mode == 1 {
                            Placement::Side(4, Direction::Right)
                        } else {
                            Placement::Outer(Direction::Left)
                        }
                })
                .unwrap()
                .rect;
            let point = center(zone);
            update_pointer(&app, &map, point, r, vec![1, 2]);
            let (point, ids) = match mode {
                0 => (Point::new(20.0, 20.0), vec![1, 2]),
                2 => (point, vec![2]),
                _ => (point, vec![1, 2]),
            };
            assert_eq!(finish(&mut app, &map, point, r, &ids), (true, None));
            assert_eq!(app.application_layout, original);
            assert!(!map.is_dragging());
        }
    }
    #[test]
    fn resize_reprojects_targets_and_drag_threshold_keeps_clicks_light() {
        let (mut app, map, r) = setup();
        let before = map.snapshot().unwrap().scene;
        let resized = UiRect::new(0.0, 0.0, 1500.0, 600.0);
        assert!(refresh(&app, &map, resized, vec![1, 2]));
        let after = &scene_of(&map);
        assert_ne!(&before.unwrap().cells, &after.cells);
        assert_eq!(after.viewport, resized);
        begin(&mut app, &map, 1, Point::new(10.0, 10.0));
        update_pointer(&app, &map, Point::new(12.0, 12.0), r, vec![1, 2]);
        assert!(!map.snapshot().unwrap().active);
        assert!(map.snapshot().unwrap().scene.is_some());
        assert_eq!(
            finish(&mut app, &map, Point::new(12.0, 12.0), r, &[1, 2]),
            (false, Some(1))
        );
    }

    #[test]
    fn t_junction_uses_smaller_split_scope() {
        let (mut app, map, r) = setup();
        let layout = app.application_layout.as_ref().unwrap();
        let plan = layout
            .plan(
                1,
                &Placement::Side(1, Direction::Down),
                r,
                visibility(&app),
                vec![1, 2],
            )
            .unwrap();
        app.application_layout = Some(plan.layout);
        begin(&mut app, &map, 2, Point::new(30.0, 600.0));
        update_pointer(&app, &map, center(r), r, vec![1, 2]);
        let scene = scene_of(&map);
        let nested = scene.zones.iter().find(|z| matches!(z.placement, Placement::Between { split, .. } if split != 5 && split != 6)).unwrap();
        let junction = Point::new(
            nested.rect.left + 0.25,
            (nested.rect.top + nested.rect.bottom) / 2.0,
        );
        assert_eq!(
            scene.zones[scene.hit(junction).unwrap()].placement,
            nested.placement
        );
    }

    #[test]
    fn shared_tab_strip_is_one_map_cell_and_detached_tabs_get_their_own_cell() {
        let (_, _, r) = setup();
        let map = MapStore::default();
        let mut app = AppState::new();
        app.show_terminal = true;
        let mut layout = ApplicationLayout::initial(r, 220.0, 220.0, 240.0);
        layout.sync(&[(1, 1), (2, 1)], Some(1));
        assert_eq!(
            layout
                .snapshot(r, visibility(&app))
                .regions
                .iter()
                .filter(|(_, c, _)| matches!(c, Content::Terminal { .. }))
                .count(),
            1
        );
        let plan = layout
            .plan(
                1,
                &Placement::Side(1, Direction::Right),
                r,
                visibility(&app),
                vec![1, 2],
            )
            .unwrap();
        layout = plan.layout;
        app.application_layout = Some(layout);
        begin(&mut app, &map, 1, Point::new(30.0, 575.0));
        update_pointer(&app, &map, center(r), r, vec![1, 2]);
        let scene = scene_of(&map);
        let terminals = scene
            .cells
            .iter()
            .filter(|(_, content, _)| matches!(content, Content::Terminal { .. }))
            .collect::<Vec<_>>();
        assert_eq!(terminals.len(), 2);
        assert_ne!(terminals[0].0, terminals[1].0);
        assert_ne!(terminals[0].2, terminals[1].2);
        for (id, _, rect) in terminals {
            let zone = &scene.zones[scene.hit(center(*rect)).unwrap()];
            assert_eq!(zone.placement, Placement::Side(*id, Direction::Right));
        }
    }

    #[test]
    fn real_terminal_pointer_capture_moves_individual_items_cancels_and_keeps_input_focus() {
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{
            EventPolicy, InputEvent, PointerButton, PointerData, UiElement, UiId, UiScale,
            dispatch_runtime_output,
        };
        use lgui::prelude::group;
        use lgui::session::UiSession;
        use std::sync::{Arc, Mutex};
        let viewport = UiRect::new(0.0, 0.0, 1200.0, 800.0);
        let exposed = Arc::new(Mutex::new(None::<(State<AppState>, State<TerminalTabs>)>));
        let output = exposed.clone();
        let application = Arc::new(lgui::ApplicationHandle::new(|task| task(), || {}));
        let map = MapStore::default();
        let view_map = map.clone();
        let view: AppView = Arc::new(move |cx| {
            let tabs = cx.state_with(|| {
                let mut t = TerminalTabs::new();
                t.split(1);
                t.select(1);
                t
            });
            let state = cx.state_with(|| {
                let mut a = AppState::new();
                a.show_terminal = true;
                a.show_source_control = true;
                a.application_layout =
                    Some(ApplicationLayout::initial(viewport, 220.0, 220.0, 240.0));
                a
            });
            *output.lock().unwrap() = Some((state.clone(), tabs.clone()));
            let t = tabs.get();
            let ids = t.tabs().iter().map(|t| t.id).collect::<Vec<_>>();
            state.update(|app| {
                app.application_layout.as_mut().unwrap().sync(
                    &t.tabs()
                        .iter()
                        .map(|t| (t.id, t.group_id))
                        .collect::<Vec<_>>(),
                    t.active_id(),
                );
                refresh(app, &view_map, viewport, ids);
            });
            let app = state.get();
            let snapshot = app
                .application_layout
                .as_ref()
                .unwrap()
                .snapshot(viewport, visibility(&app));
            let id = UiId::new("layout-terminal-focus");
            let focus = cx.focus_handle(id.clone());
            let editor_id = UiId::new("layout-editor-focus");
            let editor_focus = cx.focus_handle(editor_id.clone());
            let mut root = attach(
                group(viewport),
                state.clone(),
                view_map.clone(),
                tabs.clone(),
                viewport,
                focus.clone(),
            );
            for (region, content, r) in snapshot.regions {
                if matches!(content, Content::Terminal { .. }) {
                    root = root.child(crate::ui::terminal::render_region(
                        r,
                        state.clone(),
                        view_map.clone(),
                        editor_focus.clone(),
                        focus.clone(),
                        id.clone(),
                        tabs.clone(),
                        application.clone(),
                        true,
                        region,
                    ));
                } else {
                    let element_id = UiId::owned(format!("test-area-{region}"));
                    root = root.child(
                        Element::new(move |cx| {
                            UiElement::panel(element_id, r, VisualStyle::default())
                                .children(cx.children)
                        })
                        .event_policy(EventPolicy::INTERACTIVE),
                    );
                }
            }
            for d in snapshot.dividers {
                root = root.child(render_divider(d, state.clone()));
            }
            root.child(render(
                viewport,
                state.clone(),
                view_map.clone(),
                tabs.clone(),
                focus.clone(),
            ))
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let (state, tabs) = exposed.lock().unwrap().clone().unwrap();
        let context = ApplicationContext::empty(Default::default());
        let send = |session: &mut UiSession, input| {
            dispatch_runtime_output(
                session.handle_input(input),
                &context,
                &lgui::window::WindowId::new("layout-test"),
                |a| session.handle_default_action(a),
                |_| {},
            );
            session.render_view(&view, viewport, UiScale::ONE);
            // The release frame distinguishes normal Up from the framework's
            // synthetic Up on cancellation, then paints the committed model.
            session.render_view(&view, viewport, UiScale::ONE);
        };
        let down = |p| InputEvent::PointerDown {
            pointer: PointerData::mouse(p),
            button: PointerButton::Left,
        };
        let up = |p| InputEvent::PointerUp {
            pointer: PointerData::mouse(p),
            button: PointerButton::Left,
        };
        let header = Point::new(35.0, 575.0);
        // An ordinary click retains text-input focus without invoking layout.
        send(&mut session, down(header));
        send(&mut session, up(header));
        assert!(!map.is_dragging());
        assert_eq!(tabs.get().active_id(), Some(1));
        let controllers = tabs
            .get()
            .tabs()
            .iter()
            .map(|t| t.controller.clone())
            .collect::<Vec<_>>();
        for c in &controllers {
            c.begin_selection(0, 0);
            c.extend_selection(0, 2);
        }
        send(&mut session, InputEvent::TextInput("one".into()));
        assert!(controllers[0].snapshot().selection.is_empty());
        assert!(!controllers[1].snapshot().selection.is_empty());
        send(&mut session, down(header));
        send(
            &mut session,
            InputEvent::PointerMove(PointerData::mouse(Point::new(600.0, 400.0))),
        );
        assert!(map.snapshot().unwrap().active);
        let scene = scene_of(&map);
        let target = scene
            .zones
            .iter()
            .find(|z| z.placement == Placement::Outer(Direction::Left))
            .unwrap();
        let target_point = center(target.rect);
        send(
            &mut session,
            InputEvent::PointerMove(PointerData::mouse(target_point)),
        );
        let expected = scene_of(&map)
            .zones
            .into_iter()
            .find(|z| z.placement == Placement::Outer(Direction::Left))
            .unwrap()
            .plan
            .unwrap();
        send(&mut session, up(target_point));
        assert_eq!(state.get().application_layout.unwrap(), expected.layout);
        assert_eq!(tabs.get().tabs().len(), 2);
        for c in &controllers {
            c.begin_selection(0, 0);
            c.extend_selection(0, 2);
        }
        send(&mut session, InputEvent::TextInput("after move".into()));
        assert!(controllers[0].snapshot().selection.is_empty());
        assert!(!controllers[1].snapshot().selection.is_empty());
        // Start from its new header; leaving the window cancels the captured press.
        let topology = state.get().application_layout.unwrap().root;
        let sash_down = Point::new(600.0, 400.0);
        let sash_up = Point::new(520.0, 400.0);
        send(&mut session, down(sash_down));
        assert!(state.get().application_sash_drag.is_some());
        send(
            &mut session,
            InputEvent::PointerMove(PointerData::mouse(sash_up)),
        );
        send(&mut session, up(sash_up));
        let resized = state.get();
        assert!(resized.application_sash_drag.is_none());
        assert_eq!(resized.application_layout.as_ref().unwrap().root, topology);
        let snapshot = resized
            .application_layout
            .as_ref()
            .unwrap()
            .snapshot(viewport, visibility(&resized));
        let left = snapshot
            .regions
            .iter()
            .find(|(_, c, _)| matches!(c, Content::Terminal { tabs, .. } if tabs.contains(&1)))
            .unwrap()
            .2;
        assert!((left.width() - 520.0).abs() < 0.01);
        let moved = Point::new(35.0, 16.0);
        send(&mut session, down(moved));
        send(
            &mut session,
            InputEvent::PointerMove(PointerData::mouse(Point::new(600.0, 400.0))),
        );
        let before = state.get().application_layout.clone();
        send(
            &mut session,
            InputEvent::PointerLeave(PointerData::mouse(Point::new(-1.0, 400.0))),
        );
        assert!(!map.is_dragging());
        assert_eq!(state.get().application_layout, before);
        // Cancellation at a valid miniature point must also cancel: lgui
        // translates TouchPhase::Cancelled to PointerUp with that last point.
        send(&mut session, down(moved));
        send(
            &mut session,
            InputEvent::PointerMove(PointerData::mouse(Point::new(600.0, 400.0))),
        );
        let scene = scene_of(&map);
        let valid = center(
            scene
                .zones
                .iter()
                .find(|z| z.placement == Placement::Side(4, Direction::Right))
                .unwrap()
                .rect,
        );
        send(
            &mut session,
            InputEvent::PointerMove(PointerData::mouse(valid)),
        );
        send(
            &mut session,
            InputEvent::Touch {
                pointer: PointerData::mouse(valid),
                phase: lgui::core::TouchPhase::Cancelled,
            },
        );
        assert!(!map.is_dragging());
        assert_eq!(state.get().application_layout, before);
        send(&mut session, down(moved));
        send(
            &mut session,
            InputEvent::PointerMove(PointerData::mouse(Point::new(600.0, 400.0))),
        );
        send(
            &mut session,
            InputEvent::Keyboard(lgui::core::KeyboardEvent {
                state: lgui::core::KeyState::Down,
                key: LogicalKey::Named(NamedKey::Escape),
                ..Default::default()
            }),
        );
        assert!(!map.is_dragging());
        assert_eq!(state.get().application_layout, before);
        send(&mut session, up(Point::new(600.0, 400.0)));
        // Place beside the second terminal through that terminal's own cell.
        send(&mut session, down(moved));
        send(
            &mut session,
            InputEvent::PointerMove(PointerData::mouse(Point::new(600.0, 400.0))),
        );
        let scene = scene_of(&map);
        let adjacent = scene
            .zones
            .iter()
            .find(|z| z.placement == Placement::Side(4, Direction::Right))
            .unwrap();
        let adjacent_point = center(adjacent.rect);
        send(
            &mut session,
            InputEvent::PointerMove(PointerData::mouse(adjacent_point)),
        );
        send(&mut session, up(adjacent_point));
        assert_eq!(
            state
                .get()
                .application_layout
                .as_ref()
                .unwrap()
                .terminal_region(1),
            Some(7)
        );
        assert_eq!(
            state
                .get()
                .application_layout
                .as_ref()
                .unwrap()
                .leaves()
                .iter()
                .filter(|(_, c)| matches!(c, Content::Terminal { .. }))
                .count(),
            2
        );
        assert_eq!(tabs.get().tabs().len(), 2);
        // The + button of a moved item still creates in the default strip.
        let app = state.get();
        let layout = app.application_layout.as_ref().unwrap();
        let snapshot = layout.snapshot(viewport, visibility(&app));
        let moved_rect = snapshot.regions.iter().find(|r| r.0 == 7).unwrap().2;
        let plus = Point::new(moved_rect.right - 19.0, moved_rect.top + 17.0);
        send(&mut session, down(plus));
        send(&mut session, up(plus));
        assert_eq!(tabs.get().tabs().len(), 3);
        let app = state.get();
        let layout = app.application_layout.as_ref().unwrap();
        let snapshot = layout.snapshot(viewport, visibility(&app));
        let new_id = layout.terminal_region(3).unwrap();
        let new_rect = snapshot.regions.iter().find(|r| r.0 == new_id).unwrap().2;
        let default_rect = snapshot.regions.iter().find(|r| r.0 == 4).unwrap().2;
        assert_eq!(
            snapshot.regions.iter().find(|r| r.0 == 7).unwrap().2,
            moved_rect
        );
        assert_eq!(new_rect.top, default_rect.top);
        assert_eq!(new_rect.bottom, default_rect.bottom);
        assert_eq!(new_rect, default_rect);
        assert_eq!(layout.terminal_view(4), Some((vec![2, 3], Some(3))));
    }
}
