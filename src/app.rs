//! Root component: owns `AppState`, computes layout regions, wires the global
//! keymap, and composes every UI module. No module talks to another directly —
//! all cross-cutting state lives in `AppState`.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use lgui::ApplicationHandle;
use lgui::core::{KeyboardEvent, UiEventKind, UiEventPayload};
use lgui::prelude::{Element, RenderCx, UiRect, group};
use lgui::window::WindowFocusChanged;

use crate::editor::editor_view;
use crate::git::GitStoreSnapshot;
use crate::input::keymap;
use crate::state::AppState;
use crate::terminal_session::{ShellKind, TerminalTabs};
use crate::theme;
use crate::ui::{
    clone_repository, command_palette, context_menu, diff_editor, git_panel, sidebar, statusbar,
    tabs, terminal, titlebar, toast,
};
use crate::window_geometry;
use crate::workspace_persistence;

pub fn app(cx: &mut RenderCx<'_, '_>) -> Element {
    let state = cx.state_with(AppState::restored);
    let git_store = cx.state_with(GitStoreSnapshot::default);
    let terminal_tabs = cx.state_with(|| {
        let session = workspace_persistence::load();
        TerminalTabs::restored(session.terminal_tabs, session.active_terminal)
    });
    let terminal_cursor_blink = cx.state(true);
    let terminal_cursor_visible = terminal_cursor_blink.get();
    let terminal_tab_snapshot = terminal_tabs.get();
    let active_terminal_id = terminal_tab_snapshot.active_id();
    let terminal_shells = terminal_tab_snapshot.shells();
    let active_terminal_index = terminal_tab_snapshot.active_index();
    let terminal_controller = terminal_tab_snapshot
        .active()
        .map(|tab| tab.controller.clone());
    let application = cx.application().resource::<ApplicationHandle>();
    let editor_id = cx.use_stable_id();
    let editor_focus = cx.focus_handle(editor_id.clone());
    let terminal_id = cx.use_stable_id();
    let terminal_focus = cx.focus_handle(terminal_id.clone());
    let clone_input_id = cx.use_stable_id();
    let clone_input_focus = cx.focus_handle(clone_input_id.clone());
    let commit_input_id = cx.use_stable_id();
    let commit_input_focus = cx.focus_handle(commit_input_id.clone());
    let vp = cx.viewport();
    let w = vp.width();
    let h = vp.height();
    window_geometry::observe_viewport(w, h);
    let window_focus_state = state.clone();
    let window_focus_git_store = git_store.clone();
    cx.use_event_once::<WindowFocusChanged>(move |event| {
        if event.window_id.as_str() == "loom" {
            window_geometry::handle_focus_change(event.focused);
            if !event.focused {
                window_focus_state.try_update(editor_view::finish_scrollbar_drag);
            } else {
                window_focus_state.update(|app| {
                    app.workspace.reconcile_disk();
                });
                crate::git_actions::refresh(&window_focus_state, &window_focus_git_store);
            }
        }
    });
    cx.use_effect((), || {
        window_geometry::remember_native_window();
        || {}
    });

    // Snapshot toggles for this frame.
    let s = state.get();
    let open_tab_paths = s.workspace.open_paths();
    let active_tab_path = s.workspace.active_path().map(std::path::Path::to_path_buf);
    let git_roots = s.workspace_folders.clone();
    let git_active_path = active_tab_path.clone();
    let polling_store = git_store.clone();
    cx.use_effect((git_roots.clone(), git_active_path.clone()), move || {
        let handle = crate::git::start_polling_async(
            git_roots,
            git_active_path,
            Duration::from_millis(2_000),
            move |snapshot| {
                polling_store.update(move |current| {
                    current.replace_if_newer(snapshot);
                })
            },
        );
        move || drop(handle)
    });
    cx.use_effect(
        (open_tab_paths.clone(), active_tab_path.clone()),
        move || {
            let _ =
                workspace_persistence::save_file_tabs(&open_tab_paths, active_tab_path.as_deref());
            || {}
        },
    );
    let document_state = state.clone();
    cx.use_effect(s.workspace.active(), move || {
        document_state.update(|app| {
            app.editor.drag = None;
            app.editor.menu = None;
            app.editor.preedit.clear();
            app.editor.preedit_cursor = None;
            app.editor.ime_pending = false;
        });
    });
    let show_term = s.show_terminal;
    cx.use_effect(
        (
            terminal_shells.clone(),
            active_terminal_index,
            show_term,
            s.terminal_h,
        ),
        move || {
            let _ = workspace_persistence::save_terminal_state(
                &terminal_shells,
                active_terminal_index,
                show_term,
                s.terminal_h,
            );
            || {}
        },
    );
    let show_drawer = s.show_drawer;
    let git_ui_preferences = (
        s.show_source_control,
        s.git_tree_view,
        s.git_split_diff,
        s.git_inline_blame,
    );
    cx.use_effect(git_ui_preferences, move || {
        let _ = workspace_persistence::save_git_ui(
            git_ui_preferences.0,
            git_ui_preferences.1,
            git_ui_preferences.2,
            git_ui_preferences.3,
        );
        || {}
    });
    let show_palette = s.show_palette;
    let show_clone_dialog = s.show_clone_dialog;

    // ---- Region layout ------------------------------------------------
    let titlebar_rect = UiRect::new(0.0, 0.0, w, theme::TITLEBAR_H);
    let statusbar_rect = UiRect::new(0.0, h - theme::STATUS_H, w, h);
    let max_terminal_h =
        (h - theme::STATUS_H - theme::TITLEBAR_H - theme::TABS_H - theme::EDITOR_MIN_H)
            .clamp(theme::TERMINAL_MIN_H, theme::TERMINAL_MAX_H);
    let terminal_h = s.terminal_h.clamp(theme::TERMINAL_MIN_H, max_terminal_h);
    let main_bottom = if show_term {
        h - theme::STATUS_H - terminal_h
    } else {
        h - theme::STATUS_H
    };
    let source_control_left = s.show_source_control;
    let explorer_right = show_drawer;
    let editor_left = if source_control_left {
        s.git_sidebar_w
    } else {
        0.0
    };
    // The editor and its overlay scrollbars stop at the drawer's visible edge.
    // This keeps the vertical editor thumb reachable while the drawer is open.
    let tabs_right = if explorer_right { w - s.sidebar_w } else { w };
    let tabs_rect = UiRect::new(
        editor_left,
        theme::TITLEBAR_H,
        tabs_right,
        theme::TITLEBAR_H + theme::TABS_H,
    );
    let code_rect = UiRect::new(
        editor_left,
        theme::TITLEBAR_H + theme::TABS_H,
        tabs_right,
        main_bottom,
    );
    let source_control_rect = UiRect::new(
        0.0,
        theme::TITLEBAR_H,
        s.git_sidebar_w,
        main_bottom,
    );
    let explorer_rect = UiRect::new(w - s.sidebar_w, theme::TITLEBAR_H, w, main_bottom);
    let terminal_rect = UiRect::new(
        0.0,
        h - theme::STATUS_H - terminal_h,
        w,
        h - theme::STATUS_H,
    );
    let terminal_size = terminal::pty_size(terminal_rect);
    let drag_state = state.clone();
    let dragging = s.editor.drag.is_some();
    cx.use_effect((dragging, s.workspace.active(), code_rect), move || {
        let (sender, receiver) = mpsc::channel();
        let worker = if dragging {
            thread::Builder::new()
                .name("loom-selection-scroll".into())
                .spawn(move || {
                    while let Err(RecvTimeoutError::Timeout) =
                        receiver.recv_timeout(Duration::from_millis(30))
                    {
                        drag_state.try_update(|app| editor_view::drag_scroll_tick(app, code_rect));
                    }
                })
                .ok()
        } else {
            None
        };
        move || {
            let _ = sender.send(());
            if let Some(worker) = worker {
                let _ = worker.join();
            }
        }
    });
    let terminal_cwd = s
        .workspace_folders
        .first()
        .cloned()
        .or_else(|| std::env::current_dir().ok());

    let terminal_start = terminal_controller.clone();
    let terminal_start_application = application.clone();
    let start_cwd = terminal_cwd.clone();
    cx.use_effect(
        (
            show_term,
            active_terminal_id,
            terminal_size,
            terminal_cwd.clone(),
        ),
        move || {
            if show_term && let Some(terminal_start) = terminal_start.as_ref() {
                terminal_start.ensure_started(
                    start_cwd.as_deref(),
                    terminal_size,
                    terminal_start_application,
                );
            }
        },
    );
    let mounted_terminal_focus = terminal_focus.clone();
    cx.use_effect(show_term, move || {
        if show_term {
            mounted_terminal_focus.focus();
        }
    });
    let mounted_clone_focus = clone_input_focus.clone();
    cx.use_effect(show_clone_dialog, move || {
        if show_clone_dialog {
            mounted_clone_focus.focus();
        }
    });
    let blink_focused = s.terminal_focused;
    let blink_state = terminal_cursor_blink.clone();
    cx.use_effect((show_term, blink_focused, active_terminal_id), move || {
        blink_state.set(true);
        let (stop_sender, stop_receiver) = mpsc::channel();
        let worker = if show_term && blink_focused {
            let worker_state = blink_state.clone();
            thread::Builder::new()
                .name("loom-terminal-cursor-blink".to_string())
                .spawn(move || {
                    while let Err(RecvTimeoutError::Timeout) =
                        stop_receiver.recv_timeout(Duration::from_millis(500))
                    {
                        worker_state.update(|visible| *visible = !*visible);
                    }
                })
                .ok()
        } else {
            None
        };

        move || {
            let _ = stop_sender.send(());
            if let Some(worker) = worker {
                let _ = worker.join();
            }
        }
    });

    // ---- Root (global shortcut listener) ------------------------------
    let mut root = group(vp);
    for kind in [UiEventKind::KeyDown, UiEventKind::KeyUp] {
        let modifiers_state = state.clone();
        root = root.on_event_capture(kind, move |_, payload| {
            if let UiEventPayload::Keyboard { event } = payload {
                modifiers_state.try_update(|app| {
                    let mut modifiers = event.modifiers;
                    // Winit can report the modifier transition after the key event itself.
                    if event.key == lgui::core::LogicalKey::Named(lgui::core::NamedKey::Shift) {
                        modifiers.set(
                            lgui::core::KeyModifiers::SHIFT,
                            event.state == lgui::core::KeyState::Down,
                        );
                    }
                    let changed = app.editor.modifiers != modifiers;
                    app.editor.modifiers = modifiers;
                    changed
                });
            }
        });
    }
    let interrupt_state = state.clone();
    let interrupt_controller = terminal_controller.clone();
    let interrupt_application = application.clone();
    root = root.on_event_capture(UiEventKind::KeyDown, move |ctx, payload| {
        let UiEventPayload::Keyboard { event } = payload else {
            return;
        };
        let is_ctrl_c = event.state == lgui::core::KeyState::Down
            && event.modifiers.ctrl()
            && matches!(
                &event.key,
                lgui::core::LogicalKey::Character(value) if value.eq_ignore_ascii_case("c")
            );
        if interrupt_state.get().terminal_focused
            && is_ctrl_c
            && let Some(controller) = interrupt_controller.as_ref()
        {
            controller.write(b"\x03");
            interrupt_application.request_frame();
            ctx.prevent_default();
            ctx.stop_propagation();
        }
    });
    let st = state.clone();
    let global_editor_focus = editor_focus.clone();
    let global_terminal_focus = terminal_focus.clone();
    let global_terminal_tabs = terminal_tabs.clone();
    root = root.on_key_down(move |ctx, ev: &KeyboardEvent| {
        if let Some(action) = keymap::action_for(ev) {
            ctx.prevent_default();
            if action == keymap::Action::ToggleTerminal {
                let opening = !st.get().show_terminal;
                if opening && global_terminal_tabs.get().is_empty() {
                    global_terminal_tabs.update(|tabs| {
                        tabs.add(ShellKind::default());
                    });
                }
                st.update(move |app| app.apply(action));
                if opening {
                    global_terminal_focus.focus();
                } else {
                    global_editor_focus.focus();
                }
                return;
            }
            st.update(move |app| app.apply(action));
        }
    });

    // Track the pointer against the whole drawer rather than individual tree
    // rows. Capturing at the root also observes moves into sibling regions and
    // overlays, so the scrollbar thumb disappears as soon as the drawer is
    // left.
    let st_hover = state.clone();
    root = root.on_event_capture(UiEventKind::PointerMove, move |_ctx, payload| {
        if let UiEventPayload::PointerMove { pointer } = payload {
            let hovered = show_drawer && explorer_rect.contains(pointer.point);
            st_hover.try_update(move |app| {
                let changed =
                    app.sidebar_hovered != hovered || (!hovered && app.tree_hovered_path.is_some());
                app.sidebar_hovered = hovered;
                if !hovered {
                    app.tree_hovered_path = None;
                }
                changed
            });
        }
    });

    // Keep Source Control's overlay scrollbar behavior aligned with Explorer:
    // it is visible while the pointer is inside the drawer or while its thumb
    // is being dragged.
    let st_git_hover = state.clone();
    root = root.on_event_capture(UiEventKind::PointerMove, move |_ctx, payload| {
        if let UiEventPayload::PointerMove { pointer } = payload {
            let hovered = source_control_left && source_control_rect.contains(pointer.point);
            st_git_hover.try_update(move |app| {
                let changed = app.git_sidebar_hovered != hovered;
                app.git_sidebar_hovered = hovered;
                changed
            });
        }
    });

    let st_editor_hover = state.clone();
    root = root.on_event_capture(UiEventKind::PointerMove, move |_ctx, payload| {
        if let UiEventPayload::PointerMove { pointer } = payload {
            let hovered = code_rect.contains(pointer.point);
            st_editor_hover.try_update(move |app| {
                let changed =
                    app.editor_hovered != hovered || (!hovered && app.welcome_hover.is_some());
                app.editor_hovered = hovered;
                if !hovered {
                    app.welcome_hover = None;
                }
                changed
            });
        }
    });

    // Keep editor scrollbar drags alive while the pointer is anywhere in the
    // window. Relying only on the narrow thumb/track as the drag source makes
    // the interaction fragile once the pointer leaves that hit region.
    let st_editor_scrollbar_drag = state.clone();
    root = root.on_event_capture(UiEventKind::PointerMove, move |_ctx, payload| {
        if let UiEventPayload::PointerMove { pointer } = payload {
            st_editor_scrollbar_drag.try_update(|app| {
                editor_view::drag_scrollbars(app, code_rect, pointer.point.x, pointer.point.y)
            });
        }
    });
    let st_editor_scrollbar_up = state.clone();
    root = root.on_event_capture(UiEventKind::PointerUp, move |_ctx, _payload| {
        st_editor_scrollbar_up.try_update(editor_view::finish_scrollbar_drag);
    });

    // ---- Compose chrome (back-to-front) -------------------------------
    root = root.child(titlebar::render(titlebar_rect));

    root = root.child(tabs::render(tabs_rect, state.clone(), editor_focus.clone()));
    if s.workspace.active_diff().is_some() {
        root = root.child(diff_editor::render(
            code_rect,
            state.clone(),
            editor_focus.clone(),
        ));
    } else {
        root = root.child(editor_view::render(
            code_rect,
            state.clone(),
            git_store.clone(),
            editor_id,
            editor_focus.clone(),
        ));
    }

    if source_control_left {
        root = root.child(git_panel::render(
            source_control_rect,
            state.clone(),
            git_store.clone(),
            editor_focus.clone(),
            commit_input_focus,
            commit_input_id,
        ));
    }
    if show_drawer {
        root = root.child(sidebar::render(
            explorer_rect,
            state.clone(),
            git_store.clone(),
            editor_focus.clone(),
        ));
    }

    if show_term && let Some(terminal_controller) = terminal_controller {
        root = root.child(terminal::render(
            terminal_rect,
            state.clone(),
            editor_focus.clone(),
            terminal_focus.clone(),
            terminal_id,
            terminal_controller,
            terminal_tabs.clone(),
            application,
            terminal_cursor_visible,
            max_terminal_h,
        ));
    }

    root = root.child(statusbar::render(
        statusbar_rect,
        state.clone(),
        git_store.clone(),
        editor_focus.clone(),
        terminal_focus,
        terminal_tabs,
    ));

    // ---- Overlays ------------------------------------------------------
    if let Some(pos) = s.editor.menu {
        root = root.child(crate::editor::edit_menu::render(
            vp,
            code_rect,
            pos,
            state.clone(),
            editor_focus.clone(),
        ));
    }
    if show_palette {
        root = root.child(command_palette::render(vp, state.clone(), editor_focus));
    }
    if let Some(pos) = s.context_menu {
        root = root.child(context_menu::render(vp, pos, state.clone()));
    }
    if show_clone_dialog {
        root = root.child(clone_repository::render(
            vp,
            state.clone(),
            clone_input_focus,
            clone_input_id,
        ));
    }
    root = root.child(toast::render(vp, state.clone()));

    root
}
