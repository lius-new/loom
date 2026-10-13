//! Terminal choices and editable user profiles on the Settings page.

use crate::state::AppState;
use crate::terminal_profiles::{ShellProfile, format_arguments, parse_arguments, resolve_program};
use crate::terminal_session::ShellKind;
use crate::theme;
use crate::ui::components::input::{self, InputBinding, InputOptions, InputState, InputStyle};
use crate::ui::tabs::{TEXT_MARGIN, ellipsize, measure};
use lgui::core::{CursorIcon, EventPolicy};
use lgui::prelude::{
    Element, State, TextAlign, UiRect, VisualStyle, component, group, panel, text,
};
use std::path::PathBuf;

/// Label and description, matching the other settings rows.
const HEADER: f32 = 58.0;
/// Space below the terminal list before the next setting.
const FOOTER: f32 = 20.0;
const ROW: f32 = 48.0;
const RADIO: f32 = 14.0;
const BUTTON_H: f32 = 28.0;
const BUTTON_PAD_X: f32 = 12.0;
const ADD_LABEL: &str = "Add terminal";
const LABEL_W: f32 = 96.0;
const FORM_TITLE_H: f32 = 44.0;
const FIELD_H: f32 = 30.0;
const FIELD_STRIDE: f32 = 40.0;
const FORM_H: f32 = FORM_TITLE_H + FIELD_STRIDE * 2.0 + FIELD_H + 14.0 + BUTTON_H + 16.0;

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
    HEADER + card_height(app) + FOOTER
}

fn card_height(app: &AppState) -> f32 {
    let editing = app.shell_editor.as_ref().filter(|e| e.open);
    let rows = app.shell_options.len().max(1) as f32 * ROW;
    let form = match editing {
        Some(editor) if editor.id.is_none() => FORM_H,
        // An edited row is replaced by its form.
        Some(_) => FORM_H - ROW,
        None => 0.0,
    };
    rows + form + 2.0
}

/// The current project's folder name and the shell its project settings
/// choose, when they choose one.
fn project_shell(app: &AppState) -> Option<(String, String)> {
    let root = crate::key_actions::project_settings_root(app)?;
    let (_, project) = app.project_settings.iter().find(|(r, _)| *r == root)?;
    project.overrides.get("default_shell")?;
    let shell = app.settings_at(Some(&root)).default_shell;
    let folder = root.file_name().map_or_else(
        || root.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    Some((folder, app.shell_name(shell)))
}

pub fn render(rect: UiRect, app: &AppState, state: State<AppState>) -> Element {
    let add_w = measure(ADD_LABEL, theme::UI_SIZE, 400) + BUTTON_PAD_X * 2.0;
    let add_rect = UiRect::new(
        (rect.right - add_w).max(rect.left),
        rect.top + 14.0,
        rect.right,
        rect.top + 14.0 + BUTTON_H,
    );
    let text_right = (add_rect.left - 24.0).max(rect.left);
    let add = state.clone();
    let description = match project_shell(app) {
        Some((folder, shell)) => ellipsize(
            &format!("This is your default. {folder} uses {shell} from its .loom/settings.json."),
            (text_right - rect.left).max(0.0),
            theme::SMALL + 1.0,
            400,
        ),
        None => "Used when a new terminal opens without picking a shell.".to_owned(),
    };
    let root = group(rect)
        .child(text(
            UiRect::new(rect.left, rect.top + 11.0, text_right, rect.top + 29.0),
            "Default shell",
            theme::sans(theme::c().text, theme::UI_SIZE),
        ))
        .child(text(
            UiRect::new(rect.left, rect.top + 30.0, text_right, rect.top + 48.0),
            description,
            theme::sans(theme::c().text_dim, theme::SMALL + 1.0),
        ))
        .child(button(add_rect, ADD_LABEL, "shell-add", false, move || {
            add.update(|app| edit(app, None))
        }));

    let card = UiRect::new(
        rect.left,
        rect.top + HEADER,
        rect.right,
        rect.top + HEADER + card_height(app),
    );
    let mut list = theme::bordered(card, theme::c().surface, theme::c().border, 6.0, 1.0);
    let inner_left = card.left + 1.0;
    let inner_right = card.right - 1.0;
    let editor = app.shell_editor.as_ref().filter(|e| e.open);
    let mut top = card.top + 1.0;
    let mut first = true;
    let divider = |list: Element, top: f32, first: &mut bool| {
        let list = if *first {
            list
        } else {
            list.child(panel(
                UiRect::new(inner_left, top, inner_right, top + 1.0),
                VisualStyle::filled(theme::c().border),
            ))
        };
        *first = false;
        list
    };
    for option in &app.shell_options {
        list = divider(list, top, &mut first);
        let editing_this = matches!(
            (editor.and_then(|e| e.id), option.kind),
            (Some(id), ShellKind::Custom(kind)) if id == kind
        );
        if editing_this {
            let form = UiRect::new(inner_left, top, inner_right, top + FORM_H);
            list = list.child(editor_form(form, state.clone(), editor.unwrap()));
            top += FORM_H;
        } else {
            let row = UiRect::new(inner_left, top, inner_right, top + ROW);
            list = list.child(shell_row(row, option, app, state.clone()));
            top += ROW;
        }
    }
    if app.shell_options.is_empty() {
        list = list.child(text(
            UiRect::new(inner_left + 16.0, top, inner_right - 16.0, top + ROW),
            "No shell was found. Add a terminal to get started.",
            theme::sans(theme::c().text_dim, theme::UI_SIZE),
        ));
        first = false;
        top += ROW;
    }
    if let Some(editor) = editor.filter(|e| e.id.is_none()) {
        list = divider(list, top, &mut first);
        let form = UiRect::new(inner_left, top, inner_right, top + FORM_H);
        list = list.child(editor_form(form, state, editor));
    }
    root.child(list)
}

fn shell_row(
    row: UiRect,
    option: &crate::terminal_profiles::ShellOption,
    app: &AppState,
    state: State<AppState>,
) -> Element {
    let available = option.available();
    let active = option.kind == app.default_shell;
    let custom = match option.kind {
        ShellKind::Custom(id) => Some(id),
        _ => None,
    };
    let actions_w = if custom.is_some() { 132.0 } else { 0.0 };
    let text_left = row.left + 44.0;
    let text_right = (row.right - actions_w - 16.0).max(text_left);

    let mut item = panel(
        row,
        if active {
            VisualStyle::filled(theme::c().active_line)
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
    item = item.child(radio(
        UiRect::new(
            row.left + 16.0,
            row.top + (ROW - RADIO) / 2.0,
            row.left + 16.0 + RADIO,
            row.top + (ROW + RADIO) / 2.0,
        ),
        active,
        available,
    ));

    // Name, then a status tag that only appears when it says something.
    let name_w = measure(&option.name, theme::UI_SIZE, 400) + TEXT_MARGIN;
    let tag = if !available && active && app.effective_shell() != option.kind {
        Some((
            format!(
                "Unavailable · using {}",
                app.shell_name(app.effective_shell())
            ),
            theme::c().warning,
        ))
    } else if !available {
        Some(("Unavailable".to_owned(), theme::c().warning))
    } else if active {
        Some(("Default".to_owned(), theme::c().accent))
    } else {
        None
    };
    let name_right = (text_left + name_w).min(text_right);
    item = item.child(text(
        UiRect::new(text_left, row.top + 7.0, name_right, row.top + 25.0),
        ellipsize(
            &option.name,
            (name_right - text_left).max(0.0),
            theme::UI_SIZE,
            400,
        ),
        theme::sans(
            if available {
                theme::c().text_bright
            } else {
                theme::c().text_muted
            },
            theme::UI_SIZE,
        ),
    ));
    if let Some((tag, color)) = tag {
        let left = (name_right - TEXT_MARGIN + 4.0).max(text_left);
        item = item.child(text(
            UiRect::new(left, row.top + 8.0, text_right.max(left), row.top + 25.0),
            ellipsize(&tag, (text_right - left).max(0.0), theme::SMALL, 400),
            theme::sans(color, theme::SMALL),
        ));
    }

    let mut detail = option
        .program
        .as_ref()
        .map(|p| crate::ui::display_path(p))
        .unwrap_or_else(|| "Edit the executable path to use this terminal.".into());
    if !option.args.is_empty() {
        detail = format!("{detail}  {}", format_arguments(&option.args));
    }
    item = item.child(text(
        UiRect::new(text_left, row.top + 25.0, text_right, row.top + 42.0),
        ellipsize(
            &detail,
            (text_right - text_left).max(0.0),
            theme::SMALL,
            400,
        ),
        theme::mono(theme::c().text_dim, theme::SMALL),
    ));

    if let Some(id) = custom {
        let edit_state = state.clone();
        let top = row.top + (ROW - BUTTON_H) / 2.0;
        item = item
            .child(button(
                UiRect::new(row.right - 136.0, top, row.right - 80.0, top + BUTTON_H),
                "Edit",
                &format!("shell-edit-{id}"),
                false,
                move || edit_state.update(|app| edit(app, Some(id))),
            ))
            .child(button(
                UiRect::new(row.right - 74.0, top, row.right - 12.0, top + BUTTON_H),
                "Remove",
                &format!("shell-delete-{id}"),
                false,
                move || state.update(|app| remove(app, id)),
            ));
    }
    item
}

fn radio(rect: UiRect, on: bool, enabled: bool) -> Element {
    let ring = if on {
        theme::c().accent
    } else if enabled {
        theme::c().text_dim
    } else {
        theme::c().text_ghost
    };
    let mut radio = theme::bordered(rect, theme::c().surface, ring, RADIO / 2.0, 1.5);
    if on {
        let inset = (RADIO - 6.0) / 2.0;
        radio = radio.child(panel(
            UiRect::new(
                rect.left + inset,
                rect.top + inset,
                rect.right - inset,
                rect.bottom - inset,
            ),
            VisualStyle::filled(theme::c().accent).radius(3.0),
        ));
    }
    radio
}

fn button(
    rect: UiRect,
    label: &'static str,
    key: &str,
    primary: bool,
    click: impl Fn() + Send + Sync + 'static,
) -> Element {
    let mut label_style = theme::sans(
        if primary {
            theme::c().on_accent
        } else {
            theme::c().text
        },
        theme::UI_SIZE,
    );
    label_style.align = TextAlign::Center;
    let frame = if primary {
        panel(rect, VisualStyle::filled(theme::c().accent).radius(4.0))
    } else {
        theme::bordered(rect, theme::c().surface, theme::c().border, 4.0, 1.0)
    };
    frame
        .key(key)
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_click(move |cx: &mut lgui::core::UiEventContext| {
            click();
            cx.stop_propagation();
        })
        .child(text(rect, label, label_style))
}

fn editor_form(rect: UiRect, state: State<AppState>, editor: &ProfileEditor) -> Element {
    let mut root = panel(rect, VisualStyle::filled(theme::c().bg));
    root = root.child(text(
        UiRect::new(
            rect.left + 16.0,
            rect.top + 12.0,
            rect.right - 16.0,
            rect.top + 30.0,
        ),
        if editor.id.is_some() {
            "Edit terminal"
        } else {
            "New terminal"
        },
        theme::sans_semibold(theme::c().text_bright, theme::UI_SIZE),
    ));
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
            "Path or command name, e.g. nu",
            |app| &app.shell_editor.as_ref().unwrap().program,
            |app| &mut app.shell_editor.as_mut().unwrap().program,
        ),
        (
            "Arguments",
            "Optional, e.g. --login \"-c some command\"",
            |app| &app.shell_editor.as_ref().unwrap().args,
            |app| &mut app.shell_editor.as_mut().unwrap().args,
        ),
    ];
    let label_w = (rect.width() * 0.25).clamp(0.0, LABEL_W);
    let input_left = rect.left + 16.0 + label_w;
    let input_right = (rect.right - 16.0).max(input_left);
    for (index, (label, placeholder, read, write)) in fields.into_iter().enumerate() {
        let top = rect.top + FORM_TITLE_H + index as f32 * FIELD_STRIDE;
        root = root.child(text(
            UiRect::new(rect.left + 16.0, top, input_left, top + FIELD_H),
            label,
            theme::sans(theme::c().text_muted, theme::UI_SIZE),
        ));
        let frame = UiRect::new(input_left, top, input_right, top + FIELD_H);
        let input_state = state.clone();
        root = root.child(
            theme::bordered(frame, theme::c().surface, theme::c().border, 4.0, 1.0).child(
                component(
                    UiRect::new(
                        frame.left + 4.0,
                        frame.top + 1.0,
                        (frame.right - 4.0).max(frame.left + 4.0),
                        frame.bottom - 1.0,
                    ),
                    move |cx, &rect| {
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
                                    placeholder: theme::mono(theme::c().text_faint, theme::UI_SIZE),
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
                    },
                )
                .key(format!("shell-profile-input-{index}")),
            ),
        );
    }

    let footer = rect.bottom - 16.0 - BUTTON_H;
    let save_rect = UiRect::new(
        (rect.right - 16.0 - 72.0).max(rect.left),
        footer,
        rect.right - 16.0,
        footer + BUTTON_H,
    );
    let cancel_rect = UiRect::new(
        (save_rect.left - 8.0 - 72.0).max(rect.left),
        footer,
        save_rect.left - 8.0,
        footer + BUTTON_H,
    );
    if let Some(error) = &editor.error {
        root = root.child(text(
            UiRect::new(
                input_left,
                footer,
                (cancel_rect.left - 12.0).max(input_left),
                footer + BUTTON_H,
            ),
            ellipsize(
                error,
                (cancel_rect.left - 12.0 - input_left).max(0.0),
                theme::SMALL + 1.0,
                400,
            ),
            theme::sans(theme::c().error_text, theme::SMALL + 1.0),
        ));
    }
    let save = state.clone();
    root.child(button(
        cancel_rect,
        "Cancel",
        "shell-cancel",
        false,
        move || state.update(close_editor),
    ))
    .child(button(save_rect, "Save", "shell-save", true, move || {
        save.update(|app| {
            save_editor(app);
        })
    }))
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
                for pixel in rgba.as_chunks_mut::<4>().0 {
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
        fn click(
            session: &mut UiSession,
            context: &ApplicationContext,
            view: &AppView,
            viewport: UiRect,
            label: &str,
        ) {
            let node = session
                .tree()
                .nodes()
                .iter()
                .find(|n| n.text.as_deref() == Some(label))
                .unwrap_or_else(|| panic!("missing label {label}"));
            let point = Point::new(
                (node.layout_rect.left + node.layout_rect.right) * 0.5,
                (node.layout_rect.top + node.layout_rect.bottom) * 0.5,
            );
            for event in [
                InputEvent::PointerDown {
                    pointer: PointerData::mouse(point),
                    button: PointerButton::Left,
                },
                InputEvent::PointerUp {
                    pointer: PointerData::mouse(point),
                    button: PointerButton::Left,
                },
            ] {
                send(session, context, view, viewport, event);
            }
        }
        click(&mut session, &context, &view, viewport, "My shell");
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
        click(&mut session, &context, &view, viewport, "Save");
        assert_eq!(state.get().terminal_profiles.len(), 1);
        let id = state.get().terminal_profiles[0].id;
        state.update(|app| app.default_shell = ShellKind::Custom(id));
        session.render_view(&view, viewport, UiScale::ONE);
        // Editing replaces the row with its form; the form's title confirms it.
        click(&mut session, &context, &view, viewport, "Edit");
        assert!(
            session
                .tree()
                .nodes()
                .iter()
                .any(|n| n.text.as_deref() == Some("Edit terminal"))
        );
        click(&mut session, &context, &view, viewport, "Cancel");
        click(&mut session, &context, &view, viewport, "Remove");
        assert!(state.get().terminal_profiles.is_empty());
        assert_ne!(state.get().default_shell, ShellKind::Custom(id));
    }
}
