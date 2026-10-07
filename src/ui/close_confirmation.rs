//! Modal confirmation shown before closing dirty editor tabs.

use lgui::core::{
    CursorIcon, EventPolicy, KeyState, LogicalKey, NamedKey, SemanticRole, Semantics, UiElement,
    UiEventContext, UiFocusHandle, UiId,
};
use lgui::prelude::{Element, State, TextAlign, UiRect, VisualStyle, panel, text};

use crate::state::{AppState, CloseContinuation};
use crate::theme;
use crate::workspace_actions;

const CARD_W: f32 = 520.0;
const CARD_H: f32 = 174.0;

pub fn render(
    viewport: UiRect,
    state: State<AppState>,
    dialog_id: UiId,
    editor_focus: UiFocusHandle,
) -> Element {
    let snapshot = state.get();
    let Some(request) = snapshot.close_request.as_ref() else {
        return panel(viewport, VisualStyle::default());
    };
    let dirty_targets = request
        .targets
        .iter()
        .copied()
        .filter(|id| snapshot.workspace.is_dirty(*id))
        .collect::<Vec<_>>();
    let dirty_count = dirty_targets.len();
    let target_name = dirty_targets
        .first()
        .and_then(|id| snapshot.workspace.meta(*id))
        .map(|meta| meta.name.clone())
        .unwrap_or_else(|| "this file".to_owned());
    let message = if dirty_count == 1 {
        format!("Do you want to save the changes you made to {target_name}?")
    } else {
        format!("Do you want to save changes to {dirty_count} files?")
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
    let key_state = state.clone();
    let key_focus = editor_focus.clone();
    let overlay_style = VisualStyle::filled(theme::c().bg).alpha(214);
    let mut root = Element::new(move |cx| {
        UiElement::panel(dialog_id.clone(), viewport, overlay_style).children(cx.children)
    })
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(Semantics::new(SemanticRole::Dialog).name("Save your changes?"))
    .on_key_down(move |cx, event| {
        if event.state != KeyState::Down {
            return;
        }
        let cmd = event.modifiers.ctrl() || event.modifiers.meta();
        let resolved = match &event.key {
            LogicalKey::Named(NamedKey::Escape) => {
                workspace_actions::cancel_close_request(&key_state);
                Some(None)
            }
            LogicalKey::Named(NamedKey::Enter) => {
                Some(workspace_actions::save_close_request(&key_state))
            }
            // Cmd/Ctrl+D is the platform shortcut for "Don't Save".
            LogicalKey::Character(value) if cmd && value.eq_ignore_ascii_case("d") => {
                Some(workspace_actions::discard_close_request(&key_state))
            }
            _ => None,
        };
        if let Some(continuation) = resolved {
            finish(cx, &key_state, &key_focus, continuation);
        }
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
                let continuation = workspace_actions::discard_close_request(&discard_state);
                finish(cx, &discard_state, &discard_focus, continuation);
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
                workspace_actions::cancel_close_request(&cancel_state);
                finish(cx, &cancel_state, &cancel_focus, None);
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
            let continuation = workspace_actions::save_close_request(&save_state);
            finish(cx, &save_state, &editor_focus, continuation);
        })
        .child(text(
            save_rect,
            "Save",
            centered(theme::sans_semibold(theme::c().text_bright, theme::UI_SIZE)),
        )),
    )
}

/// Apply the outcome of a dialog decision: exit when the request was an
/// application close, otherwise hand focus back to the editor once the
/// request is resolved. A failed save keeps the dialog (and its focus) open.
fn finish(
    cx: &mut UiEventContext,
    state: &State<AppState>,
    editor_focus: &UiFocusHandle,
    continuation: Option<CloseContinuation>,
) {
    if continuation == Some(CloseContinuation::ExitApplication) {
        cx.window().close();
    } else if state.get().close_request.is_none() {
        editor_focus.focus();
    }
}

fn centered(mut style: lgui::prelude::TextStyle) -> lgui::prelude::TextStyle {
    style.align = TextAlign::Center;
    style
}
