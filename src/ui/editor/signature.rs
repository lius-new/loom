//! The signature help card: the signature of the call being typed, with the
//! parameter the caret is on in bold.
use super::docs;
use crate::ui::markdown::card::Document;
use crate::ui::{fonts, theme};
use lgui::core::{CursorIcon, EventPolicy, UiEventContext, clip};
use lgui::prelude::{Element, UiRect, VisualStyle, panel, scroll_view, selection_region, text};
use std::ops::Range;
use std::sync::Arc;

const PAD: f32 = 8.0;
const ROW: f32 = 18.0;
const MAX_W: f32 = 640.0;
const GAP: f32 = 4.0;

#[derive(Clone)]
pub(crate) struct View {
    pub label: String,
    /// The active parameter as character offsets into the label.
    pub active: Option<Range<usize>>,
    pub docs: Option<Document>,
    /// Which signature of how many.
    pub position: Option<(usize, usize)>,
    pub revision: u64,
}

/// Draws the card above `anchor` (or below it when there is no room above).
pub(crate) fn render(
    view: &View,
    anchor: UiRect,
    area: UiRect,
    change: Arc<dyn Fn(bool) + Send + Sync>,
    actions: docs::Actions,
) -> (UiRect, Element) {
    let size = theme::UI_SIZE;
    let char_w = fonts::editor().char_w * size / fonts::editor().size.max(1.0);
    let label_chars = view.label.chars().count();
    let max_width = MAX_W.min((area.width() - 2.0 * GAP).max(0.0));
    let docs = view
        .docs
        .as_ref()
        .map(|blocks| docs::layout(blocks, max_width - 2.0 * PAD));
    let label_w = label_chars as f32 * char_w;
    let navigation_w = if view.position.is_some() { 104.0 } else { 0.0 };
    let width = (label_w
        .max(navigation_w)
        .max(docs.as_ref().map_or(0.0, |l| l.width))
        + 2.0 * PAD)
        .min(max_width);
    let docs_h = docs.as_ref().map_or(0.0, |l| l.height + 6.0);
    let navigation_h = if view.position.is_some() {
        ROW + GAP
    } else {
        0.0
    };
    let height =
        (ROW + navigation_h + docs_h + 2.0 * PAD).min((area.height() - 2.0 * GAP).max(0.0));
    let above = anchor.top - GAP - height;
    let top = if above >= area.top {
        above
    } else {
        (anchor.bottom + GAP)
            .min(area.bottom - height - GAP)
            .max(area.top + GAP)
    };
    let left = anchor
        .left
        .min(area.right - width - GAP)
        .max(area.left + GAP);
    let card = UiRect::new(left, top, left + width, top + height);
    let policy = EventPolicy {
        focus: false,
        ..EventPolicy::INTERACTIVE
    };
    let mut body = theme::bordered(card, theme::c().surface, theme::c().border, 4.0, 1.0)
        .key("language-signature")
        .event_policy(policy)
        .on_pointer_down_with_button(|cx, _, _| cx.stop_propagation());
    let label_top = card.top + PAD + navigation_h;
    let mut label_pieces: Vec<Element> = Vec::new();
    let chars: Vec<char> = view.label.chars().collect();
    let (from, to) = view
        .active
        .as_ref()
        .map_or((chars.len(), chars.len()), |r| {
            (r.start.min(chars.len()), r.end.min(chars.len()))
        });
    let pieces = [(0, from, false), (from, to, true), (to, chars.len(), false)];
    for (start, end, active) in pieces {
        if start >= end {
            continue;
        }
        let piece: String = chars[start..end].iter().collect();
        let x = card.left + PAD + start as f32 * char_w;
        let style = if active {
            theme::mono_bold(theme::c().accent, size)
        } else {
            theme::mono(theme::c().text, size)
        };
        label_pieces.push(
            text(
                UiRect::new(
                    x,
                    label_top,
                    x + (end - start) as f32 * char_w + 4.0,
                    label_top + ROW,
                ),
                piece,
                style,
            )
            .into(),
        );
    }
    // A long signature scrolls sideways in its line rather than being cut off.
    let label_view = UiRect::new(
        card.left + PAD,
        label_top,
        (card.right - PAD).max(card.left + PAD),
        (label_top + ROW).min(card.bottom),
    );
    let label_line = scroll_view(label_view, (label_w, ROW))
        .horizontal()
        .thumb_color(theme::c().text_faint)
        .children(label_pieces);
    if let Some((index, total)) = view.position {
        let label = format!("{index}/{total}");
        let button_w = 24.0_f32.min((card.width() - 2.0 * PAD).max(0.0) / 3.0);
        for (forward, x, label, key) in [
            (false, card.left + PAD, "↑", "signature-previous"),
            (true, card.right - PAD - button_w, "↓", "signature-next"),
        ] {
            let button = UiRect::new(x, card.top + PAD, x + button_w, card.top + PAD + ROW);
            let change = change.clone();
            body = body.child(
                panel(
                    button,
                    VisualStyle::filled(theme::c().active_line).radius(3.0),
                )
                .key(key)
                .event_policy(policy)
                .cursor(CursorIcon::Pointer)
                .on_click(move |cx: &mut UiEventContext| {
                    change(forward);
                    cx.stop_propagation();
                })
                .child(text(
                    button,
                    label,
                    theme::sans(theme::c().text_soft, theme::SMALL),
                )),
            );
        }
        body = body.child(text(
            UiRect::new(
                card.left + PAD + button_w + GAP,
                card.top + PAD,
                card.right - PAD - button_w - GAP,
                card.top + PAD + ROW,
            ),
            label,
            theme::sans(theme::c().text_faint, theme::SMALL),
        ));
    }
    // The source label keeps its own selection; documentation is a controlled
    // reading surface with full logical selection and copy.
    let region_rect = UiRect::new(
        card.left + PAD,
        label_top.min(card.bottom),
        card.right - PAD,
        card.bottom - PAD,
    );
    let interaction = actions.interaction();
    let texts: Vec<Element> = vec![label_line.into()];
    let mut layers = clip(region_rect, 0.0, 0.0);
    let mut links = None;
    if let Some(layout) = &docs {
        let documentation = docs::render(
            layout,
            card.left + PAD,
            label_top + ROW + 6.0,
            Some(&interaction),
            (card.bottom - PAD - label_top - ROW - 6.0).max(1.0),
        );
        links = Some(documentation);
    }
    let on_select = interaction.on_select.clone();
    let mut region = selection_region(region_rect)
        .selection_color(theme::c().accent)
        .children(texts);
    if let Some(on_select) = on_select {
        region = region.on_select(move |_cx, text| on_select(text));
    }
    layers = layers.child(region);
    if let Some(links) = links {
        layers = layers.child(links);
    }
    body = body.child(layers);
    (card, body)
}
