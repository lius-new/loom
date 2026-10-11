//! Terminal choices and editable user profiles on the Settings page.

use crate::state::AppState;
use crate::terminal_profiles::{ShellProfile, format_arguments, parse_arguments, resolve_program};
use crate::terminal_session::ShellKind;
use crate::theme;
use crate::ui::components::input::{self, InputBinding, InputOptions, InputState, InputStyle};
use crate::ui::tabs::ellipsize;
use lgui::core::{CursorIcon, EventPolicy};
use lgui::prelude::{Element, State, UiRect, VisualStyle, component, group, panel, text};
use std::path::PathBuf;

const ROW: f32 = 44.0;
const FORM_H: f32 = 250.0;

#[derive(Clone)]
pub struct ProfileEditor {
    pub open: bool,
    pub id: Option<u64>,
    pub name: InputState,
    pub program: InputState,
    pub args: InputState,
    pub error: Option<String>,
}

pub fn edit(app: &mut AppState, id: Option<u64>) {
    let profile = id.and_then(|id| app.terminal_profiles.iter().find(|p| p.id == id));
    app.shell_editor = Some(ProfileEditor {
        open: true,
        id,
        name: InputState::new(profile.map(|p| p.name.clone()).unwrap_or_default()),
        program: InputState::new(
            profile
                .map(|p| crate::ui::display_path(&p.program))
                .unwrap_or_default(),
        ),
        args: InputState::new(
            profile
                .map(|p| format_arguments(&p.args))
                .unwrap_or_default(),
        ),
        error: None,
    });
}

pub fn save_editor(app: &mut AppState) -> bool {
    let Some(editor) = app.shell_editor.as_ref() else {
        return false;
    };
    let validate = || -> Result<ShellProfile, String> {
        let name = editor.name.text().trim();
        if name.is_empty() {
            return Err("Enter a name for this terminal.".into());
        }
        let raw = editor.program.text().trim();
        let raw = raw
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(raw);
        let program = resolve_program(&PathBuf::from(raw))
            .ok_or("Choose an available executable or enter its command name.")?;
        let program = program
            .canonicalize()
            .map(|p| PathBuf::from(crate::ui::display_path(&p)))
            .unwrap_or(program);
        let args = parse_arguments(editor.args.text())?;
        let id = editor.id.unwrap_or_else(|| {
            let clock = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;
            clock.max(
                app.terminal_profiles
                    .iter()
                    .map(|p| p.id)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1),
            )
        });
        Ok(ShellProfile {
            id,
            name: name.into(),
            program,
            args,
        })
    };
    match validate() {
        Ok(profile) => {
            if let Some(old) = app
                .terminal_profiles
                .iter_mut()
                .find(|p| p.id == profile.id)
            {
                *old = profile;
            } else {
                app.terminal_profiles.push(profile);
            }
            close_editor(app);
            app.refresh_shells();
            true
        }
        Err(error) => {
            app.shell_editor.as_mut().unwrap().error = Some(error);
            false
        }
    }
}

pub fn remove(app: &mut AppState, id: u64) {
    app.terminal_profiles.retain(|p| p.id != id);
    // A deleted profile cannot be repaired, unlike an unavailable one.
    if app.default_shell == ShellKind::Custom(id) {
        app.default_shell = ShellKind::default();
    }
    if app
        .shell_editor
        .as_ref()
        .is_some_and(|editor| editor.id == Some(id))
    {
        close_editor(app);
    }
    app.refresh_shells();
}

fn close_editor(app: &mut AppState) {
    // Blur handlers of the unmounted inputs still reference this draft.
    if let Some(editor) = &mut app.shell_editor {
        editor.open = false;
    }
}

pub fn height(app: &AppState) -> f32 {
    58.0 + ROW * app.shell_options.len().max(1) as f32
        + 38.0
        + if app.shell_editor.as_ref().is_some_and(|e| e.open) {
            FORM_H
        } else {
            0.0
        }
}

pub fn render(rect: UiRect, app: &AppState, state: State<AppState>) -> Element {
    let mut root = group(rect)
        .child(text(
            UiRect::new(rect.left, rect.top + 11.0, rect.right, rect.top + 29.0),
            "Default terminal",
            theme::sans(theme::c().text, theme::UI_SIZE),
        ))
        .child(text(
            UiRect::new(rect.left, rect.top + 30.0, rect.right, rect.top + 48.0),
            "Choose a default, or add another shell with its own launch arguments.",
            theme::sans(theme::c().text_dim, theme::SMALL + 1.0),
        ));
    let mut top = rect.top + 58.0;
    for option in &app.shell_options {
        let row = UiRect::new(rect.left, top, rect.right, top + ROW - 2.0);
        let available = option.available();
        let active = option.kind == app.default_shell;
        let custom = matches!(option.kind, ShellKind::Custom(_));
        let controls = if custom { 112.0 } else { 0.0 };
        let name_right = (row.right - controls - 8.0).max(row.left + 24.0);
        let name = if available {
            option.name.clone()
        } else if active && app.effective_shell() != option.kind {
            format!(
                "{} (unavailable; using {})",
                option.name,
                app.shell_name(app.effective_shell())
            )
        } else {
            format!("{} (unavailable)", option.name)
        };
        let path = option
            .program
            .as_ref()
            .map(|p| crate::ui::display_path(p))
            .unwrap_or_else(|| "Update the executable path to use this terminal.".into());
        let mut item = panel(
            row,
            if active {
                VisualStyle::filled(theme::c().selection).radius(3.0)
            } else {
                VisualStyle::default()
            },
        )
        .key(format!("settings-shell-{:?}", option.kind));
        if available {
            let select = state.clone();
            let kind = option.kind;
            item = item
                .event_policy(EventPolicy::INTERACTIVE)
                .cursor(CursorIcon::Pointer)
                .on_click(move || select.update(|app| app.default_shell = kind));
        }
        item = item
            .child(text(
                UiRect::new(
                    row.left + 4.0,
                    row.top + 6.0,
                    row.left + 22.0,
                    row.top + 26.0,
                ),
                if active { "●" } else { "○" },
                theme::sans(theme::c().accent, theme::UI_SIZE),
            ))
            .child(text(
                UiRect::new(row.left + 26.0, row.top + 3.0, name_right, row.top + 22.0),
                ellipsize(
                    &name,
                    (name_right - row.left - 26.0).max(0.0),
                    theme::UI_SIZE,
                    400,
                ),
                theme::sans(
                    if available {
                        theme::c().text
                    } else {
                        theme::c().text_dim
                    },
                    theme::UI_SIZE,
                ),
            ))
            .child(text(
                UiRect::new(row.left + 26.0, row.top + 23.0, name_right, row.bottom),
                ellipsize(
                    &path,
                    (name_right - row.left - 26.0).max(0.0),
                    theme::SMALL,
                    400,
                ),
                theme::sans(theme::c().text_dim, theme::SMALL),
            ));
        if let ShellKind::Custom(id) = option.kind {
            let edit_state = state.clone();
            let delete_state = state.clone();
            item = item
                .child(button(
                    UiRect::new(
                        row.right - 108.0,
                        row.top + 6.0,
                        row.right - 60.0,
                        row.top + 34.0,
                    ),
                    "Edit",
                    format!("shell-edit-{id}"),
                    move || edit_state.update(|app| edit(app, Some(id))),
                ))
                .child(button(
                    UiRect::new(
                        row.right - 56.0,
                        row.top + 6.0,
                        row.right - 2.0,
                        row.top + 34.0,
                    ),
                    "Delete",
                    format!("shell-delete-{id}"),
                    move || delete_state.update(|app| remove(app, id)),
                ));
        }
        root = root.child(item);
        top += ROW;
    }
    if app.shell_options.is_empty() {
        root = root.child(text(
            UiRect::new(rect.left, top, rect.right, top + ROW),
            "No system shell found. Add a terminal below.",
            theme::sans(theme::c().text_dim, theme::UI_SIZE),
        ));
        top += ROW;
    }
    let add = state.clone();
    let refresh = state.clone();
    root = root
        .child(button(
            UiRect::new(rect.left, top + 2.0, rect.left + 108.0, top + 30.0),
            "Add terminal",
            "shell-add".into(),
            move || add.update(|app| edit(app, None)),
        ))
        .child(button(
            UiRect::new(rect.left + 116.0, top + 2.0, rect.left + 184.0, top + 30.0),
            "Refresh",
            "shell-refresh".into(),
            move || refresh.update(AppState::refresh_shells),
        ));
    top += 38.0;
    if let Some(editor) = app.shell_editor.as_ref().filter(|e| e.open) {
        root = root.child(editor_form(
            UiRect::new(rect.left, top, rect.right, top + FORM_H),
            state,
            editor.error.as_deref(),
        ));
    }
    root
}

fn button(
    rect: UiRect,
    label: &'static str,
    key: String,
    click: impl Fn() + Send + Sync + 'static,
) -> Element {
    theme::bordered(rect, theme::c().surface, theme::c().border, 3.0, 1.0)
        .key(key)
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_click(move |cx: &mut lgui::core::UiEventContext| {
            click();
            cx.stop_propagation();
        })
        .child(text(
            UiRect::new(
                rect.left + 6.0,
                rect.top + 5.0,
                rect.right - 4.0,
                rect.bottom,
            ),
            label,
            theme::sans(theme::c().text, theme::UI_SIZE),
        ))
}

fn editor_form(rect: UiRect, state: State<AppState>, error: Option<&str>) -> Element {
    let mut root = theme::bordered(rect, theme::c().surface, theme::c().border, 4.0, 1.0);
    type Read = fn(&AppState) -> &InputState;
    type Write = fn(&mut AppState) -> &mut InputState;
    let fields: [(&str, &'static str, Read, Write); 3] = [
        (
            "Name",
            "My shell",
            |app| &app.shell_editor.as_ref().unwrap().name,
            |app| &mut app.shell_editor.as_mut().unwrap().name,
        ),
        (
            "Executable",
            "Program path or command name",
            |app| &app.shell_editor.as_ref().unwrap().program,
            |app| &mut app.shell_editor.as_mut().unwrap().program,
        ),
        (
            "Arguments",
            "Optional; quote arguments containing spaces",
            |app| &app.shell_editor.as_ref().unwrap().args,
            |app| &mut app.shell_editor.as_mut().unwrap().args,
        ),
    ];
    for (index, (label, placeholder, read, write)) in fields.into_iter().enumerate() {
        let top = rect.top + 9.0 + index as f32 * 58.0;
        root = root.child(text(
            UiRect::new(rect.left + 12.0, top, rect.right - 12.0, top + 19.0),
            label,
            theme::sans(theme::c().text_muted, theme::SMALL),
        ));
        let input_rect = UiRect::new(
            rect.left + 12.0,
            top + 21.0,
            (rect.right - 12.0).max(rect.left + 12.0),
            top + 51.0,
        );
        let input_state = state.clone();
        root = root.child(
            component(input_rect, move |cx, &rect| {
                let id = cx.use_stable_id();
                let focus = cx.focus_handle(id.clone());
                let focus_state = input_state.clone();
                if index == 0 {
                    let mounted = focus.clone();
                    cx.use_effect((), move || {
                        mounted.focus();
                    });
                }
                input::render(
                    rect,
                    id,
                    focus,
                    InputBinding::new(input_state.clone(), read, write),
                    InputOptions {
                        label: placeholder,
                        placeholder,
                        style: InputStyle {
                            text: theme::mono(theme::c().text, theme::UI_SIZE),
                            placeholder: theme::sans(theme::c().text_dim, theme::UI_SIZE),
                            caret: theme::c().text,
                            selection: theme::c().selection,
                        },
                        on_submit: Some(|state| {
                            state.update(|app| {
                                save_editor(app);
                            })
                        }),
                        on_cancel: Some(|state| state.update(close_editor)),
                        on_blur: None,
                    },
                )
                .on_focus(move |_| focus_state.update(|app| app.terminal_focused = false))
            })
            .key(format!("shell-profile-input-{index}")),
        );
    }
    if let Some(error) = error {
        root = root.child(text(
            UiRect::new(
                rect.left + 12.0,
                rect.top + 186.0,
                rect.right - 12.0,
                rect.top + 208.0,
            ),
            error.to_owned(),
            theme::sans(theme::c().text, theme::SMALL),
        ));
    }
    let save = state.clone();
    root.child(button(
        UiRect::new(
            rect.left + 12.0,
            rect.top + 212.0,
            rect.left + 72.0,
            rect.top + 240.0,
        ),
        "Save",
        "shell-save".into(),
        move || {
            save.update(|app| {
                save_editor(app);
            })
        },
    ))
    .child(button(
        UiRect::new(
            rect.left + 80.0,
            rect.top + 212.0,
            rect.left + 148.0,
            rect.top + 240.0,
        ),
        "Cancel",
        "shell-cancel".into(),
        move || state.update(close_editor),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_editor_renders_at_normal_and_minimized_sizes() {
        use lgui::application::AppView;
        use lgui::core::{PhysicalRect, UiScale};
        use lgui::render_skia::SkiaSoftwareSurface;
        use lgui::renderer::{FrameInfo, FrameReason};
        use lgui::session::UiSession;
        use std::sync::Arc;
        for (width, height) in [(720.0, 650.0), (80.0, 80.0)] {
            let viewport = UiRect::new(0.0, 0.0, width, height);
            let view: AppView = Arc::new(move |cx| {
                let state = cx.state_with(|| {
                    let mut app = AppState::new();
                    let profile = ShellProfile {
                        id: 99,
                        name: "Developer shell".into(),
                        program: std::env::current_exe().unwrap(),
                        args: vec!["--login".into()],
                    };
                    app.terminal_profiles.push(profile);
                    app.refresh_shells();
                    edit(&mut app, Some(99));
                    app
                });
                panel(viewport, VisualStyle::filled(theme::c().bg)).child(render(
                    UiRect::new(
                        12.0,
                        0.0,
                        (width - 12.0).max(12.0),
                        self::height(&state.get()),
                    ),
                    &state.get(),
                    state.clone(),
                ))
            });
            let mut session = UiSession::new();
            session.render_view(&view, viewport, UiScale::ONE);
            let physical = PhysicalRect::new(0, 0, width as i32, height as i32);
            let dirty = [physical];
            let frame = FrameInfo::new(
                physical,
                &dirty,
                UiScale::ONE,
                FrameReason::SceneChange,
                true,
            );
            let mut surface = SkiaSoftwareSurface::new(16 * 1024 * 1024);
            surface
                .draw(
                    &session.tree().scene().project_to_physical(UiScale::ONE),
                    &frame,
                )
                .unwrap();
            if width == 720.0
                && let Some(path) = std::env::var_os("LOOM_PROFILE_PREVIEW")
            {
                let mut rgba = surface.pixels().to_vec();
                for pixel in rgba.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
                image::save_buffer(
                    path,
                    &rgba,
                    width as u32,
                    height as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn profiles_can_be_added_edited_selected_removed_and_repaired() {
        let mut app = AppState::new();
        edit(&mut app, None);
        assert!(!save_editor(&mut app));
        let editor = app.shell_editor.as_mut().unwrap();
        editor.name.set_text("Custom shell");
        editor
            .program
            .set_text(std::env::current_exe().unwrap().display().to_string());
        editor.args.set_text(r#"--login "argument with spaces" """#);
        assert!(save_editor(&mut app));
        let original = app.terminal_profiles[0].clone();
        assert_eq!(original.args, vec!["--login", "argument with spaces", ""]);
        let id = original.id;
        app.default_shell = ShellKind::Custom(id);
        edit(&mut app, Some(id));
        app.shell_editor
            .as_mut()
            .unwrap()
            .name
            .set_text("Renamed shell");
        assert!(save_editor(&mut app));
        assert_eq!(app.terminal_profiles.len(), 1);
        assert_eq!(app.terminal_profiles[0].id, id);
        assert_eq!(app.default_shell, ShellKind::Custom(id));
        app.terminal_profiles[0].program = "/missing-loom-shell".into();
        app.refresh_shells();
        assert!(
            app.shell_options
                .iter()
                .any(|p| p.kind == ShellKind::Custom(id) && !p.available())
        );
        // The saved choice survives while another shell stands in for it.
        assert_eq!(app.default_shell, ShellKind::Custom(id));
        assert_ne!(app.effective_shell(), ShellKind::Custom(id));
        assert!(app.default_shell_fallback.is_some());
        edit(&mut app, Some(id));
        app.shell_editor
            .as_mut()
            .unwrap()
            .program
            .set_text(std::env::current_exe().unwrap().display().to_string());
        assert!(save_editor(&mut app));
        assert_eq!(app.effective_shell(), ShellKind::Custom(id));
        assert_eq!(app.default_shell_fallback, None);
        remove(&mut app, id);
        assert_eq!(app.default_shell, ShellKind::default());
        assert!(app.terminal_profiles.is_empty());
        assert!(!app.shell_editor.as_ref().unwrap().open);
    }

    #[test]
    fn profile_inputs_save_and_unmount_without_losing_focus_state_or_bubbling_delete() {
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{
            InputEvent, Point, PointerButton, PointerData, UiScale, dispatch_runtime_output,
        };
        use lgui::session::UiSession;
        use std::sync::{Arc, Mutex};
        let exposed = Arc::new(Mutex::new(None::<State<AppState>>));
        let output = exposed.clone();
        let viewport = UiRect::new(0.0, 0.0, 720.0, 600.0);
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state_with(|| {
                let mut app = AppState::new();
                app.shell_options.clear();
                edit(&mut app, None);
                app
            });
            *output.lock().unwrap() = Some(state.clone());
            render(
                UiRect::new(12.0, 0.0, 708.0, height(&state.get())),
                &state.get(),
                state.clone(),
            )
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        session.runtime().run_effects();
        let state = exposed.lock().unwrap().clone().unwrap();
        let context = ApplicationContext::empty(Default::default());
        fn send(
            session: &mut UiSession,
            context: &ApplicationContext,
            view: &AppView,
            viewport: UiRect,
            event: InputEvent,
        ) {
            let events = session.handle_input(event);
            dispatch_runtime_output(
                events,
                context,
                &lgui::window::WindowId::new("terminal-settings-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
            session.render_view(view, viewport, UiScale::ONE);
            session.runtime().run_effects();
        }
        let name_point = Point::new(60.0, 182.0);
        for event in [
            InputEvent::PointerDown {
                pointer: PointerData::mouse(name_point),
                button: PointerButton::Left,
            },
            InputEvent::PointerUp {
                pointer: PointerData::mouse(name_point),
                button: PointerButton::Left,
            },
        ] {
            send(&mut session, &context, &view, viewport, event);
        }
        send(
            &mut session,
            &context,
            &view,
            viewport,
            InputEvent::TextInput("UI shell".into()),
        );
        assert_eq!(
            state.get().shell_editor.as_ref().unwrap().name.text(),
            "UI shell"
        );
        state.update(|app| {
            let draft = app.shell_editor.as_mut().unwrap();
            draft
                .program
                .set_text(std::env::current_exe().unwrap().display().to_string());
            draft.args.set_text("--login");
        });
        session.render_view(&view, viewport, UiScale::ONE);
        let save_point = Point::new(40.0, 58.0 + ROW + 38.0 + 224.0);
        for event in [
            InputEvent::PointerDown {
                pointer: PointerData::mouse(save_point),
                button: PointerButton::Left,
            },
            InputEvent::PointerUp {
                pointer: PointerData::mouse(save_point),
                button: PointerButton::Left,
            },
        ] {
            send(&mut session, &context, &view, viewport, event);
        }
        assert_eq!(state.get().terminal_profiles.len(), 1);
        let id = state.get().terminal_profiles[0].id;
        state.update(|app| app.default_shell = ShellKind::Custom(id));
        session.render_view(&view, viewport, UiScale::ONE);
        let index = state
            .get()
            .shell_options
            .iter()
            .position(|p| p.kind == ShellKind::Custom(id))
            .unwrap();
        let delete_point = Point::new(680.0, 58.0 + index as f32 * ROW + 20.0);
        for event in [
            InputEvent::PointerDown {
                pointer: PointerData::mouse(delete_point),
                button: PointerButton::Left,
            },
            InputEvent::PointerUp {
                pointer: PointerData::mouse(delete_point),
                button: PointerButton::Left,
            },
        ] {
            send(&mut session, &context, &view, viewport, event);
        }
        assert!(state.get().terminal_profiles.is_empty());
        assert_ne!(state.get().default_shell, ShellKind::Custom(id));
    }
}
