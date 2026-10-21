//! A bounded, read-only context panel inside the editor viewport.
use crate::ui::{
    editor_binding::{Binding, Event},
    fonts, theme,
};
use lgui::{
    core::{CursorIcon, EventPolicy, SemanticRole, Semantics, UiEventContext, clip},
    prelude::{Element, UiRect, VisualStyle, panel, text},
};
pub(crate) struct View {
    pub revision: u64,
    pub title: String,
    pub lines: Vec<(usize, String, bool)>,
}
pub(crate) fn render(view: &View, area: UiRect, binding: Binding) -> Element {
    let row = fonts::editor().line_h;
    let height = ((view.lines.len() + 1) as f32 * row + 16.0).min(area.height() * 0.65);
    let r = UiRect::new(
        area.left + 4.0,
        area.bottom - height - 4.0,
        area.right - 4.0,
        area.bottom - 4.0,
    );
    let policy = EventPolicy {
        focus: false,
        ..EventPolicy::INTERACTIVE
    };
    let mut body = theme::bordered(r, theme::c().surface, theme::c().border, 4.0, 1.0)
        .key("language-definition-peek")
        .event_policy(policy)
        .on_pointer_down_with_button(|cx, _, _| cx.stop_propagation());
    let header = UiRect::new(r.left + 8.0, r.top + 4.0, r.right - 80.0, r.top + row + 4.0);
    body = body.child(text(
        header,
        view.title.clone(),
        theme::sans(theme::c().text_soft, theme::SMALL),
    ));
    for (label, left, open) in [("Open", r.right - 76.0, true), ("×", r.right - 28.0, false)] {
        let button = UiRect::new(left, r.top + 4.0, left + 24.0, r.top + 4.0 + row);
        let send = binding.clone();
        body = body.child(
            panel(button, VisualStyle::default())
                .event_policy(policy)
                .cursor(CursorIcon::Pointer)
                .semantics(
                    Semantics::new(SemanticRole::Button).name(if label == "Open" {
                        "Open preview target"
                    } else {
                        "Close preview"
                    }),
                )
                .on_click({
                    let revision = view.revision;
                    move |cx: &mut UiEventContext| {
                        cx.stop_propagation();
                        send.send(if open {
                            Event::PeekOpen { revision }
                        } else {
                            Event::PeekClose { revision }
                        });
                    }
                })
                .child(if open {
                    text(button, label, theme::sans(theme::c().accent, theme::SMALL)).into()
                } else {
                    crate::ui::tabs::close_glyph(
                        lgui::core::UiId::new("language-definition-peek.close"),
                        button,
                        theme::c().accent,
                    )
                }),
        );
    }
    let code = UiRect::new(
        r.left + 8.0,
        r.top + row + 8.0,
        r.right - 8.0,
        r.bottom - 4.0,
    );
    let mut lines = clip(code, 0.0, 0.0);
    for (i, (number, line, active)) in view.lines.iter().enumerate() {
        let y = code.top + i as f32 * row;
        let r = UiRect::new(code.left, y, code.right, y + row);
        let style = if *active {
            VisualStyle::filled(theme::c().active_line)
        } else {
            VisualStyle::default()
        };
        lines = lines.child(panel(r, style).child(text(
            r,
            format!("{number:>4}  {line}"),
            fonts::editor().style(theme::c().text_bright),
        )));
    }
    body.child(lines)
}
