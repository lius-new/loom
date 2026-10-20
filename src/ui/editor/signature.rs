//! The signature help card: the signature of the call being typed, with the
//! parameter the caret is on in bold.
use crate::features::markdown::Block;
use crate::ui::{fonts, theme};
use lgui::prelude::{Element, UiRect, text};
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
    pub docs: Option<Arc<[Block]>>,
    /// Which signature of how many.
    pub position: Option<(usize, usize)>,
}

/// Draws the card above `anchor` (or below it when there is no room above).
pub(crate) fn render(view: &View, anchor: UiRect, area: UiRect) -> (UiRect, Element) {
    let size = theme::UI_SIZE;
    let char_w = fonts::editor().char_w * size / fonts::editor().size.max(1.0);
    let label_chars = view.label.chars().count();
    let max_width = MAX_W.min((area.width() - 8.0).max(80.0));
    let docs = view.docs.as_ref().map(|blocks| {
        crate::ui::markdown::layout(
            blocks,
            (max_width - 2.0 * PAD).max(40.0),
            &crate::ui::markdown::Metrics {
                measure: &|text, style| crate::ui::markdown::measure(text, style, size),
                line_h: ROW,
                gap: 6.0,
                indent: 16.0,
            },
        )
    });
    let label_w = label_chars as f32 * char_w;
    let width = (label_w.max(docs.as_ref().map_or(0.0, |l| l.width)) + 2.0 * PAD).min(max_width);
    let docs_h = docs.as_ref().map_or(0.0, |l| l.height + 6.0);
    let height = ROW + docs_h + 2.0 * PAD;
    let above = anchor.top - GAP - height;
    let top = if above >= area.top {
        above
    } else {
        (anchor.bottom + GAP).min(area.bottom - height)
    };
    let left = anchor
        .left
        .min(area.right - width - GAP)
        .max(area.left + GAP);
    let card = UiRect::new(left, top, left + width, top + height);
    let mut body = theme::bordered(card, theme::c().surface, theme::c().border, 4.0, 1.0)
        .key("language-signature");
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
        body = body.child(text(
            UiRect::new(
                x,
                card.top + PAD,
                x + (end - start) as f32 * char_w + 4.0,
                card.top + PAD + ROW,
            ),
            piece,
            style,
        ));
    }
    if let Some((index, total)) = view.position {
        let label = format!("{index}/{total}");
        body = body.child(text(
            UiRect::new(
                card.right - 40.0,
                card.top,
                card.right - 4.0,
                card.top + ROW,
            ),
            label,
            theme::sans(theme::c().text_faint, theme::SMALL),
        ));
    }
    if let Some(layout) = &docs {
        body = body.child(crate::ui::markdown::render(
            layout,
            card.left + PAD,
            card.top + PAD + ROW + 6.0,
            size,
            ROW,
        ));
    }
    (card, body)
}
