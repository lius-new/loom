//! What one language server provides, where it works and what it last said; opened
//! from the status bar's info icon, apart from the list of every server.
use lgui::core::{CursorIcon, EventPolicy, UiEventContext};
use lgui::prelude::{Color, Element, TextAlign, UiRect, VisualStyle, panel, text};
use std::sync::Arc;

use super::language_panel::{Level, Tone, tone_colour};
use super::tabs::{ellipsize_sans, measure_sans as measure};
use super::theme;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The instance the card describes, for its restart action.
    pub instance: u64,
    pub name: String,
    pub state: String,
    pub tone: Tone,
    /// Label and value, such as ("Provides", "hover, completion").
    pub rows: Vec<(String, String)>,
    /// Recent log lines, oldest first.
    pub log: Vec<(String, Level)>,
}

pub enum Event {
    Close,
    Restart(u64),
    CopyLog,
}
pub type Commands = Arc<dyn Fn(Event) + Send + Sync>;

const WIDTH: f32 = 420.0;
const PAD: f32 = 12.0;
const HEADER_H: f32 = 34.0;
const ROW_H: f32 = 20.0;
const LABEL_W: f32 = 84.0;
const SECTION_H: f32 = 26.0;
const LOG_H: f32 = 16.0;
const FOOTER_H: f32 = 36.0;
const MARGIN: f32 = 8.0;
const DOT: f32 = 7.0;
/// Approximate advance of the small monospace face.
const MONO_W: f32 = 6.4;
const NO_LOG: &str = "Nothing logged yet.";

fn height(frame: &Frame) -> f32 {
    let log = frame.log.len().max(1) as f32 * LOG_H;
    HEADER_H + frame.rows.len() as f32 * ROW_H + SECTION_H + log + 8.0 + FOOTER_H
}

/// The card opens above `anchor`, the status-bar icon which triggered it, with its
/// bottom-right corner at that icon.
pub fn render(
    viewport: UiRect,
    anchor: Option<UiRect>,
    frame: &Frame,
    commands: Commands,
) -> Element {
    let h = height(frame).min(viewport.height() - theme::STATUS_H - 2.0 * MARGIN);
    let right = anchor
        .map_or(viewport.right - MARGIN, |a| a.right)
        .min(viewport.right - MARGIN)
        .max(viewport.left + MARGIN + WIDTH);
    let bottom = viewport.bottom - theme::STATUS_H - MARGIN;
    let card = UiRect::new(right - WIDTH, bottom - h, right, bottom);
    let policy = EventPolicy {
        focus: false,
        ..EventPolicy::INTERACTIVE
    };
    let close = commands.clone();
    let overlay = panel(viewport, VisualStyle::default())
        .event_policy(policy)
        .on_pointer_down_with_button(move |cx, p, _| {
            if !card.contains(p.point) {
                close(Event::Close);
            }
            cx.stop_propagation();
        });
    let mut body =
        theme::bordered(card, theme::c().surface, theme::c().border, 6.0, 1.0).event_policy(policy);
    let left = card.left + PAD;
    let inner = card.right - PAD;
    let colour = tone_colour(frame.tone);

    // Header: the state's dot, the name, the state in words.
    let middle = card.top + HEADER_H / 2.0 + 2.0;
    body = body.child(
        panel(
            UiRect::new(left, middle - DOT / 2.0, left + DOT, middle + DOT / 2.0),
            VisualStyle::filled(colour).radius(DOT / 2.0),
        )
        .event_policy(EventPolicy::NONE),
    );
    let state_w = measure(&frame.state, theme::SMALL, 400) + 6.0;
    let name_room = (inner - state_w - 8.0 - (left + DOT + 8.0)).max(0.0);
    body = body.child(text(
        UiRect::new(
            left + DOT + 8.0,
            middle - 10.0,
            inner - state_w,
            middle + 10.0,
        ),
        ellipsize_sans(&frame.name, name_room, theme::UI_SIZE, 600),
        theme::sans_semibold(theme::c().text_bright, theme::UI_SIZE),
    ));
    body = body.child(aligned(
        UiRect::new(inner - state_w, middle - 9.0, inner, middle + 9.0),
        frame.state.clone(),
        theme::c().text_soft,
        TextAlign::Right,
    ));

    // Rows: a muted label and its value.
    let mut y = card.top + HEADER_H;
    for (label, value) in &frame.rows {
        body = body
            .child(text(
                UiRect::new(left, y, left + LABEL_W, y + ROW_H),
                label.clone(),
                theme::sans(theme::c().text_dim, theme::SMALL),
            ))
            .child(text(
                UiRect::new(left + LABEL_W, y, inner, y + ROW_H),
                ellipsize_sans(value, inner - left - LABEL_W, theme::SMALL, 400),
                theme::sans(theme::c().text_soft, theme::SMALL),
            ));
        y += ROW_H;
    }

    // The log: what the server said lately.
    body = body
        .child(
            panel(
                UiRect::new(card.left, y + 4.0, card.right, y + 4.5),
                VisualStyle::filled(theme::c().border),
            )
            .event_policy(EventPolicy::NONE),
        )
        .child(text(
            UiRect::new(left, y + 6.0, inner, y + SECTION_H),
            "Log",
            theme::sans(theme::c().text_faint, theme::SMALL),
        ));
    y += SECTION_H;
    if frame.log.is_empty() {
        body = body.child(text(
            UiRect::new(left, y, inner, y + LOG_H),
            NO_LOG,
            theme::sans(theme::c().text_muted, theme::SMALL),
        ));
    }
    let max_chars = ((inner - left) / MONO_W) as usize;
    for (line, level) in &frame.log {
        let ink = match level {
            Level::Quiet => theme::c().text_muted,
            Level::Warning => theme::c().warning,
            Level::Error => theme::c().error,
        };
        body = body.child(text(
            UiRect::new(left, y, inner, y + LOG_H),
            clip(line, max_chars),
            theme::mono(ink, theme::SMALL),
        ));
        y += LOG_H;
    }

    // Footer: the actions, as text links.
    let footer = UiRect::new(card.left, card.bottom - FOOTER_H, card.right, card.bottom);
    body = body.child(
        panel(
            UiRect::new(footer.left, footer.top, footer.right, footer.top + 0.5),
            VisualStyle::filled(theme::c().border),
        )
        .event_policy(EventPolicy::NONE),
    );
    let mut x = left;
    let instance = frame.instance;
    for (label, event) in [
        (
            "Restart",
            Box::new(move || Event::Restart(instance)) as Box<dyn Fn() -> Event + Send + Sync>,
        ),
        (
            "Copy log",
            Box::new(|| Event::CopyLog) as Box<dyn Fn() -> Event + Send + Sync>,
        ),
    ] {
        let width = measure(label, theme::SMALL, 400) + 8.0;
        let rect = UiRect::new(x, footer.top, x + width, footer.bottom);
        let commands = commands.clone();
        body = body.child(
            panel(rect, VisualStyle::default())
                .event_policy(policy)
                .cursor(CursorIcon::Pointer)
                .on_click(move |cx: &mut UiEventContext| {
                    commands(event());
                    cx.stop_propagation();
                })
                .child(aligned(
                    rect,
                    label.to_owned(),
                    theme::c().text_soft,
                    TextAlign::Left,
                )),
        );
        x += width + 16.0;
    }
    overlay.child(body)
}

fn aligned(rect: UiRect, label: String, colour: Color, align: TextAlign) -> Element {
    let mut style = theme::sans(colour, theme::SMALL);
    style.align = align;
    text(rect, label, style).into()
}

/// `value` cut to `max` characters, with an ellipsis when something was left out.
fn clip(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_owned();
    }
    let kept: String = value.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(rows: usize, log: usize) -> Frame {
        Frame {
            instance: 1,
            name: "a".into(),
            state: "Ready".into(),
            tone: Tone::Ready,
            rows: (0..rows).map(|i| (format!("l{i}"), "v".into())).collect(),
            log: (0..log).map(|_| ("x".into(), Level::Quiet)).collect(),
        }
    }

    #[test]
    fn the_card_grows_with_its_rows_and_log_lines() {
        assert_eq!(height(&frame(3, 2)) - height(&frame(2, 2)), ROW_H);
        assert_eq!(height(&frame(2, 3)) - height(&frame(2, 2)), LOG_H);
    }

    #[test]
    fn an_empty_log_still_reserves_a_line_for_its_message() {
        assert_eq!(height(&frame(2, 0)), height(&frame(2, 1)));
    }
}
