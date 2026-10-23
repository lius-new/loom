//! A list that opens over the code: completion items, definitions, symbols, code
//! actions. It never takes focus from the editor; the keyboard reaches it
//! through the keymap, the mouse through its rows.
use super::docs;
use super::view::card_origin;
use crate::ui::markdown::card::Document;
use crate::ui::theme;
use lgui::core::{CursorIcon, EventPolicy, UiEventContext, WheelUnit, clip};
use lgui::prelude::{Element, UiRect, VisualStyle, panel, text};
use std::sync::Arc;

pub(crate) const ROW_H: f32 = 22.0;
const PAD: f32 = 4.0;
const BADGE_W: f32 = 22.0;
const MIN_W: f32 = 220.0;
const MAX_W: f32 = 640.0;
const INPUT_H: f32 = 26.0;
const DOCS_W: f32 = 320.0;
/// The widest the documentation grows to fit a long code line.
const DOCS_MAX_W: f32 = 560.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub label: String,
    /// Faint text after the label (a signature, a path, a source).
    pub detail: String,
    /// One or two characters naming the kind of thing.
    pub badge: &'static str,
    /// Character positions of the label to emphasise (the fuzzy match).
    pub matches: Vec<usize>,
    pub faint: bool,
}

#[derive(Clone)]
pub(crate) struct View {
    pub panel: bool,
    pub rows: Arc<[Row]>,
    pub selected: usize,
    /// The first row shown.
    pub scroll: usize,
    /// How many rows fit at most.
    pub visible: usize,
    /// The text typed into the popup's own field, when it has one.
    pub query: Option<String>,
    pub title: Option<String>,
    pub empty: String,
    /// The offset of the text it opens beside; at the top centre, like a command
    /// palette, when there is none.
    pub anchor: Option<usize>,
    /// Documentation of the selected row, shown beside the list.
    pub docs: Option<Document>,
}

pub(crate) enum Event {
    Focus,
    Hover(usize),
    Pick(usize),
    Dismiss,
    Scroll(isize),
    /// The text selected in the documentation, or `None` once cleared.
    Select(Option<String>),
    /// A link in the documentation was clicked.
    OpenLink(String),
}

pub(crate) type Sender = Arc<dyn Fn(Event) + Send + Sync>;

/// Width in the proportional font the popup draws with, at `weight`.
fn measure_weight(text: &str, weight: i32) -> f32 {
    crate::ui::tabs::measure_sans(text, theme::UI_SIZE, weight)
}

fn measure(text: &str) -> f32 {
    measure_weight(text, 400)
}

/// The width and height of the popup for `view`, before it is placed.
fn size(view: &View, area: UiRect) -> (f32, f32) {
    let shown = view.rows.len().min(view.visible).max(1);
    let widest = view
        .rows
        .iter()
        .map(|row| BADGE_W + measure(&row.label) + 12.0 + measure(&row.detail))
        .fold(0.0f32, f32::max);
    let width = (widest + 2.0 * PAD + 16.0)
        .clamp(MIN_W, MAX_W)
        .min((area.width() - 8.0).max(120.0));
    let header = if view.query.is_some() { INPUT_H } else { 0.0 }
        + if view.title.is_some() { ROW_H } else { 0.0 };
    (width, header + shown as f32 * ROW_H + 2.0 * PAD)
}

/// Draws the popup and returns where it is.
pub(crate) fn render(
    view: &View,
    anchor: Option<UiRect>,
    area: UiRect,
    send: Sender,
) -> (UiRect, Element) {
    let (mut width, mut height) = size(view, area);
    if view.panel {
        width = (area.width() - 8.0).max(0.0);
        height = height.min(
            (area.height() * 0.45)
                .max(90.0)
                .min((area.height() - 8.0).max(0.0)),
        );
    }
    let (left, top) = if view.panel {
        (area.left + 4.0, area.bottom - height - 4.0)
    } else {
        match anchor {
            Some(anchor) => card_origin(anchor, area, width, height),
            None => (
                area.left + ((area.width() - width) / 2.0).max(4.0),
                area.top + 24.0,
            ),
        }
    };
    let card = UiRect::new(left, top, left + width, top + height);
    let policy = EventPolicy {
        focus: false,
        ..EventPolicy::INTERACTIVE
    };
    let wheel = send.clone();
    let mut body = theme::bordered(card, theme::c().surface, theme::c().border, 5.0, 1.0)
        .key("language-popup")
        .event_policy(policy)
        .on_wheel(move |cx, delta| {
            let rows = match delta.unit {
                WheelUnit::Lines => delta.y.round() as isize,
                WheelUnit::Pixels => (delta.y / ROW_H).round() as isize,
            };
            if rows != 0 {
                wheel(Event::Scroll(-rows));
            }
            cx.stop_propagation();
        });
    let mut y = top + PAD;
    if view.panel {
        let r = UiRect::new(
            card.right - 26.0,
            card.top + 4.0,
            card.right - 4.0,
            card.top + 26.0,
        );
        let close = send.clone();
        body = body.child(
            panel(r, VisualStyle::default())
                .event_policy(policy)
                .cursor(CursorIcon::Pointer)
                .on_click(move |cx: &mut UiEventContext| {
                    cx.stop_propagation();
                    close(Event::Dismiss);
                })
                .child(crate::ui::tabs::close_glyph(
                    lgui::core::UiId::new("language-popup.close"),
                    r,
                    theme::c().text_soft,
                )),
        );
    }
    if let Some(title) = &view.title {
        body = body.child(text(
            UiRect::new(left + PAD + 4.0, y, left + width - PAD, y + ROW_H),
            title.clone(),
            theme::sans(theme::c().text_faint, theme::SMALL),
        ));
        y += ROW_H;
    }
    if let Some(query) = &view.query {
        let field = UiRect::new(left + PAD, y, left + width - PAD, y + INPUT_H - 4.0);
        body = body.child(
            panel(field, VisualStyle::filled(theme::c().bg).radius(3.0))
                .event_policy(policy)
                .on_pointer_down({
                    let focus = send.clone();
                    move |cx, _| {
                        cx.stop_propagation();
                        focus(Event::Focus);
                    }
                })
                .child(text(
                    UiRect::new(field.left + 6.0, field.top, field.right - 6.0, field.bottom),
                    format!("{query}\u{258F}"),
                    theme::sans(theme::c().text_bright, theme::UI_SIZE),
                )),
        );
        y += INPUT_H;
    }
    if view.rows.is_empty() {
        body = body.child(text(
            UiRect::new(left + PAD + 4.0, y, left + width - PAD, y + ROW_H),
            view.empty.clone(),
            theme::sans(theme::c().text_muted, theme::UI_SIZE),
        ));
    }
    let list = UiRect::new(left + PAD, y, left + width - PAD, card.bottom - PAD);
    let mut rows = clip(list, 0.0, 0.0);
    for (n, row) in view
        .rows
        .iter()
        .enumerate()
        .skip(view.scroll)
        .take(view.visible)
    {
        let r = UiRect::new(
            list.left,
            list.top + (n - view.scroll) as f32 * ROW_H,
            list.right,
            list.top + (n - view.scroll + 1) as f32 * ROW_H,
        );
        let hover = send.clone();
        let pick = send.clone();
        let style = if n == view.selected {
            VisualStyle::filled(theme::c().active_line).radius(3.0)
        } else {
            VisualStyle::default()
        };
        let colour = if row.faint {
            theme::c().text_faint
        } else {
            theme::c().text_bright
        };
        let mut item = panel(r, style)
            .event_policy(policy)
            .cursor(CursorIcon::Pointer)
            .on_pointer_move(move |_, _| hover(Event::Hover(n)))
            .on_click(move |cx: &mut UiEventContext| {
                pick(Event::Pick(n));
                cx.stop_propagation();
            });
        if !row.badge.is_empty() {
            item = item.child(text(
                UiRect::new(r.left + 4.0, r.top, r.left + BADGE_W, r.bottom),
                row.badge,
                theme::mono(theme::c().accent, theme::SMALL),
            ));
        }
        let label_left = r.left + BADGE_W;
        // The label in runs: the fuzzy-matched characters bold and brighter,
        // each run placed after the measured width of the one before it, so
        // nothing is drawn twice.
        let mut label_right = label_left;
        let mut run = String::new();
        let mut run_matched = false;
        let mut runs: Vec<(String, bool)> = Vec::new();
        for (at, ch) in row.label.chars().enumerate() {
            let matched = row.matches.contains(&at);
            if !run.is_empty() && matched != run_matched {
                runs.push((std::mem::take(&mut run), run_matched));
            }
            run_matched = matched;
            run.push(ch);
        }
        if !run.is_empty() {
            runs.push((run, run_matched));
        }
        for (piece, matched) in runs {
            let width = measure_weight(&piece, if matched { 600 } else { 400 });
            item = item.child(text(
                UiRect::new(label_right, r.top, label_right + width + 2.0, r.bottom),
                piece,
                if matched {
                    theme::sans_semibold(theme::c().accent, theme::UI_SIZE)
                } else {
                    theme::sans(colour, theme::UI_SIZE)
                },
            ));
            label_right += width;
        }
        if !row.detail.is_empty() {
            let x = label_right + 10.0;
            if x < r.right {
                item = item.child(text(
                    UiRect::new(x, r.top, r.right - 4.0, r.bottom),
                    row.detail.clone(),
                    theme::sans(theme::c().text_faint, theme::SMALL),
                ));
            }
        }
        rows = rows.child(item);
    }
    body = body.child(rows);
    if view.panel {
        body = body.on_pointer_down_with_button(|cx, _, _| cx.stop_propagation());
    }
    let mut out = if view.panel {
        body
    } else {
        panel(area, VisualStyle::default()).child(body)
    };
    let mut rect = card;
    if let Some(blocks) = &view.docs
        && !blocks.is_empty()
    {
        // Documentation beside the list, or below it when there is no room.
        // Wide enough for the widest code line, up to a limit; text wraps at it.
        let max_w = DOCS_MAX_W.min((area.width() - 8.0).max(DOCS_W));
        let (layout, text_w) = docs::fit(blocks, DOCS_W - 2.0 * docs::PAD, max_w - 2.0 * docs::PAD);
        let docs_w = text_w + 2.0 * docs::PAD;
        let doc_h = (layout.height + 2.0 * docs::PAD).min(area.height() * 0.6);
        let beside = card.right + 4.0 + docs_w <= area.right;
        let docs_left = if beside { card.right + 4.0 } else { card.left };
        let docs_top = if beside {
            card.top
        } else if card.bottom + 4.0 + doc_h <= area.bottom {
            card.bottom + 4.0
        } else {
            (card.top - 4.0 - doc_h).max(area.top)
        };
        let docs_rect = UiRect::new(docs_left, docs_top, docs_left + docs_w, docs_top + doc_h);
        let interaction = docs::Actions {
            open_link: {
                let send = send.clone();
                Arc::new(move |url| send(Event::OpenLink(url)))
            },
            select: {
                let send = send.clone();
                Arc::new(move |text| send(Event::Select(text)))
            },
        }
        .interaction();
        out = out.child(
            theme::bordered(docs_rect, theme::c().surface, theme::c().border, 5.0, 1.0)
                .key("language-popup-docs")
                .child(clip(docs_rect, 0.0, 0.0).child(docs::render(
                    &layout,
                    docs_rect.left + docs::PAD,
                    docs_rect.top + docs::PAD,
                    Some(&interaction),
                    (doc_h - 2.0 * docs::PAD).max(1.0),
                ))),
        );
        rect = UiRect::new(
            card.left.min(docs_rect.left),
            card.top.min(docs_rect.top),
            card.right.max(docs_rect.right),
            card.bottom.max(docs_rect.bottom),
        );
    }
    // A press outside the popup dismisses it.
    let dismiss = send;
    if !view.panel {
        out = out
            .event_policy(policy)
            .on_pointer_down_with_button(move |cx, p, _| {
                if !rect.contains(p.point) {
                    dismiss(Event::Dismiss);
                } else {
                    cx.stop_propagation();
                }
            });
    }
    (rect, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(n: usize) -> Arc<[Row]> {
        (0..n)
            .map(|i| Row {
                label: format!("item {i}"),
                detail: String::new(),
                badge: "f",
                matches: Vec::new(),
                faint: false,
            })
            .collect::<Vec<_>>()
            .into()
    }

    fn view(count: usize, visible: usize) -> View {
        View {
            panel: false,
            rows: rows(count),
            selected: 0,
            scroll: 0,
            visible,
            query: None,
            title: None,
            empty: "none".into(),
            anchor: None,
            docs: None,
        }
    }

    #[test]
    fn the_height_follows_the_rows_shown_not_all_rows() {
        let area = UiRect::new(0.0, 0.0, 800.0, 600.0);
        let (_, few) = size(&view(3, 8), area);
        let (_, many) = size(&view(300, 8), area);
        assert_eq!(few, 3.0 * ROW_H + 2.0 * PAD);
        assert_eq!(many, 8.0 * ROW_H + 2.0 * PAD);
        // An empty list still has room for its message.
        let (_, none) = size(&view(0, 8), area);
        assert_eq!(none, ROW_H + 2.0 * PAD);
    }

    #[test]
    fn a_field_and_a_title_add_a_header() {
        let area = UiRect::new(0.0, 0.0, 800.0, 600.0);
        let mut v = view(2, 8);
        let (_, plain) = size(&v, area);
        v.query = Some(String::new());
        v.title = Some("Symbols".into());
        let (_, headed) = size(&v, area);
        assert_eq!(headed - plain, INPUT_H + ROW_H);
    }

    #[test]
    fn the_width_is_bounded_by_the_area_and_the_maximum() {
        let mut v = view(1, 8);
        v.rows = vec![Row {
            label: "x".repeat(400),
            detail: String::new(),
            badge: "",
            matches: Vec::new(),
            faint: false,
        }]
        .into();
        let (wide, _) = size(&v, UiRect::new(0.0, 0.0, 2000.0, 600.0));
        assert_eq!(wide, MAX_W);
        let (narrow, _) = size(&v, UiRect::new(0.0, 0.0, 300.0, 600.0));
        assert_eq!(narrow, 292.0);
    }
}
