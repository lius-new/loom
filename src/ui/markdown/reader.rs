//! Controlled reading surface. Native paragraph painting is supplied through a
//! narrow port; this module owns neither parsing nor text/native resources.
use crate::features::markdown::layout::Decoration;
use crate::ui::components::scrollbar;
use crate::ui::theme;
use lgui::core::{
    CursorIcon, EventPolicy, KeyState, LogicalKey, NamedKey, PointerButton, SemanticRole,
    Semantics, UiElement, UiEventKind, UiFocusHandle, UiId, WheelUnit, clip,
};
use lgui::prelude::{Element, UiRect, VisualStyle, panel, text};
use std::sync::Arc;

pub use crate::features::markdown::reader::{
    FrameKey, Hit, Move, ParagraphGeometry, ReaderEvent, ReaderFrame,
};

type PaintImage = dyn Fn(&ReaderFrame, &crate::features::markdown::layout::ImageBox, UiRect) -> Element
    + Send
    + Sync;
type PaintParagraph = dyn Fn(&ReaderFrame, usize, UiRect, UiRect) -> Element + Send + Sync;
#[derive(Clone)]
pub struct Binding {
    read: Arc<dyn Fn() -> ReaderFrame + Send + Sync>,
    emit: Arc<dyn Fn(ReaderEvent) + Send + Sync>,
    paint: Arc<PaintParagraph>,
    image: Arc<PaintImage>,
}
impl Binding {
    /// The host keeps the publication/retention envelope alive with this binding.
    /// Paint must use the same styled paragraph request as summary and geometry.
    pub fn new(
        read: impl Fn() -> ReaderFrame + Send + Sync + 'static,
        emit: impl Fn(ReaderEvent) + Send + Sync + 'static,
        paint: impl Fn(&ReaderFrame, usize, UiRect, UiRect) -> Element + Send + Sync + 'static,
    ) -> Self {
        Self {
            read: Arc::new(read),
            emit: Arc::new(emit),
            paint: Arc::new(paint),
            image: Arc::new(|frame, image, rect| {
                text(
                    rect,
                    frame
                        .document
                        .text(image.display.clone())
                        .unwrap_or("Image")
                        .to_owned(),
                    theme::sans(theme::c().text_soft, 14.0),
                )
                .into()
            }),
        }
    }
    pub fn with_images(
        mut self,
        paint: impl Fn(&ReaderFrame, &crate::features::markdown::layout::ImageBox, UiRect) -> Element
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.image = Arc::new(paint);
        self
    }
    pub fn get(&self) -> ReaderFrame {
        (self.read)()
    }
    pub fn send(&self, event: ReaderEvent) {
        (self.emit)(event);
    }
}
fn hit(frame: &ReaderFrame, rect: UiRect, point: (f32, f32)) -> Option<Hit> {
    if frame.geometry_key != frame.key {
        return None;
    }
    crate::features::markdown::reader::hit_at(
        &frame.layout,
        &frame.geometry,
        &frame.horizontal,
        frame.viewport,
        (point.0 - rect.left, point.1 - rect.top),
    )
}
fn translated(
    rect: crate::features::markdown::layout::TextRect,
    viewport: UiRect,
    scroll: f32,
) -> UiRect {
    UiRect::new(
        viewport.left + rect.left,
        viewport.top + rect.top - scroll,
        viewport.left + rect.right,
        viewport.top + rect.bottom - scroll,
    )
}
/// Only visible paragraphs and bounded visible glyph rectangles enter the scene.
/// Selection/copy remains a full-document logical range outside this scene.
pub fn render(id: UiId, rect: UiRect, binding: Binding, focus: UiFocusHandle) -> Element {
    let emphasized = false;
    render_in(id, rect, rect, emphasized, binding, focus)
}
/// `area` is the whole pane; `rect` is the (possibly narrower, centered)
/// reading column. Wheel input and the vertical scrollbar use the pane edge.
pub fn render_in(
    id: UiId,
    area: UiRect,
    rect: UiRect,
    emphasized: bool,
    binding: Binding,
    focus: UiFocusHandle,
) -> Element {
    let frame = binding.get();
    let semantic_scope = id.to_string();
    let mut root = Element::new(move |cx| {
        UiElement::panel(id, area, VisualStyle::filled(theme::c().bg))
            .semantics(Semantics::new(SemanticRole::Group).name("Markdown document"))
            .children(cx.children)
    })
    .event_policy(EventPolicy::INTERACTIVE)
    .cursor(CursorIcon::Default);
    for kind in [
        UiEventKind::Input,
        UiEventKind::CompositionStart,
        UiEventKind::CompositionUpdate,
        UiEventKind::CompositionEnd,
    ] {
        root = root.on_event_capture(kind, |cx, _| {
            cx.prevent_default();
            cx.stop_propagation();
        });
    }
    let mut scrollbar_layer = None;
    // I-beam only over the reading column, as in the editor; margins and scrollbars keep the arrow.
    let mut content = clip(rect, 0.0, 0.0)
        .key("markdown-reading-content")
        .cursor(CursorIcon::Text);
    for image in &frame.layout.images {
        let bounds = translated(image.rect, rect, frame.viewport.top);
        if bounds.bottom > rect.top && bounds.top < rect.bottom {
            content = content.child((binding.image)(&frame, image, bounds));
        }
    }
    for decoration in &frame.layout.decorations {
        match decoration {
            Decoration::Fallback { node, rect: bounds } => {
                let mut bounds = translated(*bounds, rect, frame.viewport.top);
                if let Some(container) = frame.layout.node_scroll_container(*node) {
                    let offset = frame.horizontal.get(&container).copied().unwrap_or(0.0);
                    bounds.left -= offset;
                    bounds.right -= offset;
                }
                if bounds.bottom > rect.top && bounds.top < rect.bottom {
                    content=content.child(panel(bounds,VisualStyle::filled(theme::c().surface)).child(text(bounds,"Large block exceeds the preview budget. Toggle the preview to read the source; Select All / Copy includes its complete text.",theme::sans(theme::c().warning,13.0))));
                }
            }
            Decoration::Rule { y, left, right } => {
                let top = rect.top + y - frame.viewport.top;
                if top >= rect.top && top <= rect.bottom {
                    content = content.child(panel(
                        UiRect::new(rect.left + left, top, rect.left + right, top + 1.0),
                        VisualStyle::filled(theme::c().border),
                    ));
                }
            }
            Decoration::Quote { rect: bounds } => {
                let bounds = translated(*bounds, rect, frame.viewport.top);
                if bounds.bottom > rect.top && bounds.top < rect.bottom {
                    content = content.child(panel(bounds, VisualStyle::filled(theme::c().border)));
                }
            }
            Decoration::Code {
                node, rect: bounds, ..
            } => {
                let bounds = translated(*bounds, rect, frame.viewport.top);
                if bounds.bottom > rect.top && bounds.top < rect.bottom {
                    content = content.child(panel(
                        bounds,
                        VisualStyle::filled(theme::c().surface).radius(4.0),
                    ));
                    let copy = binding.clone();
                    let node = *node;
                    let button = UiRect::new(
                        (bounds.right - 52.0).max(bounds.left),
                        bounds.top + 2.0,
                        bounds.right - 4.0,
                        bounds.top + 20.0,
                    );
                    content = content.child(
                        panel(button, VisualStyle::default())
                            .event_policy(EventPolicy {
                                focus: false,
                                ..EventPolicy::INTERACTIVE
                            })
                            .cursor(CursorIcon::Pointer)
                            .semantics(Semantics::new(SemanticRole::Button).name("Copy code"))
                            .child(text(
                                button,
                                "Copy",
                                theme::sans(theme::c().text_soft, 11.0),
                            ))
                            .on_pointer_down_with_button(|cx, _, _| cx.stop_propagation())
                            .on_click(move |cx: &mut lgui::core::UiEventContext| {
                                let frame = copy.get();
                                copy.send(ReaderEvent::CopyNode {
                                    key: frame.key,
                                    node,
                                });
                                cx.stop_propagation();
                            }),
                    );
                }
            }
            Decoration::ListMarker {
                node,
                display,
                x,
                y,
            } => {
                let top = rect.top + y - frame.viewport.top;
                if top + frame.layout.style.line_height > rect.top && top < rect.bottom {
                    content = content.child(text(
                        UiRect::new(
                            rect.left + x,
                            top,
                            rect.left + x + frame.layout.style.indent,
                            top + frame.layout.style.line_height,
                        ),
                        match frame.document.nodes[node.0].kind {
                            crate::features::markdown::model::NodeKind::ListItem {
                                task: Some(done),
                                ..
                            } => if done { "☑" } else { "☐" }.to_owned(),
                            _ => frame
                                .document
                                .text(display.clone())
                                .unwrap_or("")
                                .trim()
                                .to_owned(),
                        },
                        theme::sans(theme::c().text, 16.0 * frame.layout.style.scale),
                    ));
                }
            }
            Decoration::TableCell {
                node,
                rect: bounds,
                header,
                ..
            } => {
                let mut bounds = translated(*bounds, rect, frame.viewport.top);
                let offset = frame
                    .layout
                    .node_scroll_container(*node)
                    .and_then(|node| frame.horizontal.get(&node).copied())
                    .unwrap_or(0.0);
                bounds.left -= offset;
                bounds.right -= offset;
                if bounds.bottom > rect.top && bounds.top < rect.bottom {
                    if *header {
                        content = content
                            .child(panel(bounds, VisualStyle::filled(theme::c().active_line)));
                    }
                    content = content
                        .child(panel(
                            UiRect::new(
                                bounds.right - 1.0,
                                bounds.top,
                                bounds.right,
                                bounds.bottom,
                            ),
                            VisualStyle::filled(theme::c().border),
                        ))
                        .child(panel(
                            UiRect::new(
                                bounds.left,
                                bounds.bottom - 1.0,
                                bounds.right,
                                bounds.bottom,
                            ),
                            VisualStyle::filled(theme::c().border),
                        ));
                }
            }
        }
    }
    if frame.geometry_key == frame.key
        && let Some(selected) = frame
            .selection
            .and_then(|selection| selection.range(&frame.document, frame.key.parse))
    {
        for image in &frame.layout.images {
            if selected.start < image.display.end && image.display.start < selected.end {
                let bounds = translated(image.rect, rect, frame.viewport.top);
                content = content.child(panel(
                    bounds,
                    VisualStyle::filled(theme::c().selection).alpha(110),
                ));
            }
        }
        for geometry in frame.geometry.iter() {
            let Some(paragraph) = frame.layout.paragraphs.get(geometry.paragraph) else {
                continue;
            };
            for glyph in geometry.glyphs.iter() {
                let start = paragraph.display.start.0 + glyph.bytes.start;
                let end = paragraph.display.start.0 + glyph.bytes.end;
                if start < selected.end.0 && end > selected.start.0 {
                    let bounds = glyph.rect;
                    let bounds = UiRect::new(
                        rect.left + paragraph.x
                            - frame
                                .layout
                                .scroll_container(geometry.paragraph)
                                .and_then(|node| frame.horizontal.get(&node).copied())
                                .unwrap_or(0.0)
                            + bounds.left,
                        rect.top + paragraph.y + bounds.top - frame.viewport.top,
                        rect.left + paragraph.x
                            - frame
                                .layout
                                .scroll_container(geometry.paragraph)
                                .and_then(|node| frame.horizontal.get(&node).copied())
                                .unwrap_or(0.0)
                            + bounds.right,
                        rect.top + paragraph.y + bounds.bottom - frame.viewport.top,
                    );
                    content =
                        content.child(panel(bounds, VisualStyle::filled(theme::c().selection)));
                }
            }
        }
    }
    for index in frame
        .viewport
        .visible(&frame.layout, frame.layout.style.line_height * 2.0)
    {
        for paragraph in frame.layout.boxes[index].paragraphs.clone() {
            let value = &frame.layout.paragraphs[paragraph];
            let bounds = UiRect::new(
                rect.left + value.x
                    - frame
                        .layout
                        .scroll_container(paragraph)
                        .and_then(|node| frame.horizontal.get(&node).copied())
                        .unwrap_or(0.0),
                rect.top + value.y - frame.viewport.top,
                rect.left + value.x
                    - frame
                        .layout
                        .scroll_container(paragraph)
                        .and_then(|node| frame.horizontal.get(&node).copied())
                        .unwrap_or(0.0)
                    + value.viewport_width,
                rect.top + value.y + value.summary.height - frame.viewport.top,
            );
            content = content.child((binding.paint)(&frame, paragraph, bounds, rect));
        }
    }
    // Semantic text uses the same visible glyph bounds as hit testing. These
    // transparent nodes do not shape or paint text a second time.
    if frame.geometry_key == frame.key {
        for geometry in frame.geometry.iter() {
            let Some(paragraph) = frame.layout.paragraphs.get(geometry.paragraph) else {
                continue;
            };
            let value = frame.document.text(paragraph.display.clone()).unwrap_or("");
            let mut offset = 0;
            for word in value.split_whitespace() {
                let Some(found) = value[offset..].find(word) else {
                    continue;
                };
                let start = offset + found;
                let end = start + word.len();
                offset = end;
                let from = geometry
                    .glyphs
                    .partition_point(|glyph| glyph.bytes.end <= start);
                let mut bounds: Option<UiRect> = None;
                for glyph in geometry.glyphs[from..]
                    .iter()
                    .take_while(|glyph| glyph.bytes.start < end)
                {
                    let b = glyph.rect;
                    let r = UiRect::new(
                        rect.left + paragraph.x
                            - frame
                                .layout
                                .scroll_container(geometry.paragraph)
                                .and_then(|node| frame.horizontal.get(&node).copied())
                                .unwrap_or(0.0)
                            + b.left,
                        rect.top + paragraph.y + b.top - frame.viewport.top,
                        rect.left + paragraph.x
                            - frame
                                .layout
                                .scroll_container(geometry.paragraph)
                                .and_then(|node| frame.horizontal.get(&node).copied())
                                .unwrap_or(0.0)
                            + b.right,
                        rect.top + paragraph.y + b.bottom - frame.viewport.top,
                    );
                    bounds = Some(bounds.map_or(r, |old| {
                        UiRect::new(
                            old.left.min(r.left),
                            old.top.min(r.top),
                            old.right.max(r.right),
                            old.bottom.max(r.bottom),
                        )
                    }));
                }
                if let Some(bounds) =
                    bounds.filter(|bounds| bounds.bottom > rect.top && bounds.top < rect.bottom)
                {
                    let linked = paragraph
                        .spans
                        .iter()
                        .find(|span| span.bytes.start <= start && start < span.bytes.end)
                        .and_then(|span| span.style.link)
                        .and_then(|node| frame.document.node(node))
                        .and_then(|node| match &node.kind {
                            crate::features::markdown::model::NodeKind::Link {
                                destination,
                                ..
                            } => Some(destination.clone()),
                            _ => None,
                        });
                    let word = word.to_owned();
                    let semantics = match linked {
                        Some(url) => Semantics::new(SemanticRole::Link)
                            .name(word.clone())
                            .value(word)
                            .description(url),
                        None => Semantics::new(SemanticRole::Text).value(word),
                    };
                    let semantic_id = UiId::owned(format!(
                        "{semantic_scope}-text-{}-{start}",
                        geometry.paragraph
                    ));
                    content = content.child(Element::new(move |_| {
                        UiElement::panel(semantic_id, bounds, VisualStyle::default())
                            .semantics(semantics)
                    }));
                }
            }
        }
    }
    for block in &frame.layout.horizontal {
        let max = frame.layout.max_scroll(block.node);
        if max <= 0.0 {
            continue;
        }
        let bounds = translated(block.rect, rect, frame.viewport.top);
        if bounds.bottom < rect.top || bounds.top > rect.bottom {
            continue;
        }
        let node = block.node;
        let wheel = binding.clone();
        let surface = panel(bounds, VisualStyle::default())
            .event_policy(EventPolicy {
                focus: false,
                ..EventPolicy::INTERACTIVE
            })
            .on_wheel(move |cx, delta| {
                let frame = wheel.get();
                if delta.x.abs() > delta.y.abs() || frame.extend_selection {
                    let factor = if delta.unit == WheelUnit::Lines {
                        40.0
                    } else {
                        1.0
                    };
                    let delta = if delta.x != 0.0 { -delta.x } else { -delta.y };
                    wheel.send(ReaderEvent::HorizontalScroll {
                        key: frame.key,
                        node,
                        delta: delta * factor,
                    });
                    cx.stop_propagation();
                }
            });
        content = content.child(surface);
        let track = UiRect::new(
            bounds.left,
            bounds.bottom - scrollbar::SIZE,
            bounds.right,
            bounds.bottom,
        );
        let current = frame
            .horizontal
            .get(&node)
            .copied()
            .unwrap_or(0.0)
            .clamp(0.0, max);
        let on_scroll = binding.clone();
        content = content.child(axis_scrollbar(
            false,
            track,
            block.extent,
            track.width(),
            current,
            max,
            emphasized,
            move |position| {
                let frame = on_scroll.get();
                let previous = frame.horizontal.get(&node).copied().unwrap_or(0.0);
                on_scroll.send(ReaderEvent::HorizontalScroll {
                    key: frame.key,
                    node,
                    delta: position - previous,
                });
            },
        ));
    }
    if frame.layout.height > frame.viewport.height {
        let max = (frame.layout.height - frame.viewport.height).max(1.0);
        let track = UiRect::new(
            area.right - scrollbar::SIZE,
            area.top,
            area.right,
            area.bottom,
        );
        let on_scroll = binding.clone();
        // Drawn on the root, not the clipped column, so it sits at the pane edge.
        scrollbar_layer = Some(axis_scrollbar(
            true,
            track,
            frame.layout.height,
            track.height(),
            frame.viewport.top,
            max,
            emphasized,
            move |position| {
                let frame = on_scroll.get();
                on_scroll.send(ReaderEvent::Scroll {
                    key: frame.key,
                    delta: position - frame.viewport.top,
                });
            },
        ));
    }
    root = root.child(content);
    if let Some(layer) = scrollbar_layer {
        root = root.child(layer);
    }
    let down = binding.clone();
    let down_focus = focus.clone();
    root = root.on_pointer_down_with_button(move |cx, pointer, button| {
        if button != PointerButton::Left {
            return;
        }
        down_focus.focus();
        let frame = down.get();
        down.send(ReaderEvent::PointerDown {
            key: frame.key,
            hit: hit(&frame, rect, (pointer.point.x, pointer.point.y)),
            extend: frame.extend_selection,
        });
        cx.stop_propagation();
    });
    let drag = binding.clone();
    root = root.on_pointer_drag(move |cx, pointer| {
        let frame = drag.get();
        let point = (pointer.point.x, pointer.point.y);
        drag.send(ReaderEvent::PointerDrag {
            key: frame.key,
            hit: hit(&frame, rect, point),
            point: (point.0 - rect.left, point.1 - rect.top),
        });
        cx.stop_propagation();
    });
    let up = binding.clone();
    root = root.on_pointer_up(move |_, pointer| {
        let frame = up.get();
        up.send(ReaderEvent::PointerUp {
            key: frame.key,
            hit: hit(&frame, rect, (pointer.point.x, pointer.point.y)),
        });
    });
    let wheel = binding.clone();
    root = root.on_wheel(move |cx, delta| {
        let frame = wheel.get();
        let amount = match delta.unit {
            WheelUnit::Lines => -delta.y * frame.layout.style.line_height * 3.0,
            WheelUnit::Pixels => -delta.y,
        };
        wheel.send(ReaderEvent::Scroll {
            key: frame.key,
            delta: amount,
        });
        cx.stop_propagation();
    });
    let keyboard = binding.clone();
    root = root.on_key_down(move |cx, event| {
        if event.state != KeyState::Down {
            return;
        }
        let frame = keyboard.get();
        let command = event.modifiers.ctrl() || event.modifiers.meta();
        let action = match &event.key {
            LogicalKey::Character(value) if command && value.eq_ignore_ascii_case("c") => {
                Some(ReaderEvent::Copy { key: frame.key })
            }
            LogicalKey::Character(value) if command && value.eq_ignore_ascii_case("a") => {
                Some(ReaderEvent::SelectAll { key: frame.key })
            }
            LogicalKey::Named(NamedKey::Tab) => crate::features::markdown::reader::next_link(
                &frame.document,
                frame.selection,
                frame.key.parse,
                event.modifiers.shift(),
            )
            .map(|node| ReaderEvent::FocusLink {
                key: frame.key,
                node,
            }),
            LogicalKey::Named(NamedKey::Enter) => crate::features::markdown::reader::active_link(
                &frame.document,
                frame.selection,
                frame.key.parse,
            )
            .map(|node| ReaderEvent::OpenLink {
                key: frame.key,
                node,
            }),
            LogicalKey::Named(NamedKey::Escape) => {
                Some(ReaderEvent::ClearSelection { key: frame.key })
            }
            LogicalKey::Named(NamedKey::PageDown) => Some(ReaderEvent::Scroll {
                key: frame.key,
                delta: frame.viewport.height * 0.9,
            }),
            LogicalKey::Named(NamedKey::PageUp) => Some(ReaderEvent::Scroll {
                key: frame.key,
                delta: -frame.viewport.height * 0.9,
            }),
            LogicalKey::Named(key) => {
                let direction = match key {
                    NamedKey::ArrowLeft => Some(Move::Left),
                    NamedKey::ArrowRight => Some(Move::Right),
                    NamedKey::ArrowUp => Some(Move::Up),
                    NamedKey::ArrowDown => Some(Move::Down),
                    NamedKey::Home => Some(Move::Start),
                    NamedKey::End => Some(Move::End),
                    _ => None,
                };
                direction.map(|direction| ReaderEvent::Move {
                    key: frame.key,
                    direction,
                    extend: event.modifiers.shift(),
                })
            }
            _ => None,
        };
        if let Some(action) = action {
            keyboard.send(action);
            cx.prevent_default();
            cx.stop_propagation();
        } else if !matches!(event.key, LogicalKey::Named(NamedKey::Tab)) {
            cx.prevent_default();
            cx.stop_propagation();
        }
    });
    // Reading input and composition are intercepted above, never sent to text edits.
    let focused = binding.clone();
    root = root.on_focus(move |_| focused.send(ReaderEvent::Focus(true)));
    root.on_blur(move |_| binding.send(ReaderEvent::Focus(false)))
}
/// Pointer offset inside the thumb while a scrollbar is held. Only one pointer
/// drags at a time, and the element tree is rebuilt on every scroll step.
static GRAB: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
fn grab() -> f32 {
    f32::from_bits(GRAB.load(std::sync::atomic::Ordering::Relaxed))
}
fn set_grab(value: f32) {
    GRAB.store(value.to_bits(), std::sync::atomic::Ordering::Relaxed);
}
/// Overlay scrollbar that behaves like the editor's: dimmer at rest, brighter
/// while the pane is hovered, the thumb keeps the grab offset, and a click on
/// the track centers the thumb and continues as a drag.
#[allow(clippy::too_many_arguments)]
fn axis_scrollbar(
    vertical: bool,
    track: UiRect,
    content: f32,
    viewport: f32,
    scroll: f32,
    max: f32,
    emphasized: bool,
    on_scroll: impl Fn(f32) + Clone + Send + Sync + 'static,
) -> Element {
    let (start, length) = if vertical {
        (track.top, track.height())
    } else {
        (track.left, track.width())
    };
    let thumb = (length * viewport / content.max(1.0))
        .max(scrollbar::MIN_THUMB_LENGTH)
        .min(length);
    let travel = (length - thumb).max(0.0);
    let thumb_start = start + travel * (scroll / max.max(1.0)).clamp(0.0, 1.0);
    let pick = move |x: f32, y: f32| if vertical { y } else { x };
    let to_scroll = move |thumb_start: f32| {
        if travel <= 0.0 {
            0.0
        } else {
            (thumb_start.clamp(0.0, travel) / travel * max).clamp(0.0, max)
        }
    };
    let (down, drag, held) = (on_scroll.clone(), on_scroll.clone(), on_scroll);
    let track_hit = scrollbar::track(track)
        .event_policy(EventPolicy::INTERACTIVE)
        .on_pointer_down_with_button(move |cx, pointer, button| {
            if button != PointerButton::Left {
                return;
            }
            set_grab(thumb * 0.5);
            down(to_scroll(
                pick(pointer.point.x, pointer.point.y) - start - thumb * 0.5,
            ));
            cx.stop_propagation();
        })
        .on_pointer_drag(move |cx, pointer| {
            drag(to_scroll(
                pick(pointer.point.x, pointer.point.y) - start - grab(),
            ));
            cx.stop_propagation();
        });
    let rect = if vertical {
        UiRect::new(
            track.left + scrollbar::INSET,
            thumb_start,
            track.right - scrollbar::INSET,
            thumb_start + thumb,
        )
    } else {
        UiRect::new(
            thumb_start,
            track.top + scrollbar::INSET,
            thumb_start + thumb,
            track.bottom - scrollbar::INSET,
        )
    };
    let thumb_hit = scrollbar::thumb(rect, if emphasized { 0xb8 } else { 0x68 })
        .event_policy(EventPolicy::INTERACTIVE)
        .on_pointer_down_with_button(move |cx, pointer, button| {
            if button != PointerButton::Left {
                return;
            }
            set_grab(pick(pointer.point.x, pointer.point.y) - thumb_start);
            cx.stop_propagation();
        })
        .on_pointer_drag(move |cx, pointer| {
            held(to_scroll(
                pick(pointer.point.x, pointer.point.y) - start - grab(),
            ));
            cx.stop_propagation();
        });
    lgui::core::group(track).child(track_hit).child(thumb_hit)
}
