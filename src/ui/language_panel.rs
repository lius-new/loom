//! Language resources and servers, presented from supplied state and commands.
use lgui::core::{CursorIcon, EventPolicy, UiEventContext};
use lgui::prelude::{Element, UiRect, VisualStyle, panel, text};
use std::sync::Arc;

use super::tabs::{ellipsize, measure};
use super::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Quiet,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub level: Level,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// The server and its state, drawn bold.
    pub title: String,
    pub level: Level,
    pub lines: Vec<Line>,
    /// The server whose pack this section can install, with the button's label.
    pub install: Option<(String, &'static str)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub sections: Vec<Section>,
    pub retry_downloads: bool,
}

pub enum Event {
    Close,
    Restart,
    RetryDownloads,
    /// Install the pack of the named server.
    Install(String),
}
pub type Commands = Arc<dyn Fn(Event) + Send + Sync>;

const WIDTH: f32 = 440.0;
const PAD: f32 = 12.0;
const LINE_H: f32 = 18.0;
const TITLE_H: f32 = 24.0;
const GAP: f32 = 10.0;
const FOOTER_H: f32 = 38.0;
const MARGIN: f32 = 8.0;
/// Approximate advance of the small monospace face.
const MONO_W: f32 = 6.4;
const EMPTY: &str = "No language downloads or servers are active.";

fn colour(level: Level) -> lgui::prelude::Color {
    let c = theme::c();
    match level {
        Level::Quiet => c.text_muted,
        Level::Warning => c.warning,
        Level::Error => c.error,
    }
}

/// The card's height for a frame: sections, then the footer.
fn height(frame: &Frame) -> f32 {
    let body: f32 = if frame.sections.is_empty() {
        LINE_H
    } else {
        frame
            .sections
            .iter()
            .map(|s| TITLE_H + s.lines.len() as f32 * LINE_H + GAP)
            .sum()
    };
    PAD + body + FOOTER_H
}

pub fn render(viewport: UiRect, frame: &Frame, commands: Commands) -> Element {
    let h = height(frame).min(viewport.bottom - viewport.top - theme::STATUS_H - 2.0 * MARGIN);
    let right = viewport.right - MARGIN;
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
    let text_w = WIDTH - 2.0 * PAD;
    let mut y = card.top + PAD;
    if frame.sections.is_empty() {
        body = body.child(text(
            UiRect::new(card.left + PAD, y, card.right - PAD, y + LINE_H),
            EMPTY,
            theme::sans(theme::c().text_muted, theme::SMALL),
        ));
    }
    for section in &frame.sections {
        let title = ellipsize(&section.title, text_w, theme::UI_SIZE, 600);
        body = body.child(text(
            UiRect::new(card.left + PAD, y, card.right - PAD, y + TITLE_H),
            title,
            theme::sans_semibold(colour_title(section.level), theme::UI_SIZE),
        ));
        if let Some((server, label)) = &section.install {
            let w = measure(label, theme::SMALL, 400) + 24.0;
            let button = UiRect::new(
                card.right - PAD - w,
                y + 2.0,
                card.right - PAD,
                y + TITLE_H - 2.0,
            );
            let (server, install) = (server.clone(), commands.clone());
            body = body.child(
                panel(
                    button,
                    VisualStyle::filled(theme::c().active_line).radius(4.0),
                )
                .event_policy(policy)
                .cursor(CursorIcon::Pointer)
                .on_click(move |cx: &mut UiEventContext| {
                    install(Event::Install(server.clone()));
                    cx.stop_propagation();
                })
                .child(text(
                    UiRect::new(button.left + 12.0, button.top, button.right, button.bottom),
                    *label,
                    theme::sans(theme::c().text_soft, theme::SMALL),
                )),
            );
        }
        y += TITLE_H;
        for line in &section.lines {
            let shown = clip(&line.text, (text_w / MONO_W) as usize);
            body = body.child(text(
                UiRect::new(card.left + PAD, y, card.right - PAD, y + LINE_H),
                shown,
                theme::mono(colour(line.level), theme::SMALL),
            ));
            y += LINE_H;
        }
        y += GAP;
    }
    // Footer: restart.
    let label = "Restart servers";
    let button_w = measure(label, theme::SMALL, 400) + 24.0;
    let button = UiRect::new(
        card.right - PAD - button_w,
        card.bottom - FOOTER_H + 6.0,
        card.right - PAD,
        card.bottom - 8.0,
    );
    let restart = commands.clone();
    body = body.child(
        panel(
            button,
            VisualStyle::filled(theme::c().active_line).radius(4.0),
        )
        .event_policy(policy)
        .cursor(CursorIcon::Pointer)
        .on_click(move |cx: &mut UiEventContext| {
            restart(Event::Restart);
            cx.stop_propagation();
        })
        .child(text(
            UiRect::new(button.left + 12.0, button.top, button.right, button.bottom),
            label,
            theme::sans(theme::c().text_soft, theme::SMALL),
        )),
    );
    if frame.retry_downloads {
        let label = "Retry downloads";
        let button = UiRect::new(
            card.left + PAD,
            button.top,
            card.left + PAD + measure(label, theme::SMALL, 400) + 24.0,
            button.bottom,
        );
        body = body.child(
            panel(
                button,
                VisualStyle::filled(theme::c().active_line).radius(4.0),
            )
            .event_policy(policy)
            .cursor(CursorIcon::Pointer)
            .on_click(move |cx: &mut UiEventContext| {
                commands(Event::RetryDownloads);
                cx.stop_propagation();
            })
            .child(text(
                UiRect::new(button.left + 12.0, button.top, button.right, button.bottom),
                label,
                theme::sans(theme::c().text_soft, theme::SMALL),
            )),
        );
    }
    overlay.child(body)
}

/// `value` cut to `max` characters, with an ellipsis when something was left out.
fn clip(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_owned();
    }
    let kept: String = value.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

fn colour_title(level: Level) -> lgui::prelude::Color {
    match level {
        Level::Quiet => theme::c().text_soft,
        other => colour(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_grows_with_its_lines() {
        let line = |t: &str| Line {
            text: t.into(),
            level: Level::Quiet,
        };
        let one = Frame {
            retry_downloads: false,
            sections: vec![Section {
                title: "a".into(),
                level: Level::Quiet,
                lines: vec![line("x")],
                install: None,
            }],
        };
        let two = Frame {
            retry_downloads: false,
            sections: vec![Section {
                title: "a".into(),
                level: Level::Quiet,
                lines: vec![line("x"), line("y")],
                install: None,
            }],
        };
        assert_eq!(height(&two) - height(&one), LINE_H);
        assert!(height(&Frame::default()) > FOOTER_H);
    }
}
