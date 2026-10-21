//! A single non-modal invitation for missing language servers.
use super::{
    tabs::{ellipsize, measure},
    theme,
};
use lgui::{
    core::{CursorIcon, EventPolicy, UiEventContext},
    prelude::{Element, UiRect, VisualStyle, panel, text},
};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub servers: Vec<String>,
    pub title: String,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Install,
    Later,
    Never,
}

/// No full-window hit surface: text editing outside this strip keeps working.
pub fn render(
    viewport: UiRect,
    frame: &Frame,
    commands: Arc<dyn Fn(Choice, Vec<String>) + Send + Sync>,
) -> Element {
    let width = (viewport.right - viewport.left - 16.0).clamp(0.0, 740.0);
    let left = viewport.left + (viewport.right - viewport.left - width) / 2.0;
    let narrow = width < 340.0;
    let card = UiRect::new(
        left,
        viewport.top + 8.0,
        left + width,
        viewport.top + if narrow { 166.0 } else { 108.0 },
    );
    let policy = EventPolicy {
        focus: false,
        ..EventPolicy::INTERACTIVE
    };
    let mut strip = theme::bordered(card, theme::c().surface, theme::c().border, 6.0, 1.0)
        .event_policy(policy)
        .on_pointer_down_with_button(|cx, _, _| cx.stop_propagation())
        .child(text(
            UiRect::new(
                card.left + 12.0,
                card.top + 8.0,
                card.right - 12.0,
                card.top + 30.0,
            ),
            ellipsize(&frame.title, (width - 24.0).max(0.0), theme::UI_SIZE, 600),
            theme::sans_semibold(theme::c().text, theme::UI_SIZE),
        ))
        .child(text(
            UiRect::new(
                card.left + 12.0,
                card.top + 32.0,
                card.right - 12.0,
                card.top + 52.0,
            ),
            ellipsize(&frame.detail, (width - 24.0).max(0.0), theme::SMALL, 400),
            theme::sans(theme::c().text_muted, theme::SMALL),
        ));
    let mut right = card.right - 12.0;
    for (index, (label, choice)) in [
        ("Don't ask again", Choice::Never),
        ("Not now", Choice::Later),
        ("Install", Choice::Install),
    ]
    .into_iter()
    .enumerate()
    {
        let button_width = measure(label, theme::SMALL, 400) + 20.0;
        let button = if narrow {
            let top = card.top + 62.0 + (2 - index) as f32 * 28.0;
            UiRect::new(card.left + 12.0, top, card.right - 12.0, top + 24.0)
        } else {
            UiRect::new(
                right - button_width,
                card.top + 62.0,
                right,
                card.bottom - 10.0,
            )
        };
        let send = commands.clone();
        let servers = frame.servers.clone();
        strip = strip.child(
            panel(
                button,
                VisualStyle::filled(theme::c().active_line).radius(4.0),
            )
            .event_policy(policy)
            .cursor(CursorIcon::Pointer)
            .on_click(move |cx: &mut UiEventContext| {
                send(choice, servers.clone());
                cx.stop_propagation();
            })
            .child(text(
                UiRect::new(
                    button.left + 10.0,
                    button.top,
                    button.right - 10.0,
                    button.bottom,
                ),
                label,
                theme::sans(theme::c().text_soft, theme::SMALL),
            )),
        );
        right -= button_width + 8.0;
    }
    strip
}
