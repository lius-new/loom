//! Context menu shares the same commands and clipboard path as keyboard editing.
use super::{commands::Command, editor_view::execute_command};
use crate::input::action::Action;
use crate::key_actions;
use crate::{state::AppState, theme};
use lgui::core::UiFocusHandle;
use lgui::core::{CursorIcon, EventPolicy};
use lgui::prelude::{Element, State, UiRect, VisualStyle, panel, text};
use lgui::services::ServicesContextExt;

pub fn render(
    viewport: UiRect,
    editor_rect: UiRect,
    pos: (f32, f32),
    state: State<AppState>,
    focus: UiFocusHandle,
) -> Element {
    let items = [
        ("Undo", Command::Undo),
        ("Redo", Command::Redo),
        ("Cut", Command::Cut),
        ("Copy", Command::Copy),
        ("Paste", Command::Paste),
        ("Select All", Command::SelectAll),
    ];
    let s = state.get();
    let width = 230.0;
    let height = items.len() as f32 * 28.0 + 8.0;
    let x = pos
        .0
        .clamp(viewport.left, (viewport.right - width).max(viewport.left));
    let y = pos
        .1
        .clamp(viewport.top, (viewport.bottom - height).max(viewport.top));
    let card = UiRect::new(x, y, x + width, y + height);
    let close = state.clone();
    let close_focus = focus.clone();
    // Menu surfaces do not take focus away from the editor, so Escape still works.
    let policy = EventPolicy {
        focus: false,
        ..EventPolicy::INTERACTIVE
    };
    let overlay = panel(viewport, VisualStyle::default())
        .event_policy(policy)
        .on_pointer_down_with_button(move |cx, p, _| {
            if !card.contains(p.point) {
                close.update(|app| app.editor.menu = None);
                close_focus.focus();
            }
            cx.stop_propagation();
        });
    let mut menu =
        theme::bordered(card, theme::c().surface, theme::c().border, 5.0, 1.0).event_policy(policy);
    for (i, (label, command)) in items.into_iter().enumerate() {
        // Labels come from the keymap so they follow user rebindings.
        let shortcut = Action::for_editor_command(command)
            .and_then(|action| key_actions::editor_shortcut_label(&s, action))
            .unwrap_or_default();
        let enabled = s.workspace.active_editor().is_some_and(|b| match command {
            Command::Undo => b.can_undo(),
            Command::Redo => b.can_redo(),
            Command::Copy | Command::Cut => b.selection().is_some(),
            _ => true,
        });
        let r = UiRect::new(
            x + 4.0,
            y + 4.0 + i as f32 * 28.0,
            x + width - 4.0,
            y + 4.0 + (i + 1) as f32 * 28.0,
        );
        let color = if enabled {
            theme::c().text_soft
        } else {
            theme::c().text_dim
        };
        let st = state.clone();
        let hover = state.clone();
        let target = focus.clone();
        let row = panel(
            r,
            if enabled && s.editor.menu_hover == Some(i) {
                VisualStyle::filled(theme::c().active_line)
            } else {
                VisualStyle::default()
            },
        )
        .event_policy(policy)
        .cursor(if enabled {
            CursorIcon::Pointer
        } else {
            CursorIcon::Default
        })
        .on_pointer_move(move |_, _| {
            hover.try_update(|app| {
                let changed = app.editor.menu_hover != Some(i);
                app.editor.menu_hover = Some(i);
                changed
            });
        })
        .on_click(move |cx: &mut lgui::core::UiEventContext| {
            if enabled {
                execute_command(&st, command, editor_rect, cx.application().clipboard());
                st.update(|app| app.editor.menu = None);
                target.focus();
            }
            cx.stop_propagation();
        })
        .child(text(
            UiRect::new(r.left + 8.0, r.top, r.left + 100.0, r.bottom),
            label,
            theme::sans(color, theme::UI_SIZE),
        ))
        .child(text(
            UiRect::new(r.right - 110.0, r.top, r.right - 4.0, r.bottom),
            shortcut,
            theme::mono(color, theme::SMALL),
        ));
        menu = menu.child(row);
    }
    overlay.child(menu)
}
