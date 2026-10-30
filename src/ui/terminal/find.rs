//! The find box floating over the top right of a terminal. The current match
//! is shown through the terminal's own selection, so nothing else is drawn.

use std::sync::Arc;

use lgui::core::{EventPolicy, SemanticRole, Semantics};
use lgui::prelude::{Color, Element, ShadowStyle, UiRect, component, text};

use super::bindings::{Binding, Command};
use super::header;
use super::state::{SearchBar, TerminalUiState};
use crate::ui::components::input::{
    self, AccessInputOptions, InputBinding, InputState, InputStyle,
};
use crate::ui::theme;

const MAX_W: f32 = 340.0;
const H: f32 = 30.0;
const BUTTON: f32 = 22.0;
const COUNT_W: f32 = 76.0;
/// Keeps the box clear of the terminal's overlay scrollbar.
const MARGIN_RIGHT: f32 = 14.0;
const MARGIN_TOP: f32 = 8.0;

/// The box's rectangle inside the terminal body.
pub(super) fn card(body: UiRect) -> UiRect {
    let width = MAX_W.min((body.width() - MARGIN_RIGHT * 2.0).max(0.0));
    let right = body.right - MARGIN_RIGHT;
    let top = body.top + MARGIN_TOP;
    UiRect::new(right - width, top, right, top + H)
}

/// What the count area says: nothing for an empty query, "No results", or
/// "current/total".
pub(super) fn count_label(bar: &SearchBar) -> String {
    if bar.input.text().is_empty() {
        String::new()
    } else if bar.status.total == 0 {
        "No results".to_owned()
    } else {
        format!("{}/{}", bar.status.current, bar.status.total)
    }
}

pub(super) fn render(
    body: UiRect,
    state: Binding,
    bar: &SearchBar,
    terminal_focus: lgui::core::UiFocusHandle,
) -> Element {
    let card = card(body);
    let query = bar.input.text().to_owned();
    // Keys and text typed here belong to the box, never to the shell beneath it.
    let mut surface = theme::bordered(card, theme::c().surface, theme::c().border, 6.0, 1.0)
        .shadow(
            ShadowStyle::new(Color::BLACK)
                .alpha(theme::c().shadow_alpha)
                .offset(0.0, 2.0)
                .blur(3.0),
        )
        .key("terminal-find")
        .event_policy(EventPolicy::INTERACTIVE)
        .on_input(|cx, _| cx.stop_propagation())
        .on_key_down(|cx, _| cx.stop_propagation());

    // Buttons from the right: close, next, previous.
    let mut right = card.right - 4.0;
    let mut slot = || {
        let top = card.top + (H - BUTTON) / 2.0;
        let rect = UiRect::new(right - BUTTON, top, right, top + BUTTON);
        right -= BUTTON + 2.0;
        rect
    };
    let (close_rect, next_rect, prev_rect) = (slot(), slot(), slot());
    let (close_state, next_state, prev_state) = (state.clone(), state.clone(), state.clone());
    let close_focus = terminal_focus.clone();
    let tone = theme::c().text_soft;
    surface = surface
        .child(header::button(
            "terminal-find-prev",
            prev_rect,
            "chevron-up",
            "Previous match",
            false,
            tone,
            move || {
                prev_state.run(Command::SearchStep(-1));
            },
        ))
        .child(header::button(
            "terminal-find-next",
            next_rect,
            "chevron-down",
            "Next match",
            false,
            tone,
            move || {
                next_state.run(Command::SearchStep(1));
            },
        ))
        .child(header::button(
            "terminal-find-close",
            close_rect,
            "close",
            "Close search",
            false,
            tone,
            move || {
                close_state.run(Command::CloseSearch);
                close_focus.focus();
            },
        ));
    let count_right = right + 2.0;
    let count_rect = UiRect::new(
        (count_right - COUNT_W).max(card.left),
        card.top,
        count_right,
        card.bottom,
    );
    let mut style = theme::sans(
        if bar.status.total == 0 && !query.is_empty() {
            theme::c().error_text
        } else {
            theme::c().text_muted
        },
        theme::SMALL + 1.0,
    );
    style.align = lgui::prelude::TextAlign::Right;
    surface = surface.child(text(count_rect, count_label(bar), style));

    let field = UiRect::new(
        card.left + 8.0,
        card.top + 3.0,
        (count_rect.left - 4.0).max(card.left + 8.0),
        card.bottom - 3.0,
    );
    let input_state = state.clone();
    let cancel_focus = terminal_focus;
    surface.child(
        component(field, move |cx, &rect| {
            let id = cx.use_stable_id();
            let focus = cx.focus_handle(id.clone());
            let mounted = focus.clone();
            cx.use_effect((), move || mounted.focus());
            // Every change of the text searches again.
            let searching = input_state.clone();
            cx.use_effect(query.clone(), move || {
                searching.run(Command::Search);
            });
            let read: fn(&TerminalUiState) -> &InputState = TerminalUiState::search_input;
            let write: fn(&mut TerminalUiState) -> &mut InputState =
                TerminalUiState::search_input_mut;
            let submit = input_state.clone();
            let cancel = input_state.clone();
            let cancel_focus = cancel_focus.clone();
            input::render_access(
                rect,
                id,
                focus,
                InputBinding::new_access(input_state.ui.clone(), read, write),
                AccessInputOptions {
                    label: "Find in terminal output",
                    placeholder: "Find",
                    style: InputStyle {
                        text: theme::mono(theme::c().text, theme::UI_SIZE),
                        placeholder: theme::mono(theme::c().text_faint, theme::UI_SIZE),
                        caret: theme::c().text,
                        selection: theme::c().selection,
                    },
                    on_submit: Some(Arc::new(move || {
                        submit.run(Command::SearchStep(1));
                    })),
                    on_cancel: Some(Arc::new(move || {
                        cancel.run(Command::CloseSearch);
                        cancel_focus.focus();
                    })),
                    on_blur: None,
                },
            )
        })
        .key("terminal-find-input")
        .semantics(Semantics::new(SemanticRole::TextInput).name("Find in terminal output")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::terminal::SearchStatus;

    fn bar(text: &str, total: usize, current: usize) -> SearchBar {
        SearchBar {
            target: 1,
            input: InputState::new(text),
            status: SearchStatus { total, current },
        }
    }

    #[test]
    fn the_count_says_nothing_for_an_empty_query_and_reports_misses() {
        assert_eq!(count_label(&bar("", 0, 0)), "");
        assert_eq!(count_label(&bar("x", 0, 0)), "No results");
        assert_eq!(count_label(&bar("x", 12, 3)), "3/12");
    }

    #[test]
    fn the_box_hangs_from_the_top_right_and_shrinks_in_a_narrow_terminal() {
        let wide = card(UiRect::new(0.0, 50.0, 800.0, 300.0));
        assert_eq!(wide.width(), MAX_W);
        assert_eq!(wide.right, 800.0 - MARGIN_RIGHT);
        assert_eq!(wide.top, 50.0 + MARGIN_TOP);
        let narrow = card(UiRect::new(0.0, 50.0, 200.0, 300.0));
        assert_eq!(narrow.width(), 200.0 - MARGIN_RIGHT * 2.0);
        assert!(narrow.left >= 0.0);
    }
}
