//! Modal repository URL prompt used by the Explorer empty state.

use lgui::core::{
    CursorIcon, EventPolicy, KeyState, LogicalKey, NamedKey, SemanticRole, Semantics, UiElement,
    UiFocusHandle, UiId,
};
use lgui::prelude::{Element, UiRect, VisualStyle, panel, text};

use crate::theme;
use crate::ui::git::commands::{Binding, Command};

pub(crate) const CARD_W: f32 = 480.0;
pub(crate) const CARD_H: f32 = 188.0;
pub(crate) const INPUT_CHAR_W: f32 = 7.2;

pub fn render(
    viewport: UiRect,
    state: Binding,
    input_focus: UiFocusHandle,
    input_id: UiId,
) -> Element {
    let snapshot = state.get();
    let cx = viewport.left + viewport.width() / 2.0;
    let top = viewport.top + (viewport.height() - CARD_H) * 0.34;
    let card = UiRect::new(cx - CARD_W / 2.0, top, cx + CARD_W / 2.0, top + CARD_H);
    let input_rect = UiRect::new(
        card.left + 20.0,
        card.top + 65.0,
        card.right - 20.0,
        card.top + 99.0,
    );

    let mut root = panel(viewport, VisualStyle::filled(theme::c().bg).alpha(214))
        .event_policy(EventPolicy::INTERACTIVE)
        .child(theme::bordered(
            card,
            theme::c().sidebar,
            theme::c().border,
            8.0,
            1.0,
        ));

    root = root.child(text(
        UiRect::new(
            card.left + 20.0,
            card.top + 17.0,
            card.right - 48.0,
            card.top + 40.0,
        ),
        "Clone Repository",
        theme::sans_semibold(theme::c().text_bright, 15.0),
    ));
    root = root.child(text(
        UiRect::new(
            card.left + 20.0,
            card.top + 40.0,
            card.right - 20.0,
            card.top + 58.0,
        ),
        "Enter a Git repository URL, then choose its destination folder.",
        theme::sans(theme::c().text_dim, theme::SMALL),
    ));

    let close_state = state.clone();
    root = root.child(
        panel(
            UiRect::new(
                card.right - 38.0,
                card.top + 8.0,
                card.right - 8.0,
                card.top + 38.0,
            ),
            VisualStyle::default().radius(4.0),
        )
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(if snapshot.model.cloning() {
            CursorIcon::NotAllowed
        } else {
            CursorIcon::Pointer
        })
        .on_click(move || {
            close_state.update(|app| {
                if !app.model.cloning() {
                    app.ui.clone_visible = false;
                    app.ui.clone_input_focused = false;
                    app.ui.clone_error = None;
                }
            });
        })
        .child(crate::ui::drawer::drawer_icon(
            "clone-dialog.close",
            "close",
            theme::icon_rect(UiRect::new(
                card.right - 29.0,
                card.top + 17.0,
                card.right - 17.0,
                card.top + 29.0,
            )),
            theme::c().text_dim,
        )),
    );

    let available_chars = ((input_rect.width() - 22.0) / INPUT_CHAR_W).floor() as usize;
    let display_value = visible_tail(&snapshot.ui.clone_url, available_chars);
    let display_len = display_value.chars().count();
    let placeholder = snapshot.ui.clone_url.is_empty();
    let label = if placeholder {
        "https://github.com/owner/repository.git".to_owned()
    } else {
        display_value
    };
    let input_style = if snapshot.ui.clone_error.is_some() {
        theme::c().error
    } else if snapshot.ui.clone_input_focused {
        theme::c().accent
    } else {
        theme::c().border
    };
    let mut semantics = Semantics::new(SemanticRole::TextInput)
        .name("Repository URL")
        .value(snapshot.ui.clone_url.clone());
    semantics.state.disabled = snapshot.model.cloning();
    semantics.state.invalid = snapshot.ui.clone_error.is_some();
    semantics.text.character_count = Some(snapshot.ui.clone_url.chars().count());

    let field_state = state.clone();
    let key_state = state.clone();
    let change_state = state.clone();
    let focus_state = state.clone();
    let blur_state = state.clone();
    let click_focus = input_focus.clone();
    let caret_x =
        (input_rect.left + 10.0 + display_len as f32 * INPUT_CHAR_W).min(input_rect.right - 10.0);
    let ime_rect = UiRect::new(
        caret_x,
        input_rect.top + 8.0,
        caret_x + 1.0,
        input_rect.bottom - 7.0,
    );
    let input_element = Element::new(move |cx| {
        UiElement::panel(
            input_id,
            input_rect,
            VisualStyle::filled(theme::c().bg).radius(4.0),
        )
        .ime_cursor_rect(ime_rect)
        .children(cx.children)
    })
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(semantics)
    .cursor(if snapshot.model.cloning() {
        CursorIcon::NotAllowed
    } else {
        CursorIcon::Text
    })
    .on_click(move || click_focus.focus())
    .on_input(move |cx, input| {
        if !input.is_empty() {
            let input = input.replace(['\r', '\n'], "");
            field_state.update(move |app| {
                if !app.model.cloning() {
                    app.ui.clone_url.push_str(&input);
                    app.ui.clone_error = None;
                }
            });
            cx.stop_propagation();
        }
    })
    .on_change(move |_cx, value| {
        let value = value.unwrap_or_default().replace(['\r', '\n'], "");
        change_state.update(move |app| {
            if !app.model.cloning() {
                app.ui.clone_url = value;
                app.ui.clone_error = None;
            }
        });
    })
    .on_key_down(move |cx, event| {
        if event.state != KeyState::Down {
            return;
        }
        // Enter and Esc arrive as `dialog::` actions (see `perform`).
        if event.key == LogicalKey::Named(NamedKey::Backspace) {
            key_state.update(|app| {
                if !app.model.cloning() {
                    app.ui.clone_url.pop();
                    app.ui.clone_error = None;
                }
            });
            cx.prevent_default();
            cx.stop_propagation();
        }
    })
    .on_focus(move |_cx| focus_state.update(|app| app.ui.clone_input_focused = true))
    .on_blur(move |_cx| blur_state.update(|app| app.ui.clone_input_focused = false))
    .child(panel(
        UiRect::new(
            input_rect.left,
            input_rect.top,
            input_rect.right,
            input_rect.top + 1.0,
        ),
        VisualStyle::filled(input_style),
    ))
    .child(panel(
        UiRect::new(
            input_rect.left,
            input_rect.bottom - 1.0,
            input_rect.right,
            input_rect.bottom,
        ),
        VisualStyle::filled(input_style),
    ))
    .child(panel(
        UiRect::new(
            input_rect.left,
            input_rect.top,
            input_rect.left + 1.0,
            input_rect.bottom,
        ),
        VisualStyle::filled(input_style),
    ))
    .child(panel(
        UiRect::new(
            input_rect.right - 1.0,
            input_rect.top,
            input_rect.right,
            input_rect.bottom,
        ),
        VisualStyle::filled(input_style),
    ))
    .child(text(
        UiRect::new(
            input_rect.left + 10.0,
            input_rect.top,
            input_rect.right - 10.0,
            input_rect.bottom,
        ),
        label,
        theme::mono(
            if placeholder {
                theme::c().text_faint
            } else {
                theme::c().text
            },
            theme::UI_SIZE,
        ),
    ));
    root = root.child(input_element);

    if snapshot.ui.clone_input_focused && !snapshot.model.cloning() && !placeholder {
        root = root.child(panel(ime_rect, VisualStyle::filled(theme::c().text)));
    }

    if let Some(error) = snapshot.ui.clone_error.as_ref() {
        root = root.child(text(
            UiRect::new(
                card.left + 20.0,
                card.top + 103.0,
                card.right - 20.0,
                card.top + 121.0,
            ),
            error.clone(),
            theme::sans(theme::c().error, theme::SMALL),
        ));
    }

    let cancel_rect = UiRect::new(
        card.right - 190.0,
        card.bottom - 43.0,
        card.right - 112.0,
        card.bottom - 13.0,
    );
    let clone_rect = UiRect::new(
        card.right - 104.0,
        card.bottom - 43.0,
        card.right - 20.0,
        card.bottom - 13.0,
    );
    let cancel_state = state.clone();
    root = root.child(
        theme::bordered(cancel_rect, theme::c().sidebar, theme::c().border, 4.0, 1.0)
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(if snapshot.model.cloning() {
                CursorIcon::NotAllowed
            } else {
                CursorIcon::Pointer
            })
            .on_click(move || {
                cancel_state.update(|app| {
                    if !app.model.cloning() {
                        app.ui.clone_visible = false;
                        app.ui.clone_input_focused = false;
                        app.ui.clone_error = None;
                    }
                });
            })
            .child(text(
                cancel_rect,
                "Cancel",
                centered(theme::sans(theme::c().text_soft, theme::UI_SIZE)),
            )),
    );

    let can_clone = !snapshot.model.cloning() && !snapshot.ui.clone_url.trim().is_empty();
    let action_state = state;
    root.child(
        panel(
            clone_rect,
            VisualStyle::filled(if can_clone {
                theme::c().accent
            } else {
                theme::c().text_ghost
            })
            .radius(4.0),
        )
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(if can_clone {
            CursorIcon::Pointer
        } else {
            CursorIcon::NotAllowed
        })
        .on_click(move || {
            if can_clone {
                action_state.dispatch(Command::CloneRepository);
            }
        })
        .child(text(
            clone_rect,
            if snapshot.model.cloning() {
                "Cloning..."
            } else {
                "Clone"
            },
            centered(theme::sans_semibold(theme::c().text_bright, theme::UI_SIZE)),
        )),
    )
}

pub(crate) fn centered(mut style: lgui::prelude::TextStyle) -> lgui::prelude::TextStyle {
    style.align = lgui::prelude::TextAlign::Center;
    style
}

pub(crate) fn visible_tail(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_owned();
    }
    let keep = max_chars.saturating_sub(1);
    format!("…{}", value.chars().skip(count - keep).collect::<String>())
}
