//! Modal confirmation shown before closing dirty editor tabs.

use lgui::core::{
    CursorIcon, EventPolicy, SemanticRole, Semantics, UiElement, UiEventContext, UiFocusHandle,
    UiId,
};
use lgui::prelude::{Element, TextAlign, UiRect, VisualStyle, panel, text};

use crate::ui::chrome::{CloseBinding, CloseCommand, CloseOutcome};
use crate::ui::theme;

const CARD_W: f32 = 520.0;
const CARD_H: f32 = 174.0;

pub fn render(
    viewport: UiRect,
    state: CloseBinding,
    dialog_id: UiId,
    editor_focus: UiFocusHandle,
) -> Element {
    let Some(message) = state.message() else {
        return panel(viewport, VisualStyle::default());
    };

    let center_x = viewport.left + viewport.width() / 2.0;
    let top = viewport.top + (viewport.height() - CARD_H) * 0.36;
    let card = UiRect::new(
        center_x - CARD_W / 2.0,
        top,
        center_x + CARD_W / 2.0,
        top + CARD_H,
    );

    // The overlay owns keyboard focus while it is open (see `app.rs`), so
    // Enter/Esc never reach the editor underneath. Every key stops here to keep
    // the dialog modal.
    let overlay_style = VisualStyle::filled(theme::c().bg).alpha(214);
    let mut root = Element::new(move |cx| {
        UiElement::panel(dialog_id.clone(), viewport, overlay_style).children(cx.children)
    })
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(Semantics::new(SemanticRole::Dialog).name("Save your changes?"))
    // Enter, Esc and Cmd/Ctrl+D arrive as `dialog::` actions (see `perform`);
    // the dialog swallows every other key.
    .on_key_down(move |cx, _event| {
        cx.prevent_default();
        cx.stop_propagation();
    })
    .child(theme::bordered(
        card,
        theme::c().sidebar,
        theme::c().border,
        8.0,
        1.0,
    ));

    root = root.child(text(
        UiRect::new(
            card.left + 22.0,
            card.top + 20.0,
            card.right - 22.0,
            card.top + 42.0,
        ),
        "Save your changes?",
        theme::sans_semibold(theme::c().text_bright, 15.0),
    ));
    root = root.child(text(
        UiRect::new(
            card.left + 22.0,
            card.top + 52.0,
            card.right - 22.0,
            card.top + 75.0,
        ),
        message,
        theme::sans(theme::c().text_muted, theme::UI_SIZE),
    ));
    root = root.child(text(
        UiRect::new(
            card.left + 22.0,
            card.top + 76.0,
            card.right - 22.0,
            card.top + 96.0,
        ),
        "Your changes will be lost if you don't save them.",
        theme::sans(theme::c().text_dim, theme::SMALL),
    ));

    let button_top = card.bottom - 46.0;
    let discard_rect = UiRect::new(
        card.left + 22.0,
        button_top,
        card.left + 122.0,
        button_top + 30.0,
    );
    let cancel_rect = UiRect::new(
        card.right - 190.0,
        button_top,
        card.right - 112.0,
        button_top + 30.0,
    );
    let save_rect = UiRect::new(
        card.right - 104.0,
        button_top,
        card.right - 22.0,
        button_top + 30.0,
    );

    let discard_state = state.clone();
    let discard_focus = editor_focus.clone();
    root = root.child(
        theme::bordered(discard_rect, theme::c().sidebar, theme::c().error, 4.0, 1.0)
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(CursorIcon::Pointer)
            .on_click(move |cx: &mut UiEventContext| {
                let outcome = discard_state.run(CloseCommand::Discard);
                finish(cx, &discard_focus, outcome);
            })
            .child(text(
                discard_rect,
                "Don't Save",
                centered(theme::sans(theme::c().error_text, theme::UI_SIZE)),
            )),
    );

    let cancel_state = state.clone();
    let cancel_focus = editor_focus.clone();
    root = root.child(
        theme::bordered(cancel_rect, theme::c().sidebar, theme::c().border, 4.0, 1.0)
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(CursorIcon::Pointer)
            .on_click(move |cx: &mut UiEventContext| {
                let outcome = cancel_state.run(CloseCommand::Cancel);
                finish(cx, &cancel_focus, outcome);
            })
            .child(text(
                cancel_rect,
                "Cancel",
                centered(theme::sans(theme::c().text_soft, theme::UI_SIZE)),
            )),
    );

    let save_state = state;
    root.child(
        panel(
            save_rect,
            VisualStyle::filled(theme::c().accent).radius(4.0),
        )
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_click(move |cx: &mut UiEventContext| {
            let outcome = save_state.run(CloseCommand::Save);
            finish(cx, &editor_focus, outcome);
        })
        .child(text(
            save_rect,
            "Save",
            centered(theme::sans_semibold(theme::c().on_accent, theme::UI_SIZE)),
        )),
    )
}

fn finish(cx: &mut UiEventContext, editor_focus: &UiFocusHandle, outcome: CloseOutcome) {
    if outcome.exit {
        cx.window().close();
    } else if outcome.resolved {
        editor_focus.focus();
    }
}

fn centered(mut style: lgui::prelude::TextStyle) -> lgui::prelude::TextStyle {
    style.align = TextAlign::Center;
    style
}
