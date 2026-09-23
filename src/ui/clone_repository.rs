//! Modal repository URL prompt used by the Explorer empty state.

use lgui::core::{
    CursorIcon, EventPolicy, KeyState, LogicalKey, NamedKey, SemanticRole, Semantics, UiElement,
    UiFocusHandle, UiId,
};
use lgui::prelude::{Element, State, UiRect, VisualStyle, panel, text};

use crate::state::AppState;
use crate::theme;
use crate::workspace_actions;

const CARD_W: f32 = 480.0;
const CARD_H: f32 = 188.0;
const INPUT_CHAR_W: f32 = 7.2;

pub fn render(
    viewport: UiRect,
    state: State<AppState>,
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

    let mut root = panel(viewport, VisualStyle::filled(theme::BG).alpha(214))
        .event_policy(EventPolicy::INTERACTIVE)
        .child(theme::bordered(
            card,
            theme::SIDEBAR,
            theme::BORDER,
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
        theme::sans_semibold(theme::ZINC_100, 15.0),
    ));
    root = root.child(text(
        UiRect::new(
            card.left + 20.0,
            card.top + 40.0,
            card.right - 20.0,
            card.top + 58.0,
        ),
        "Enter a Git repository URL, then choose its destination folder.",
        theme::sans(theme::ZINC_500, theme::SMALL),
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
        .cursor(if snapshot.cloning_repository {
            CursorIcon::NotAllowed
        } else {
            CursorIcon::Pointer
        })
        .on_click(move || {
            close_state.update(|app| {
                if !app.cloning_repository {
                    app.show_clone_dialog = false;
                    app.clone_input_focused = false;
                    app.clone_repository_error = None;
                }
            });
        })
        .child(super::sidebar::drawer_icon(
            "clone-dialog.close",
            "close",
            UiRect::new(
                card.right - 29.0,
                card.top + 17.0,
                card.right - 17.0,
                card.top + 29.0,
            ),
            theme::ZINC_500,
        )),
    );

    let available_chars = ((input_rect.width() - 22.0) / INPUT_CHAR_W).floor() as usize;
    let display_value = visible_tail(&snapshot.clone_repository_url, available_chars);
    let display_len = display_value.chars().count();
    let placeholder = snapshot.clone_repository_url.is_empty();
    let label = if placeholder {
        "https://github.com/owner/repository.git".to_owned()
    } else {
        display_value
    };
    let input_style = if snapshot.clone_repository_error.is_some() {
        theme::DIFF_DEL_BORDER
    } else if snapshot.clone_input_focused {
        theme::ACCENT
    } else {
        theme::BORDER
    };
    let mut semantics = Semantics::new(SemanticRole::TextInput)
        .name("Repository URL")
        .value(snapshot.clone_repository_url.clone());
    semantics.state.disabled = snapshot.cloning_repository;
    semantics.state.invalid = snapshot.clone_repository_error.is_some();
    semantics.text.character_count = Some(snapshot.clone_repository_url.chars().count());

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
            VisualStyle::filled(theme::BG).radius(4.0),
        )
        .ime_cursor_rect(ime_rect)
        .children(cx.children)
    })
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(semantics)
    .cursor(if snapshot.cloning_repository {
        CursorIcon::NotAllowed
    } else {
        CursorIcon::Text
    })
    .on_click(move || click_focus.focus())
    .on_input(move |cx, input| {
        if !input.is_empty() {
            let input = input.replace(['\r', '\n'], "");
            field_state.update(move |app| {
                if !app.cloning_repository {
                    app.clone_repository_url.push_str(&input);
                    app.clone_repository_error = None;
                }
            });
            cx.stop_propagation();
        }
    })
    .on_change(move |_cx, value| {
        let value = value.unwrap_or_default().replace(['\r', '\n'], "");
        change_state.update(move |app| {
            if !app.cloning_repository {
                app.clone_repository_url = value;
                app.clone_repository_error = None;
            }
        });
    })
    .on_key_down(move |cx, event| {
        if event.state != KeyState::Down {
            return;
        }
        match &event.key {
            LogicalKey::Named(NamedKey::Backspace) => {
                key_state.update(|app| {
                    if !app.cloning_repository {
                        app.clone_repository_url.pop();
                        app.clone_repository_error = None;
                    }
                });
                cx.prevent_default();
                cx.stop_propagation();
            }
            LogicalKey::Named(NamedKey::Enter) => {
                workspace_actions::clone_repository(&key_state);
                cx.prevent_default();
                cx.stop_propagation();
            }
            LogicalKey::Named(NamedKey::Escape) => {
                key_state.update(|app| {
                    if !app.cloning_repository {
                        app.show_clone_dialog = false;
                        app.clone_input_focused = false;
                        app.clone_repository_error = None;
                    }
                });
                cx.prevent_default();
                cx.stop_propagation();
            }
            _ => {}
        }
    })
    .on_focus(move |_cx| focus_state.update(|app| app.clone_input_focused = true))
    .on_blur(move |_cx| blur_state.update(|app| app.clone_input_focused = false))
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
                theme::ZINC_600
            } else {
                theme::ZINC_200
            },
            theme::UI_SIZE,
        ),
    ));
    root = root.child(input_element);

    if snapshot.clone_input_focused && !snapshot.cloning_repository && !placeholder {
        root = root.child(panel(ime_rect, VisualStyle::filled(theme::ZINC_200)));
    }

    if let Some(error) = snapshot.clone_repository_error.as_ref() {
        root = root.child(text(
            UiRect::new(
                card.left + 20.0,
                card.top + 103.0,
                card.right - 20.0,
                card.top + 121.0,
            ),
            error.clone(),
            theme::sans(theme::DIFF_DEL_BORDER, theme::SMALL),
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
        theme::bordered(cancel_rect, theme::SIDEBAR, theme::BORDER, 4.0, 1.0)
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(if snapshot.cloning_repository {
                CursorIcon::NotAllowed
            } else {
                CursorIcon::Pointer
            })
            .on_click(move || {
                cancel_state.update(|app| {
                    if !app.cloning_repository {
                        app.show_clone_dialog = false;
                        app.clone_input_focused = false;
                        app.clone_repository_error = None;
                    }
                });
            })
            .child(text(
                cancel_rect,
                "Cancel",
                centered(theme::sans(theme::ZINC_300, theme::UI_SIZE)),
            )),
    );

    let can_clone =
        !snapshot.cloning_repository && !snapshot.clone_repository_url.trim().is_empty();
    let action_state = state;
    root.child(
        panel(
            clone_rect,
            VisualStyle::filled(if can_clone {
                theme::ACCENT
            } else {
                theme::ZINC_700
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
                workspace_actions::clone_repository(&action_state);
            }
        })
        .child(text(
            clone_rect,
            if snapshot.cloning_repository {
                "Cloning..."
            } else {
                "Clone"
            },
            centered(theme::sans_semibold(theme::ZINC_100, theme::UI_SIZE)),
        )),
    )
}

fn centered(mut style: lgui::prelude::TextStyle) -> lgui::prelude::TextStyle {
    style.align = lgui::prelude::TextAlign::Center;
    style
}

fn visible_tail(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_owned();
    }
    let keep = max_chars.saturating_sub(1);
    format!("…{}", value.chars().skip(count - keep).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_repository_url_keeps_its_editing_tail_visible() {
        assert_eq!(visible_tail("abcdefgh", 5), "…efgh");
        assert_eq!(visible_tail("repo", 8), "repo");
    }
}
