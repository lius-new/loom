//! Root component: owns `AppState`, computes layout regions, wires the global
//! keymap, and composes every UI module. No module talks to another directly —
//! all cross-cutting state lives in `AppState`.

use lgui::core::{KeyboardEvent, UiEventKind, UiEventPayload};
use lgui::prelude::{group, Element, RenderCx, UiRect};

use crate::editor::editor_view;
use crate::input::keymap;
use crate::state::AppState;
use crate::theme;
use crate::ui::{
    command_palette, context_menu, sidebar, statusbar, tabs, terminal, titlebar, toast,
};

pub fn app(cx: &mut RenderCx<'_, '_>) -> Element {
    let state = cx.state(AppState::new());
    let editor_id = cx.use_stable_id();
    let editor_focus = cx.focus_handle(editor_id.clone());
    let vp = cx.viewport();
    let w = vp.width();
    let h = vp.height();

    // Snapshot toggles for this frame.
    let s = state.get();
    let show_term = s.show_terminal;
    let show_drawer = s.show_drawer;
    let show_palette = s.show_palette;

    // ---- Region layout ------------------------------------------------
    let titlebar_rect = UiRect::new(0.0, 0.0, w, theme::TITLEBAR_H);
    let statusbar_rect = UiRect::new(0.0, h - theme::STATUS_H, w, h);
    let main_bottom = if show_term {
        h - theme::STATUS_H - theme::TERMINAL_H
    } else {
        h - theme::STATUS_H
    };
    let editor_left = 0.0;
    // The editor and its overlay scrollbars stop at the drawer's visible edge.
    // This keeps the vertical editor thumb reachable while the drawer is open.
    let tabs_right = if show_drawer { w - s.sidebar_w } else { w };
    let tabs_rect = UiRect::new(
        editor_left,
        theme::TITLEBAR_H,
        tabs_right,
        theme::TITLEBAR_H + theme::TABS_H,
    );
    let code_rect = UiRect::new(
        editor_left,
        theme::TITLEBAR_H + theme::TABS_H,
        tabs_right,
        main_bottom,
    );
    let sidebar_rect = UiRect::new(w - s.sidebar_w, theme::TITLEBAR_H, w, main_bottom);

    // ---- Root (global shortcut listener) ------------------------------
    let mut root = group(vp);
    let st = state.clone();
    root = root.on_key_down(move |_ctx, ev: &KeyboardEvent| {
        if let Some(action) = keymap::action_for(ev) {
            st.update(move |app| app.apply(action));
        }
    });

    // Track the pointer against the whole drawer rather than individual tree
    // rows. Capturing at the root also observes moves into sibling regions and
    // overlays, so the scrollbar thumb disappears as soon as the drawer is
    // left.
    let was_sidebar_hovered = s.sidebar_hovered;
    let st_hover = state.clone();
    root = root.on_event_capture(UiEventKind::PointerMove, move |_ctx, payload| {
        if let UiEventPayload::PointerMove { pointer } = payload {
            let hovered = show_drawer && sidebar_rect.contains(pointer.point);
            if hovered != was_sidebar_hovered {
                st_hover.update(move |app| app.sidebar_hovered = hovered);
            }
        }
    });

    let was_editor_hovered = s.editor_hovered;
    let st_editor_hover = state.clone();
    root = root.on_event_capture(UiEventKind::PointerMove, move |_ctx, payload| {
        if let UiEventPayload::PointerMove { pointer } = payload {
            let hovered = code_rect.contains(pointer.point);
            if hovered != was_editor_hovered {
                st_editor_hover.update(move |app| app.editor_hovered = hovered);
            }
        }
    });

    // ---- Compose chrome (back-to-front) -------------------------------
    root = root.child(titlebar::render(titlebar_rect, state.clone()));

    root = root.child(tabs::render(
        tabs_rect,
        state.clone(),
        editor_focus.clone(),
    ));
    root = root.child(editor_view::render(code_rect, state.clone(), editor_id));

    if show_drawer {
        root = root.child(sidebar::render(
            sidebar_rect,
            state.clone(),
            editor_focus.clone(),
        ));
    }

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
        root = root.child(command_palette::render(
            vp,
            state.clone(),
            editor_focus,
        ));
    }
    if let Some(pos) = s.context_menu {
        root = root.child(context_menu::render(vp, pos, state.clone()));
    }
    root = root.child(toast::render(vp, state.clone()));

    root
}
