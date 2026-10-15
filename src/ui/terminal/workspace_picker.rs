//! Workspace choice for a new terminal without an active project file.
use super::bindings::{Binding, Command};
use crate::ui::{tabs::ellipsize, theme};
use lgui::core::{
    CursorIcon, EventPolicy, LogicalKey, NamedKey, SemanticRole, Semantics, UiElement,
    UiFocusHandle,
};
use lgui::prelude::{Element, UiRect, VisualStyle, text};

const ROW: f32 = 30.0;
const HEADER: f32 = 64.0;

pub(super) fn render(viewport: UiRect, state: Binding, focus: UiFocusHandle) -> Element {
    let frame = state.get();
    let Some(picker) = frame.ui.terminal_workspace_picker else {
        return lgui::prelude::group(viewport);
    };
    let roots = frame.workspace_roots;
    let visible = (((viewport.height() - HEADER - 32.0) / ROW).floor() as usize).clamp(1, 8);
    let max_scroll = roots.len().saturating_sub(visible);
    let scroll = picker.scroll.min(max_scroll);
    let height = HEADER + ROW * roots.len().min(visible) as f32 + 8.0;
    let width = 520.0_f32.min((viewport.width() - 24.0).max(0.0));
    let left = viewport.left + (viewport.width() - width) * 0.5;
    let top = viewport.top + ((viewport.height() - height) * 0.5).max(0.0);
    let card = UiRect::new(left, top, left + width, top + height);
    let dismiss = state.clone();
    let keys = state.clone();
    let keyboard_focus = focus.clone();
    let mut overlay = Element::new(move |cx| {
        UiElement::panel(cx.id, viewport, VisualStyle::default())
            .auto_focus()
            .children(cx.children)
    })
    .key("terminal-workspace-picker")
    .event_policy(EventPolicy::INTERACTIVE)
    .focus_scope()
    .on_pointer_down_with_button(move |cx, pointer, _| {
        if !card.contains(pointer.point) {
            dismiss.ui.update(|ui| ui.terminal_workspace_picker = None);
        }
        cx.stop_propagation();
    })
    .on_input(|cx, _| cx.stop_propagation())
    .on_key_down(move |cx, event| {
        match event.key {
            LogicalKey::Named(NamedKey::Escape) => {
                keys.ui.update(|ui| ui.terminal_workspace_picker = None)
            }
            LogicalKey::Named(NamedKey::ArrowUp | NamedKey::ArrowDown) => {
                let count = keys.get().workspace_roots.len();
                keys.ui.update(|ui| {
                    if let Some(picker) = &mut ui.terminal_workspace_picker {
                        picker.selected = if event.key == LogicalKey::Named(NamedKey::ArrowUp) {
                            picker.selected.saturating_sub(1)
                        } else {
                            picker
                                .selected
                                .saturating_add(1)
                                .min(count.saturating_sub(1))
                        };
                        picker.scroll = picker
                            .scroll
                            .min(picker.selected)
                            .max(picker.selected.saturating_sub(visible - 1));
                    }
                });
            }
            LogicalKey::Named(NamedKey::Enter) => {
                let frame = keys.get();
                if let Some(root) = frame
                    .ui
                    .terminal_workspace_picker
                    .as_ref()
                    .and_then(|picker| frame.workspace_roots.get(picker.selected))
                    && keys.run(Command::ChooseWorkspace(root.clone())).applied
                {
                    keyboard_focus.focus();
                }
            }
            _ => {}
        }
        cx.prevent_default();
        cx.stop_propagation();
    });
    let wheel = state.clone();
    let mut surface = theme::bordered(card, theme::c().surface, theme::c().border, 6.0, 1.0)
        .event_policy(EventPolicy::INTERACTIVE)
        .on_wheel(move |cx, delta| {
            wheel.ui.update(|ui| {
                if let Some(picker) = &mut ui.terminal_workspace_picker {
                    picker.scroll = if delta.y > 0.0 {
                        scroll.saturating_sub(1)
                    } else {
                        scroll.saturating_add(1).min(max_scroll)
                    };
                }
            });
            cx.stop_propagation();
        })
        .child(text(
            UiRect::new(left + 14.0, top + 8.0, card.right - 14.0, top + 34.0),
            "Choose terminal workspace",
            theme::mono(theme::c().text, theme::UI_SIZE),
        ))
        .child(text(
            UiRect::new(
                left + 14.0,
                top + 34.0,
                card.right - 14.0,
                top + HEADER - 4.0,
            ),
            "Remember this choice when no project file is active.",
            theme::mono(theme::c().text_muted, theme::SMALL),
        ));
    for (index, root) in roots.iter().enumerate().skip(scroll).take(visible) {
        let row_top = top + HEADER + (index - scroll) as f32 * ROW;
        let row = UiRect::new(left + 8.0, row_top, card.right - 8.0, row_top + ROW);
        let choose = state.clone();
        let chosen = root.clone();
        let row_focus = focus.clone();
        let label = crate::ui::display_path(root);
        let selected = index == picker.selected;
        surface = surface.child(
            Element::new(move |cx| {
                let hovered = cx.context.interaction_flags(&cx.id).hovered;
                UiElement::panel(
                    cx.id,
                    row,
                    if hovered || selected {
                        VisualStyle::filled(theme::c().selection).radius(3.0)
                    } else {
                        VisualStyle::default()
                    },
                )
                .children(cx.children)
            })
            .key(format!("terminal-workspace-{index}"))
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(CursorIcon::Pointer)
            .semantics(Semantics::new(SemanticRole::Button).name(label.clone()))
            .on_click(move || {
                if choose.run(Command::ChooseWorkspace(chosen.clone())).applied {
                    row_focus.focus();
                }
            })
            .child(text(
                UiRect::new(row.left + 8.0, row.top, row.right - 8.0, row.bottom),
                ellipsize(&label, (row.width() - 16.0).max(0.0), theme::SMALL, 400),
                theme::mono(theme::c().text, theme::SMALL),
            )),
        );
    }
    overlay = overlay.child(surface);
    overlay
}
