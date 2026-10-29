//! Image canvas, compact status controls and menus; no disk or application access.
use crate::domain::media::{Background, Image, Position};
use crate::ui::theme;
use lgui::core::{
    CursorIcon, EventPolicy, ImageFit, PathStyle, Point, PointerButton, SemanticRole, Semantics,
    UiElement, UiEventContext, UiEventKind, UiEventPayload, UiId, UiImageSource, UiPath,
    UiPathCommand, clip, path,
};
use lgui::prelude::{Color, Element, ShadowStyle, TextAlign, UiRect, VisualStyle, panel, text};
use std::sync::Arc;

pub const FOCUS_ID: &str = "image.preview.menu";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    Zoom,
    Background,
}
#[derive(Clone, Copy)]
pub enum Command {
    Begin(f32, f32),
    Pointer(f32, f32),
    End,
    Wheel(f32, f32),
    Focus,
    Blur,
    Zoom(Option<f32>),
    Background(Background),
    Menu(MenuKind),
    CloseMenu,
    Hover(Option<usize>),
    Choose(usize),
    Toggle,
}
pub type Send = Arc<dyn Fn(Command) + std::marker::Send + Sync>;
#[derive(Clone)]
pub struct Frame {
    pub image: Option<Arc<Image>>,
    pub animation_frame: Option<crate::domain::media::Frame>,
    pub format: &'static str,
    pub byte_len: u64,
    pub position: Position,
    pub bounds: UiRect,
    pub error: Option<String>,
    pub show_loading: bool,
}
pub fn image_bounds(frame: &Frame) -> Option<UiRect> {
    let image = frame.image.as_ref()?;
    let rect = frame.bounds;
    let scale = frame.position.scale_for(rect.width(), rect.height(), image);
    let w = image.width as f32 * scale;
    let h = image.height as f32 * scale;
    let max_x = ((w - rect.width()) / 2.0 + 24.0).max(0.0);
    let max_y = ((h - rect.height()) / 2.0 + 24.0).max(0.0);
    let left = (rect.left + rect.right - w) / 2.0 + frame.position.x.clamp(-max_x, max_x);
    let top = (rect.top + rect.bottom - h) / 2.0 + frame.position.y.clamp(-max_y, max_y);
    Some(UiRect::new(left, top, left + w, top + h))
}
pub fn render(frame: Frame, id: UiId, focus: lgui::core::UiFocusHandle, send: Send) -> Element {
    let rect = frame.bounds;
    let key = format!("image-preview.{}", id.as_str());
    let pixels_id = UiId::owned(format!("{key}.pixels"));
    let error_id = UiId::owned(format!("{key}.error"));
    let can_pan = image_bounds(&frame).is_some_and(|bounds| {
        bounds.width() > rect.width() - 48.0 || bounds.height() > rect.height() - 48.0
    });
    let mut root = Element::new(move |cx| {
        UiElement::panel(id, rect, VisualStyle::filled(theme::c().bg))
            .semantics(Semantics::new(SemanticRole::Group).name("Image preview"))
            .children(cx.children)
    })
    .event_policy(EventPolicy::INTERACTIVE)
    .cursor(if can_pan {
        CursorIcon::Move
    } else {
        CursorIcon::Default
    });
    let down = send.clone();
    root = root.on_pointer_down_with_button(move |cx, pointer, button| {
        if button != PointerButton::Left {
            return;
        }
        focus.focus();
        down(Command::Begin(pointer.point.x, pointer.point.y));
        cx.stop_propagation();
    });
    let pointer = send.clone();
    root = root.on_event(UiEventKind::PointerMove, move |_, payload| {
        if let UiEventPayload::PointerMove { pointer: p } = payload {
            pointer(Command::Pointer(p.point.x, p.point.y));
        }
    });
    let up = send.clone();
    root = root.on_pointer_up(move |_, _| up(Command::End));
    let wheel = send.clone();
    root = root.on_wheel(move |cx, delta| {
        let factor = if delta.unit == lgui::core::WheelUnit::Lines {
            36.0
        } else {
            1.0
        };
        wheel(Command::Wheel(delta.x * factor, delta.y * factor));
        cx.stop_propagation();
    });
    let blur = send.clone();
    root = root
        .on_focus(move |_| send(Command::Focus))
        .on_blur(move |_| blur(Command::Blur));
    let Some(image) = frame.image.as_ref() else {
        if let Some(error) = frame.error {
            return root.child(crate::ui::file_error::render(rect, &error, error_id));
        }
        if !frame.show_loading {
            return root;
        }
        let mut style = theme::sans(theme::c().text_muted, theme::UI_SIZE);
        style.align = TextAlign::Center;
        return root.child(clip(rect, 0.0, 0.0).child(text(
            UiRect::new(
                rect.left,
                (rect.top + rect.bottom) / 2.0,
                rect.right,
                (rect.top + rect.bottom) / 2.0 + 24.0,
            ),
            "Preparing image preview…",
            style,
        )));
    };
    let bounds = image_bounds(&frame).unwrap();
    let (base, check) = match frame.position.background {
        Background::Light => (Color(0xffffff), None),
        Background::Dark => (Color(0x111111), None),
        Background::Checkerboard => (theme::c().surface, Some(theme::c().active_tab)),
    };
    let mut content = clip(rect, 0.0, 0.0).child(panel(bounds, VisualStyle::filled(base)));
    if let Some(color) = check {
        // One compound path, restricted to visible cells, instead of thousands
        // of reactive tile elements when the image is zoomed in.
        let left = bounds.left.max(rect.left);
        let top = bounds.top.max(rect.top);
        let right = bounds.right.min(rect.right);
        let bottom = bounds.bottom.min(rect.bottom);
        let cell = 8.0;
        let mut commands = Vec::new();
        let first_x = ((left - bounds.left) / cell).floor().max(0.0) as i32;
        let first_y = ((top - bounds.top) / cell).floor().max(0.0) as i32;
        let end_x = ((right - bounds.left) / cell).ceil() as i32;
        let end_y = ((bottom - bounds.top) / cell).ceil() as i32;
        for y in first_y..end_y {
            for x in first_x..end_x {
                if (x + y) % 2 != 0 {
                    continue;
                }
                let l = (bounds.left + x as f32 * cell).max(left);
                let t = (bounds.top + y as f32 * cell).max(top);
                let r = (bounds.left + (x + 1) as f32 * cell).min(right);
                let b = (bounds.top + (y + 1) as f32 * cell).min(bottom);
                commands.extend([
                    UiPathCommand::MoveTo(Point::new(l, t)),
                    UiPathCommand::LineTo(Point::new(r, t)),
                    UiPathCommand::LineTo(Point::new(r, b)),
                    UiPathCommand::LineTo(Point::new(l, b)),
                    UiPathCommand::Close,
                ]);
            }
        }
        content = content.child(path(
            rect,
            UiPath::new(commands),
            PathStyle {
                fill: Some(color),
                fill_alpha: 255,
                stroke: None,
            },
        ));
    }
    let animation = frame.animation_frame.as_ref().unwrap_or(&image.frames[0]);
    let source = UiImageSource::bytes(
        format!(
            "image-preview-{:016x}-{}",
            image.identity, animation.sequence
        ),
        0,
        animation.bytes.clone(),
    );
    let request = crate::ui::image_rendering::image(source)
        .cache_policy(lgui::memory::ImageCachePolicy::WhileVisible)
        .namespace("image-preview");
    content = content.child(Element::new(move |_| {
        UiElement::requested_image(pixels_id, bounds, request, ImageFit::Fill)
    }));
    root = root.child(content);
    if let Some(error) = frame.error {
        let note = UiRect::new(rect.left, rect.top, rect.right, rect.top + 44.0);
        root = root.child(
            clip(rect, 0.0, 0.0).child(
                panel(note, VisualStyle::filled(theme::c().surface))
                    .semantics(Semantics::new(SemanticRole::Alert).name(error))
                    .child(text(
                        UiRect::new(
                            rect.left + 10.0,
                            note.top,
                            rect.right - 10.0,
                            note.top + 22.0,
                        ),
                        format!("{} preview could not update.", frame.format),
                        theme::sans(theme::c().warning, theme::UI_SIZE),
                    ))
                    .child(text(
                        UiRect::new(
                            rect.left + 10.0,
                            note.top + 22.0,
                            rect.right - 10.0,
                            note.bottom,
                        ),
                        "Showing the last valid preview.",
                        theme::sans(theme::c().text_muted, theme::SMALL),
                    )),
            ),
        );
    }
    root
}
fn control(
    rect: UiRect,
    key: &str,
    label: &str,
    icon: Option<&'static str>,
    send: Send,
    command: Command,
) -> Element {
    let mut element = Element::new(move |cx| {
        let hover = cx.context.interaction_flags(&cx.id).hovered;
        UiElement::panel(
            cx.id,
            rect,
            if hover {
                VisualStyle::filled(theme::c().surface)
            } else {
                VisualStyle::default()
            },
        )
        .children(cx.children)
    })
    .key(key)
    .event_policy(EventPolicy::INTERACTIVE)
    .cursor(CursorIcon::Pointer)
    .semantics(Semantics::new(SemanticRole::Button).name(label.to_owned()))
    .on_pointer_down_with_button(|cx, _, _| cx.stop_propagation())
    .on_click(move |cx: &mut UiEventContext| {
        cx.stop_propagation();
        send(command);
    });
    if let Some(icon) = icon {
        let left = (rect.left + rect.right - 14.0) / 2.0;
        let top = (rect.top + rect.bottom - 14.0) / 2.0;
        element = element.child(crate::ui::tabs::icon(
            UiId::owned(format!("{key}.icon")),
            icon,
            UiRect::new(left, top, left + 14.0, top + 14.0),
            theme::c().text_muted,
        ));
    } else {
        let mut style = theme::mono(theme::c().text_muted, theme::SMALL);
        style.align = TextAlign::Center;
        element = element.child(text(rect, label.to_owned(), style));
    }
    element
}
pub fn status_anchors(rect: UiRect, frame: &Frame) -> (UiRect, UiRect) {
    let format = frame.format;
    let format_w = crate::ui::tabs::measure(format, theme::SMALL, 400) + 6.0;
    let right = rect.right - 12.0 - format_w - 10.0;
    let zoom = UiRect::new((right - 92.0).max(rect.left), rect.top, right, rect.bottom);
    let background = UiRect::new(
        (zoom.left - 36.0).max(rect.left),
        rect.top,
        (zoom.left - 10.0).max(rect.left),
        rect.bottom,
    );
    (zoom, background)
}
pub fn status(rect: UiRect, frame: &Frame, send: Send) -> Element {
    let (zoom, background) = status_anchors(rect, frame);
    let mut bar = panel(rect, VisualStyle::filled(theme::c().sidebar));
    let (dimensions, bytes, format, ratio) = if let Some(image) = &frame.image {
        (
            format!("{} × {}", image.width, image.height),
            if frame.byte_len < 1024 {
                format!("{} B", frame.byte_len)
            } else {
                format!("{:.1} KB", frame.byte_len as f64 / 1024.0)
            },
            image.format,
            frame
                .position
                .scale_for(frame.bounds.width(), frame.bounds.height(), image),
        )
    } else {
        (String::new(), String::new(), frame.format, 1.0)
    };
    let label = format!(
        "{}{:.0}%",
        if frame.position.scale.is_none() {
            "Fit · "
        } else {
            ""
        },
        ratio * 100.0
    );
    bar = bar
        .child(
            control(
                zoom,
                "image.zoom",
                &label,
                None,
                send.clone(),
                Command::Menu(MenuKind::Zoom),
            )
            .child(crate::ui::tabs::icon(
                UiId::new("image.zoom.chevron"),
                "chevron-up",
                UiRect::new(
                    zoom.right - 14.0,
                    zoom.top + (zoom.height() - 10.0) / 2.0,
                    zoom.right - 4.0,
                    zoom.top + (zoom.height() + 10.0) / 2.0,
                ),
                theme::c().text_muted,
            )),
        )
        .child(control(
            background,
            "image.background",
            "Preview Background",
            Some("image-background"),
            send.clone(),
            Command::Menu(MenuKind::Background),
        ));
    let details_right = background.left - 10.0;
    let detail = if details_right - rect.left < 150.0 {
        dimensions
    } else {
        format!("{dimensions}   {bytes}")
    };
    let detail_w = crate::ui::tabs::measure(&detail, theme::SMALL, 400) + 6.0;
    if details_right - rect.left >= detail_w + 6.0 {
        bar = bar.child(text(
            UiRect::new(
                (details_right - detail_w).max(rect.left + 6.0),
                rect.top,
                details_right,
                rect.bottom,
            ),
            detail,
            theme::mono(theme::c().text_muted, theme::SMALL),
        ));
    }
    bar.child(text(
        UiRect::new(zoom.right + 10.0, rect.top, rect.right - 6.0, rect.bottom),
        format,
        theme::mono(theme::c().text_muted, theme::SMALL),
    ))
}
pub fn choices(kind: MenuKind) -> Vec<(&'static str, Command)> {
    match kind {
        MenuKind::Zoom => vec![
            ("Fit to Window", Command::Zoom(None)),
            ("25%", Command::Zoom(Some(0.25))),
            ("50%", Command::Zoom(Some(0.5))),
            ("100%", Command::Zoom(Some(1.0))),
            ("200%", Command::Zoom(Some(2.0))),
            ("400%", Command::Zoom(Some(4.0))),
        ],
        MenuKind::Background => vec![
            (
                "Checkerboard",
                Command::Background(Background::Checkerboard),
            ),
            ("Light", Command::Background(Background::Light)),
            ("Dark", Command::Background(Background::Dark)),
        ],
    }
}
pub fn menu(
    viewport: UiRect,
    anchor: UiRect,
    kind: MenuKind,
    selected: usize,
    hovered: Option<usize>,
    position: Position,
    send: Send,
) -> Element {
    let rows = choices(kind);
    let separator = if kind == MenuKind::Zoom && bottom_space(viewport, anchor) >= 57.0 {
        5.0
    } else {
        0.0
    };
    let width = 188.0_f32.min(viewport.width().max(0.0));
    let bottom = (anchor.top - 4.0).clamp(viewport.top, viewport.bottom);
    let count = (((bottom - viewport.top - 8.0 - separator).max(0.0) / 22.0).floor() as usize)
        .min(rows.len());
    let start = selected
        .saturating_sub(count.saturating_sub(1))
        .min(rows.len().saturating_sub(count));
    let separator = if start == 0 && count > 1 {
        separator
    } else {
        0.0
    };
    let height = 8.0 + count as f32 * 22.0 + separator;
    let left =
        (anchor.right - width).clamp(viewport.left, (viewport.right - width).max(viewport.left));
    let card = UiRect::new(
        left,
        (bottom - height).max(viewport.top),
        left + width,
        bottom,
    );
    let close = send.clone();
    let mut overlay = Element::new(move |cx| {
        UiElement::panel(UiId::new(FOCUS_ID), viewport, VisualStyle::default())
            .children(cx.children)
    })
    .event_policy(EventPolicy::INTERACTIVE)
    .on_pointer_down_with_button(move |_, pointer, _| {
        if !card.contains(pointer.point) {
            close(Command::CloseMenu);
        }
    });
    let mut contents = theme::bordered(card, theme::c().surface, theme::c().border, 6.0, 1.0)
        .shadow(
            ShadowStyle::new(Color::BLACK)
                .alpha(theme::c().shadow_alpha)
                .offset(0.0, 2.0)
                .blur(3.0),
        );
    for (slot, (i, (label, command))) in rows
        .into_iter()
        .enumerate()
        .skip(start)
        .take(count)
        .enumerate()
    {
        let top = card.top + 4.0 + slot as f32 * 22.0 + if slot > 0 { separator } else { 0.0 };
        if slot == 1 && separator > 0.0 {
            contents = contents.child(panel(
                UiRect::new(card.left + 10.0, top - 3.0, card.right - 10.0, top - 2.0),
                VisualStyle::filled(theme::c().border),
            ));
        }
        let row = UiRect::new(card.left + 4.0, top, card.right - 4.0, top + 22.0);
        let checked = match command {
            Command::Zoom(scale) => position.scale == scale,
            Command::Background(background) => position.background == background,
            _ => false,
        };
        let run = send.clone();
        let hover = send.clone();
        let mut item = panel(
            row,
            if hovered == Some(i) || (hovered.is_none() && selected == i) {
                VisualStyle::filled(theme::c().selection)
            } else {
                VisualStyle::default()
            },
        )
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .semantics(Semantics::new(SemanticRole::MenuItem).name(label))
        .on_event(UiEventKind::PointerMove, move |_, _| {
            hover(Command::Hover(Some(i)))
        })
        .on_click(move |cx: &mut UiEventContext| {
            cx.stop_propagation();
            run(Command::Choose(i));
        })
        .child(text(
            UiRect::new(row.left + 25.0, row.top, row.right - 6.0, row.bottom),
            label,
            theme::sans(theme::c().text_soft, theme::UI_SIZE),
        ));
        if checked {
            item = item.child(crate::ui::tabs::icon(
                UiId::new("image.menu.check"),
                "check",
                UiRect::new(
                    row.left + 5.0,
                    row.top + 5.0,
                    row.left + 17.0,
                    row.bottom - 5.0,
                ),
                theme::c().text_soft,
            ));
        }
        contents = contents.child(item);
    }
    overlay = overlay.child(contents);
    overlay
}
fn bottom_space(viewport: UiRect, anchor: UiRect) -> f32 {
    (anchor.top - 4.0).clamp(viewport.top, viewport.bottom) - viewport.top
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menus_stay_inside_small_windows_and_keep_the_selected_row_visible() {
        use lgui::application::AppView;
        use lgui::core::UiScale;
        use lgui::session::UiSession;
        for height in [80.0, 180.0, 600.0] {
            let viewport = UiRect::new(0.0, 0.0, 320.0, height);
            let anchor = UiRect::new(8.0, height - 26.0, 34.0, height);
            let view: AppView = Arc::new(move |_| {
                let position = Position {
                    scale: Some(4.0),
                    ..Default::default()
                };
                menu(
                    viewport,
                    anchor,
                    MenuKind::Zoom,
                    5,
                    None,
                    position,
                    Arc::new(|_| {}),
                )
            });
            let mut session = UiSession::new();
            session.render_view(&view, viewport, UiScale::ONE);
            let labels: Vec<_> = session
                .tree()
                .nodes()
                .iter()
                .filter_map(|node| node.text.as_deref())
                .collect();
            assert!(labels.contains(&"400%"));
            for node in session
                .tree()
                .nodes()
                .iter()
                .filter(|node| node.text.is_some())
            {
                let rect = node.layout_rect;
                assert!(rect.left >= viewport.left && rect.right <= viewport.right);
                assert!(rect.top >= viewport.top && rect.bottom <= anchor.top - 4.0);
            }
        }
    }
}
