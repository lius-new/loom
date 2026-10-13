//! Narrow reads and command callbacks for migrated chrome.
use lgui::prelude::State;

use super::state::AppState;
use crate::domain::notification::Toast;

pub(super) fn toast(state: &State<AppState>) -> Option<Toast> {
    // lgui State currently clones on get; the view receives only this projection.
    state.get().toast
}

pub(super) fn dismiss_toast(state: State<AppState>) -> impl Fn(u64) + Send + Sync + 'static {
    move |id| {
        state.try_update(|app| app.dismiss_toast(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgui::application::{AppView, ApplicationContext};
    use lgui::core::{
        InputEvent, Point, PointerButton, PointerData, UiScale, dispatch_runtime_output,
    };
    use lgui::prelude::{UiRect, VisualStyle, panel};
    use lgui::session::UiSession;
    use std::sync::{Arc, Mutex};

    #[test]
    fn toast_click_dismisses_only_the_message_that_was_rendered() {
        let exposed = Arc::new(Mutex::new(None::<State<AppState>>));
        let output = exposed.clone();
        let viewport = UiRect::new(0.0, 0.0, 600.0, 400.0);
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state_with(|| {
                let mut state = AppState::new();
                state.show_toast("ready");
                state
            });
            *output.lock().unwrap() = Some(state.clone());
            panel(viewport, VisualStyle::default()).child(crate::ui::toast::render(
                viewport,
                toast(&state),
                dismiss_toast(state),
            ))
        });
        let context = ApplicationContext::empty(Default::default());
        let _text = lgui::backend::install_text_environment(
            &context,
            lgui::render_skia::skia_text_system_handle(),
        );
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let state = exposed.lock().unwrap().clone().unwrap();
        let point = Point::new(300.0, viewport.bottom - crate::theme::STATUS_H - 30.0);
        let click = |session: &mut UiSession| {
            for event in [
                InputEvent::PointerDown {
                    pointer: PointerData::mouse(point),
                    button: PointerButton::Left,
                },
                InputEvent::PointerUp {
                    pointer: PointerData::mouse(point),
                    button: PointerButton::Left,
                },
            ] {
                let events = session.handle_input(event);
                dispatch_runtime_output(
                    events,
                    &context,
                    &lgui::window::WindowId::new("toast-test"),
                    |action| session.handle_default_action(action),
                    |_| {},
                );
            }
        };
        // Replace data without rendering: the old card must not clear the new toast.
        state.update(|state| state.show_error("replacement"));
        click(&mut session);
        assert_eq!(toast(&state).unwrap().message, "replacement");
        session.render_view(&view, viewport, UiScale::ONE);
        click(&mut session);
        assert!(toast(&state).is_none());
        // No toast still renders a valid empty view.
        session.render_view(&view, viewport, UiScale::ONE);
    }
}
