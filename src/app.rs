//! Root component: owns `AppState`, computes layout regions, wires the global
//! keymap, and composes every UI module. No module talks to another directly —
//! all cross-cutting state lives in `AppState`.

use lgui::core::KeyboardEvent;
use lgui::prelude::{group, Element, RenderCx, UiRect};

use crate::editor::editor_view;
use crate::input::keymap;
use crate::state::AppState;
use crate::theme;
use crate::ui::{
    assistant, cmdk, command_palette, sidebar, statusbar, tabs, terminal, titlebar, toast,
};

pub fn app(cx: &mut RenderCx<'_, '_>) -> Element {
    let state = cx.state(AppState::new());
    let vp = cx.viewport();
    let w = vp.width();
    let h = vp.height();

    // Snapshot toggles for this frame.
    let s = state.get();
    let show_term = s.show_terminal;
    let show_drawer = s.show_drawer;
    let show_asst = s.show_assistant;
    let show_palette = s.show_palette;
    let show_cmdk = s.show_cmdk;

    // ---- Region layout ------------------------------------------------
    let titlebar_rect = UiRect::new(0.0, 0.0, w, theme::TITLEBAR_H);
    let statusbar_rect = UiRect::new(0.0, h - theme::STATUS_H, w, h);
    let main_bottom = if show_term {
        h - theme::STATUS_H - theme::TERMINAL_H
    } else {
        h - theme::STATUS_H
    };
    let editor_left = if show_drawer { theme::SIDEBAR_W } else { 0.0 };
    let editor_right = if show_asst {
        w - theme::ASSISTANT_W
    } else {
        w
    };
    let tabs_rect = UiRect::new(
        editor_left,
        theme::TITLEBAR_H,
        editor_right,
        theme::TITLEBAR_H + theme::TABS_H,
    );
    let code_rect = UiRect::new(
        editor_left,
        theme::TITLEBAR_H + theme::TABS_H,
        editor_right,
        main_bottom,
    );

    // ---- Root (global shortcut listener) ------------------------------
    let mut root = group(vp);
    let st = state.clone();
    root = root.on_key_down(move |_ctx, ev: &KeyboardEvent| {
        if let Some(action) = keymap::action_for(ev) {
            st.update(move |app| app.apply(action));
        }
    });

    // ---- Compose chrome (back-to-front) -------------------------------
    root = root.child(titlebar::render(titlebar_rect, state.clone()));

    if show_drawer {
        root = root.child(sidebar::render(
            UiRect::new(0.0, theme::TITLEBAR_H, theme::SIDEBAR_W, main_bottom),
            state.clone(),
        ));
    }
    if show_asst {
        root = root.child(assistant::render(UiRect::new(
            w - theme::ASSISTANT_W,
            theme::TITLEBAR_H,
            w,
            main_bottom,
        )));
    }

    root = root.child(tabs::render(tabs_rect, state.clone()));
    root = root.child(editor_view::render(code_rect, state.clone()));

    if show_term {
        root = root.child(terminal::render(UiRect::new(
            0.0,
            h - theme::STATUS_H - theme::TERMINAL_H,
            w,
            h - theme::STATUS_H,
        )));
    }

    root = root.child(statusbar::render(statusbar_rect, state.clone()));

    // ---- Overlays ------------------------------------------------------
    if show_palette {
        root = root.child(command_palette::render(vp, state.clone()));
    }
    if show_cmdk {
        root = root.child(cmdk::render(vp, state.clone()));
    }
    root = root.child(toast::render(vp, state.clone()));

    root
}
