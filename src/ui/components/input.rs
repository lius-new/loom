//! Reusable single-line text input with renderer-backed measurement.

use std::ops::Range;

use lgui::core::{
    Color, CursorIcon, EventPolicy, SemanticRole, Semantics, TextStyle, UiElement, UiFocusHandle,
    UiId, clip,
};
use lgui::prelude::{Element, State, UiRect, VisualStyle, group, panel};
use lgui::services::ServicesContextExt;

use crate::editor::commands::{self, Command};
use crate::model::buffer::{Movement, TextBuffer};

use super::text::SingleLineText;

const TEXT_INSET: f32 = 6.0;
const CURSOR_MARGIN: f32 = 8.0;

#[derive(Clone)]
pub struct InputState {
    buffer: TextBuffer,
    pub focused: bool,
    pub scroll_x: f32,
    pub preedit: String,
    pub preedit_cursor: Option<Range<usize>>,
    drag_anchor: Option<usize>,
}

impl Default for InputState {
    fn default() -> Self {
        Self::new("")
    }
}

impl InputState {
    pub fn new(value: impl Into<String>) -> Self {
        let mut buffer = TextBuffer::new(value);
        buffer.navigate(Movement::Finish, false);
        Self {
            buffer,
            focused: false,
            scroll_x: 0.0,
            preedit: String::new(),
            preedit_cursor: None,
            drag_anchor: None,
        }
    }

    pub fn text(&self) -> &str {
        self.buffer.text()
    }

    pub fn clear(&mut self) {
        self.buffer = TextBuffer::new("");
        self.scroll_x = 0.0;
        self.preedit.clear();
        self.preedit_cursor = None;
        self.drag_anchor = None;
    }

    pub fn set_text(&mut self, value: impl Into<String>) {
        let value = value.into();
        self.buffer = TextBuffer::new(value);
        self.buffer.navigate(Movement::Finish, false);
        self.preedit.clear();
        self.preedit_cursor = None;
    }
}

#[derive(Clone)]
pub struct InputBinding<T> {
    state: State<T>,
    read: fn(&T) -> &InputState,
    write: fn(&mut T) -> &mut InputState,
}

impl<T> InputBinding<T>
where
    T: Clone + Send + 'static,
{
    pub fn new(
        state: State<T>,
        read: fn(&T) -> &InputState,
        write: fn(&mut T) -> &mut InputState,
    ) -> Self {
        Self { state, read, write }
    }

    fn get(&self) -> InputState {
        let state = self.state.get();
        (self.read)(&state).clone()
    }

    fn update(&self, update: impl FnOnce(&mut InputState)) {
        let write = self.write;
        self.state.update(move |state| update(write(state)));
    }
}

#[derive(Clone, Copy)]
pub struct InputStyle {
    pub text: TextStyle,
    pub placeholder: TextStyle,
    pub caret: Color,
    pub selection: Color,
}

pub struct InputOptions<T> {
    pub label: &'static str,
    pub placeholder: &'static str,
    pub style: InputStyle,
    pub on_submit: Option<fn(&State<T>)>,
    pub on_cancel: Option<fn(&State<T>)>,
    pub on_blur: Option<fn(&State<T>)>,
}

pub fn render<T>(
    rect: UiRect,
    id: UiId,
    focus: UiFocusHandle,
    binding: InputBinding<T>,
    options: InputOptions<T>,
) -> Element
where
    T: Clone + Send + 'static,
{
    let snapshot = binding.get();
    let style = options.style;
    let label = options.label;
    let placeholder = options.placeholder;
    let on_submit = options.on_submit;
    let on_cancel = options.on_cancel;
    let on_blur = options.on_blur;
    let action_state = binding.state.clone();
    let blur_action_state = binding.state.clone();
    let content = UiRect::new(
        rect.left + TEXT_INSET,
        rect.top,
        rect.right - TEXT_INSET,
        rect.bottom,
    );
    let text_bounds = UiRect::new(
        content.left,
        content.top,
        content.left + 100_000.0,
        content.bottom,
    );
    let measure_bounds = UiRect::new(0.0, 0.0, 100_000.0, rect.height());
    let measured = SingleLineText::new(snapshot.text(), measure_bounds, style.text);
    let viewport_w = content.width().max(1.0);
    let base_caret = measured.caret_offset(snapshot.buffer.cursor());
    let preedit_cursor = snapshot
        .preedit_cursor
        .as_ref()
        .map_or(snapshot.preedit.len(), |range| range.start)
        .min(snapshot.preedit.len());
    let preedit_prefix = snapshot.preedit.get(..preedit_cursor).unwrap_or("");
    let preedit_bounds = measure_bounds;
    let preedit_caret = SingleLineText::new(preedit_prefix, preedit_bounds, style.text).width();
    let preedit_width = SingleLineText::new(&snapshot.preedit, preedit_bounds, style.text).width();
    let content_width = measured.width().max(base_caret + preedit_width);
    let scroll = scroll_for_position(
        snapshot.scroll_x,
        content_width,
        base_caret + preedit_caret,
        viewport_w,
    );
    let caret_x = (content.left + base_caret + preedit_caret - scroll)
        .clamp(content.left, content.right);
    let caret_height = (style.text.height + 4.0)
        .min((rect.height() - 4.0).max(1.0))
        .max(1.0);
    let caret_top = rect.top + (rect.height() - caret_height) / 2.0;
    let caret_rect = UiRect::new(caret_x, caret_top, caret_x + 1.0, caret_top + caret_height);
    let mut semantics = Semantics::new(SemanticRole::TextInput)
        .name(label)
        .value(snapshot.text().to_owned());
    semantics.text.character_count = Some(snapshot.text().chars().count());

    let pointer_layout = measured.clone();
    let pointer_binding = binding.clone();
    let pointer_focus = focus.clone();
    let drag_layout = measured.clone();
    let drag_binding = binding.clone();
    let up_binding = binding.clone();
    let input_binding = binding.clone();
    let change_binding = binding.clone();
    let key_binding = binding.clone();
    let composition_binding = binding.clone();
    let composition_end_binding = binding.clone();
    let focus_binding = binding.clone();
    let blur_binding = binding.clone();

    let mut root = Element::new(move |cx| {
        UiElement::panel(id, rect, VisualStyle::default())
            .ime_cursor_rect(caret_rect)
            .children(cx.children)
    })
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(semantics)
    .cursor(CursorIcon::Text)
    .on_pointer_down(move |cx, pointer| {
        pointer_focus.focus();
        let offset = pointer_layout.hit_byte_offset(
            pointer.point.x - content.left + scroll,
            pointer.point.y - rect.top,
        );
        pointer_binding.update(move |input| {
            input.buffer.select_to(offset, false);
            input.drag_anchor = Some(offset);
            reveal_cursor(input, viewport_w, style.text);
        });
        cx.stop_propagation();
    })
    .on_pointer_drag(move |cx, pointer| {
        let offset = drag_layout.hit_byte_offset(
            pointer.point.x - content.left + scroll,
            pointer.point.y - rect.top,
        );
        drag_binding.update(move |input| {
            if let Some(anchor) = input.drag_anchor {
                input.buffer.set_cursor(anchor);
                input.buffer.select_to(offset, true);
                reveal_cursor(input, viewport_w, style.text);
            }
        });
        cx.stop_propagation();
    })
    .on_pointer_up(move |_cx, _pointer| {
        up_binding.update(|input| input.drag_anchor = None);
    })
    .on_input(move |cx, value| {
        let value = single_line(value);
        if !value.is_empty() {
            input_binding.update(move |input| {
                input.buffer.insert(&value);
                input.preedit.clear();
                input.preedit_cursor = None;
                reveal_cursor(input, viewport_w, style.text);
            });
            cx.stop_propagation();
        }
    })
    .on_change(move |_cx, value| {
        let value = single_line(value.unwrap_or_default());
        change_binding.update(move |input| {
            input.set_text(value);
            reveal_cursor(input, viewport_w, style.text);
        });
    })
    .on_composition_update(move |_cx, value, cursor| {
        let value = value.to_owned();
        let cursor = cursor.clone();
        composition_binding.update(move |input| {
            input.preedit = value;
            input.preedit_cursor = cursor;
        });
    })
    .on_composition_end(move |_cx| {
        composition_end_binding.update(|input| {
            input.preedit.clear();
            input.preedit_cursor = None;
        });
    })
    .on_key_down(move |cx, event| {
        if !key_binding.get().preedit.is_empty() {
            return;
        }
        let Some(command) = commands::command_for(event, 1) else {
            if (event.modifiers.ctrl() || event.modifiers.meta()) && !event.modifiers.alt() {
                cx.prevent_default();
            }
            return;
        };
        if command == Command::Enter {
            if let Some(on_submit) = on_submit {
                on_submit(&action_state);
            }
            cx.prevent_default();
            cx.stop_propagation();
            return;
        }
        if command == Command::Escape
            && let Some(on_cancel) = on_cancel
        {
            on_cancel(&action_state);
            cx.prevent_default();
            cx.stop_propagation();
            return;
        }
        if matches!(command, Command::Indent(_)) {
            return;
        }
        let clipboard = cx.application().clipboard();
        key_binding.update(move |input| {
            apply_command(input, command, clipboard.as_ref());
            reveal_cursor(input, viewport_w, style.text);
        });
        cx.prevent_default();
        cx.stop_propagation();
    })
    .on_focus(move |_cx| focus_binding.update(|input| input.focused = true))
    .on_blur(move |_cx| {
        blur_binding.update(|input| {
            input.focused = false;
            input.drag_anchor = None;
            input.preedit.clear();
            input.preedit_cursor = None;
            input.buffer.break_undo_group();
        });
        if let Some(on_blur) = on_blur {
            on_blur(&blur_action_state);
        }
    });

    let mut content_group = group(text_bounds);
    if let Some(selection) = snapshot.buffer.selection() {
        for selection in measured.selection_rects(selection) {
            content_group = content_group.child(panel(
                UiRect::new(
                    content.left + selection.left,
                    rect.top + selection.top,
                    content.left + selection.right,
                    rect.top + selection.bottom,
                ),
                VisualStyle::filled(style.selection).alpha(70),
            ));
        }
    }
    content_group = content_group.child(if snapshot.text().is_empty() && snapshot.preedit.is_empty() {
        SingleLineText::new(placeholder, text_bounds, style.placeholder).element()
    } else {
        SingleLineText::new(snapshot.text(), text_bounds, style.text).element()
    });
    if !snapshot.preedit.is_empty() {
        let preedit_left = content.left + measured.caret_offset(snapshot.buffer.cursor());
        let preedit_bounds = UiRect::new(
            preedit_left,
            text_bounds.top,
            text_bounds.right,
            text_bounds.bottom,
        );
        let preedit = SingleLineText::new(
            snapshot.preedit.clone(),
            preedit_bounds,
            style.text,
        );
        let underline_right = preedit_left + preedit.width().max(2.0);
        content_group = content_group
            .child(preedit.element())
            .child(panel(
                UiRect::new(
                    preedit_left,
                    rect.bottom - 8.0,
                    underline_right,
                    rect.bottom - 7.0,
                ),
                VisualStyle::filled(style.caret),
            ));
    }
    root = root.child(clip(content, -scroll, 0.0).child(content_group));
    if snapshot.focused {
        root = root.child(panel(
            caret_rect,
            VisualStyle::filled(style.caret),
        ));
    }
    root
}

fn apply_command(
    input: &mut InputState,
    command: Command,
    clipboard: &dyn lgui::services::Clipboard,
) {
    match command {
        Command::Copy | Command::Cut => {
            if let Some(value) = input.buffer.selected_text() {
                if clipboard.write_text(value).is_ok() && command == Command::Cut {
                    input.buffer.backspace();
                }
                input.buffer.break_undo_group();
            }
        }
        Command::Paste => {
            if let Ok(Some(value)) = clipboard.read_text() {
                input.buffer.insert(&single_line(&value));
                input.buffer.break_undo_group();
            }
        }
        Command::Move(Movement::Up | Movement::PageUp(_), extend) => {
            input.buffer.navigate(Movement::Home, extend);
        }
        Command::Move(Movement::Down | Movement::PageDown(_), extend) => {
            input.buffer.navigate(Movement::End, extend);
        }
        _ => commands::apply(&mut input.buffer, command),
    }
}

fn visible_scroll(input: &InputState, measured: &SingleLineText, viewport_w: f32) -> f32 {
    scroll_for_position(
        input.scroll_x,
        measured.width(),
        measured.caret_offset(input.buffer.cursor()),
        viewport_w,
    )
}

fn scroll_for_position(
    stored: f32,
    content_width: f32,
    caret: f32,
    viewport_w: f32,
) -> f32 {
    let max_scroll = (content_width - viewport_w).max(0.0);
    let mut scroll = stored.clamp(0.0, max_scroll);
    if caret < scroll + CURSOR_MARGIN {
        scroll = (caret - CURSOR_MARGIN).max(0.0);
    } else if caret > scroll + viewport_w - CURSOR_MARGIN {
        scroll = caret - viewport_w + CURSOR_MARGIN;
    }
    scroll.clamp(0.0, max_scroll)
}

fn reveal_cursor(input: &mut InputState, viewport_w: f32, style: TextStyle) {
    let bounds = UiRect::new(0.0, 0.0, 100_000.0, style.height + 12.0);
    let measured = SingleLineText::new(input.text(), bounds, style);
    input.scroll_x = visible_scroll(input, &measured, viewport_w);
}

fn single_line(value: &str) -> String {
    value.replace(['\r', '\n'], "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgui::application::{AppView, ApplicationContext};
    use lgui::core::{
        ImeEvent, InputEvent, Point, PointerButton, PointerData, UiScale, dispatch_runtime_output,
    };
    use lgui::session::UiSession;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Harness {
        input: InputState,
    }

    fn read(state: &Harness) -> &InputState {
        &state.input
    }

    fn write(state: &mut Harness) -> &mut InputState {
        &mut state.input
    }

    fn send(
        session: &mut UiSession,
        application: &ApplicationContext,
        view: &AppView,
        viewport: UiRect,
        input: InputEvent,
    ) {
        let events = session.handle_input(input);
        dispatch_runtime_output(
            events,
            application,
            &lgui::window::WindowId::new("single-line-input-test"),
            |action| session.handle_default_action(action),
            |_| {},
        );
        session.render_view(view, viewport, UiScale::ONE);
    }

    #[test]
    fn committed_ascii_and_ime_text_leave_the_caret_at_the_end() {
        let exposed = Arc::new(Mutex::new(None::<State<Harness>>));
        let output = exposed.clone();
        let viewport = UiRect::new(0.0, 0.0, 240.0, 38.0);
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state(Harness::default());
            *output.lock().unwrap() = Some(state.clone());
            let id = cx.use_stable_id();
            let focus = cx.focus_handle(id.clone());
            render(
                viewport,
                id,
                focus,
                InputBinding::new(state, read, write),
                InputOptions {
                    label: "Test input",
                    placeholder: "Test input",
                    style: InputStyle {
                        text: TextStyle::new(Color::WHITE, 10.0, 400),
                        placeholder: TextStyle::new(Color::WHITE, 10.0, 400),
                        caret: Color::WHITE,
                        selection: Color::WHITE,
                    },
                    on_submit: None,
                    on_cancel: None,
                    on_blur: None,
                },
            )
        });
        let mut session = UiSession::new();
        let application = ApplicationContext::empty(Default::default());
        let _text = lgui::backend::install_text_environment(
            &application,
            lgui::render_skia::skia_text_system_handle(),
        );
        session.render_view(&view, viewport, UiScale::ONE);
        let state = exposed.lock().unwrap().clone().unwrap();

        send(
            &mut session,
            &application,
            &view,
            viewport,
            InputEvent::PointerDown {
                pointer: PointerData::mouse(Point::new(12.0, 19.0)),
                button: PointerButton::Left,
            },
        );
        send(
            &mut session,
            &application,
            &view,
            viewport,
            InputEvent::TextInput("abc".to_owned()),
        );
        assert_eq!(state.get().input.text(), "abc");
        assert_eq!(state.get().input.buffer.cursor(), 3);
        let rendered_text = session
            .tree()
            .nodes()
            .iter()
            .filter_map(|node| node.text.as_deref())
            .collect::<Vec<_>>();
        assert!(
            rendered_text.contains(&"abc"),
            "rendered text: {rendered_text:?}"
        );
        let ascii_caret = session
            .tree()
            .nodes()
            .iter()
            .find_map(|node| node.ime_cursor_rect)
            .unwrap();
        assert!(
            ascii_caret.left > TEXT_INSET,
            "ASCII caret did not advance: {ascii_caret:?}"
        );
        assert_eq!(ascii_caret.height(), 14.0);

        send(
            &mut session,
            &application,
            &view,
            viewport,
            InputEvent::Ime(ImeEvent::Preedit {
                text: "nihao".to_owned(),
                cursor: Some(5..5),
            }),
        );
        send(
            &mut session,
            &application,
            &view,
            viewport,
            InputEvent::Ime(ImeEvent::Commit("你好".to_owned())),
        );
        assert_eq!(state.get().input.text(), "abc你好");
        assert_eq!(state.get().input.buffer.cursor(), "abc你好".len());
        assert!(state.get().input.preedit.is_empty());
        let ime_caret = session
            .tree()
            .nodes()
            .iter()
            .find_map(|node| node.ime_cursor_rect)
            .unwrap();
        assert!(ime_caret.left > ascii_caret.left);
    }
}
