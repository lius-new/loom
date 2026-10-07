//! Root component: owns `AppState`, computes layout regions, wires the global
//! keymap, and composes every UI module. No module talks to another directly —
//! all cross-cutting state lives in `AppState`.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use lgui::ApplicationHandle;
use lgui::core::{KeyboardEvent, UiEventKind, UiEventPayload};
use lgui::prelude::{Element, RenderCx, UiRect, group};
use lgui::services::ServicesContextExt;
use lgui::window::WindowFocusChanged;

use crate::editor::editor_view;
use crate::git::GitStoreSnapshot;
use crate::input::keymap;
use crate::settings_persistence::{self, Settings};
use crate::state::{AppState, CloseContinuation, CloseRequest, MainSurface};
use crate::terminal_session::TerminalTabs;
use crate::theme;
use crate::ui::{
    clone_repository, close_confirmation, context_menu, diff_editor, git_panel, settings, sidebar,
    statusbar, tab_context_menu, tabs, terminal, titlebar, toast, welcome, workspace_home,
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
    let active_terminal_cwd = terminal_tab_snapshot
        .active()
        .and_then(|tab| tab.cwd.clone());
    let terminal_controller = terminal_tab_snapshot
        .active()
        .map(|tab| tab.controller.clone());
    let application_context = cx.application();
    let application = application_context.resource::<ApplicationHandle>();
    let window_manager = application_context.windows();
    // Welcome, Workspace Home and the editor share one stable identity so
    // focus survives transitions between central surfaces.
    let main_surface_id = cx.use_stable_id();
    let editor_focus = cx.focus_handle(main_surface_id.clone());
    let terminal_id = cx.use_stable_id();
    let terminal_focus = cx.focus_handle(terminal_id.clone());
    let clone_input_id = cx.use_stable_id();
    let clone_input_focus = cx.focus_handle(clone_input_id.clone());
    let explorer_create_input_id = cx.use_stable_id();
    let explorer_create_input_focus = cx.focus_handle(explorer_create_input_id.clone());
    let commit_input_id = cx.use_stable_id();
    let commit_input_focus = cx.focus_handle(commit_input_id.clone());
    let close_dialog_id = cx.use_stable_id();
    let close_dialog_focus = cx.focus_handle(close_dialog_id.clone());
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
                window_focus_state.try_update(|app| {
                    editor_view::finish_scrollbar_drag(app) | tabs::cancel_pointer_interaction(app)
                });
            } else {
                window_focus_state.update(|app| {
                    crate::file_tree::refresh_all_loaded_directories(app);
                    app.workspace.reconcile_disk();
                });
                crate::git_actions::refresh(&window_focus_state, &window_focus_git_store);
            }
        }
    });
    let close_state = state.clone();
    cx.use_event_once::<window_geometry::AppCloseRequested>(move |_| {
        let dirty_targets = close_state.get().workspace.dirty_file_ids();
        if dirty_targets.is_empty() {
            window_manager.close("loom");
        } else {
            close_state.update(move |app| {
                app.close_request = Some(CloseRequest {
                    targets: dirty_targets,
                    continuation: CloseContinuation::ExitApplication,
                });
                app.tab_context_menu = None;
                app.show_clone_dialog = false;
                app.editor.menu = None;
            });
        }
    });
    cx.use_effect((), || {
        window_geometry::remember_native_window();
        || {}
    });

    // Snapshot toggles for this frame.
    let s = state.get();
    theme::set(s.theme);
    let open_tab_paths = s.workspace.persistent_open_paths();
    let persistent_active_tab_path = s
        .workspace
        .persistent_active_path()
        .map(std::path::Path::to_path_buf);
    let active_tab_path = s.workspace.active_path().map(std::path::Path::to_path_buf);
    let git_roots = s.workspace_folders.clone();
    let git_active_path = active_tab_path.clone();
    let file_watch_state = state.clone();
    let file_watch_roots = git_roots.clone();
    cx.use_effect(file_watch_roots.clone(), move || {
        let handle = if file_watch_roots.is_empty() {
            None
        } else {
            let batch_state = file_watch_state.clone();
            match crate::file_watcher::start(file_watch_roots, move |batch| {
                batch_state
                    .try_update(move |app| crate::file_watcher::apply_batch(app, &batch));
            }) {
                Ok(handle) => Some(handle),
                Err(error) => {
                    file_watch_state.update(move |app| {
                        app.show_error(format!(
                            "File watching is unavailable; Explorer will refresh when Loom regains focus: {error}"
                        ));
                    });
                    None
                }
            }
        };
        move || drop(handle)
    });
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
        (open_tab_paths.clone(), persistent_active_tab_path.clone()),
        move || {
            let _ = workspace_persistence::save_file_tabs(
                &open_tab_paths,
                persistent_active_tab_path.as_deref(),
            );
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
    let source_control_open = s.show_source_control;
    cx.use_effect(source_control_open, move || {
        let _ = workspace_persistence::save_source_control_open(source_control_open);
        || {}
    });
    let settings = Settings {
        theme: s.theme,
        default_shell: s.default_shell,
        terminal_cursor_blink: s.terminal_cursor_blink,
        git_inline_blame: s.git_inline_blame,
        git_split_diff: s.git_split_diff,
        git_tree_view: s.git_tree_view,
    };
    cx.use_effect(settings, move || {
        let _ = settings_persistence::save(&settings);
        || {}
    });
    let show_clone_dialog = s.show_clone_dialog;
    let editing_explorer_entry = s.explorer_create.is_some() || s.explorer_rename.is_some();

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
    let tab_visibility_state = state.clone();
    let tab_visibility_items = s
        .workspace
        .open_files()
        .iter()
        .filter_map(|id| {
            s.workspace.meta(*id).map(|meta| {
                (
                    *id,
                    meta.name.clone(),
                    s.workspace.is_dirty(*id),
                    s.workspace.is_diff(*id),
                    s.workspace.has_disk_conflict(*id),
                    s.workspace.is_missing_on_disk(*id),
                )
            })
        })
        .collect::<Vec<_>>();
    let tab_visibility_key = (
        tab_visibility_items,
        s.workspace.active(),
        tabs_rect.width(),
        s.show_drawer,
    );
    cx.use_effect(tab_visibility_key, move || {
        let desired = tabs::active_visible_scroll(tabs_rect, &tab_visibility_state.get());
        tab_visibility_state.try_update(move |app| {
            if (app.tab_scroll_x - desired).abs() <= f32::EPSILON {
                return false;
            }
            app.tab_scroll_x = desired;
            true
        });
        || {}
    });
    let code_rect = UiRect::new(
        editor_left,
        theme::TITLEBAR_H + theme::TABS_H,
        tabs_right,
        main_bottom,
    );
    let source_control_rect = UiRect::new(0.0, theme::TITLEBAR_H, s.git_sidebar_w, main_bottom);
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
    let terminal_cwd = active_terminal_cwd.or_else(|| {
        s.workspace_folders
            .first()
            .cloned()
            .or_else(|| std::env::current_dir().ok())
    });

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
    cx.use_effect((show_term, active_terminal_id), move || {
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
    // The close confirmation takes keyboard focus while open so Enter/Esc go
    // to the dialog rather than the editor; it hands focus back on resolve.
    let showing_close_request = s.close_request.is_some();
    let mounted_close_focus = close_dialog_focus.clone();
    cx.use_effect(showing_close_request, move || {
        if showing_close_request {
            mounted_close_focus.focus();
        }
    });
    let mounted_create_focus = explorer_create_input_focus.clone();
    let finished_create_focus = editor_focus.clone();
    let pending_tree_state = state.clone();
    cx.use_effect(editing_explorer_entry, move || {
        if editing_explorer_entry {
            mounted_create_focus.focus();
        } else {
            pending_tree_state.try_update(crate::file_tree::apply_pending_refresh);
            finished_create_focus.focus();
        }
    });
    // Each toast id gets its own timer; a newer toast restarts it, and
    // `dismiss_toast` ignores a timer that outlived its toast.
    let toast_timer = s.toast.as_ref().map(|toast| (toast.id, toast.kind));
    let toast_state = state.clone();
    cx.use_effect(toast_timer, move || {
        let (stop_sender, stop_receiver) = mpsc::channel::<()>();
        let worker = toast_timer.and_then(|(id, kind)| {
            thread::Builder::new()
                .name("loom-toast-timer".to_string())
                .spawn(move || {
                    if let Err(RecvTimeoutError::Timeout) =
                        stop_receiver.recv_timeout(kind.duration())
                    {
                        toast_state.try_update(|app| app.dismiss_toast(id));
                    }
                })
                .ok()
        });
        move || {
            let _ = stop_sender.send(());
            if let Some(worker) = worker {
                let _ = worker.join();
            }
        }
    });
    let blink_focused = s.terminal_focused && s.terminal_cursor_blink;
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
        // Clipboard keys are captured here so editor-wide shortcuts never see them.
        if interrupt_state.get().terminal_focused
            && let Some(controller) = interrupt_controller.as_ref()
            && let Some(result) = terminal::handle_clipboard_shortcut(
                controller,
                event,
                ctx.application().clipboard().as_ref(),
            )
        {
            terminal::report_clipboard_error(&interrupt_state, result);
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
                terminal::toggle_panel(
                    &st,
                    &global_terminal_tabs,
                    &global_editor_focus,
                    &global_terminal_focus,
                );
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
                let changed = app.editor_hovered != hovered
                    || (!hovered
                        && (app.welcome_hover.is_some() || app.workspace_home_hover.is_some()));
                app.editor_hovered = hovered;
                if !hovered {
                    app.welcome_hover = None;
                    app.workspace_home_hover = None;
                }
                changed
            });
        }
    });

    let st_tabs_pointer = state.clone();
    root = root.on_event_capture(UiEventKind::PointerMove, move |_ctx, payload| {
        if let UiEventPayload::PointerMove { pointer } = payload {
            st_tabs_pointer.try_update(|app| {
                tabs::update_pointer(app, tabs_rect, pointer.point.x, pointer.point.y, false)
            });
        }
    });

    let st_tabs_drag = state.clone();
    root = root.on_event_capture(UiEventKind::PointerDrag, move |_ctx, payload| {
        if let UiEventPayload::PointerDrag { pointer } = payload {
            st_tabs_drag.try_update(|app| {
                tabs::update_pointer(app, tabs_rect, pointer.point.x, pointer.point.y, true)
            });
        }
    });

    let st_tabs_pointer_up = state.clone();
    root = root.on_event_capture(UiEventKind::PointerUp, move |_ctx, _payload| {
        st_tabs_pointer_up.try_update(tabs::finish_pointer_interaction);
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
    root = root.child(titlebar::render(titlebar_rect, state.clone()));

    root = root.child(tabs::render(tabs_rect, state.clone(), editor_focus.clone()));
    root = root.child(match s.main_surface() {
        MainSurface::Settings => settings::render(code_rect, state.clone()),
        MainSurface::Diff => diff_editor::render(code_rect, state.clone(), editor_focus.clone()),
        MainSurface::Editor => editor_view::render(
            code_rect,
            state.clone(),
            git_store.clone(),
            main_surface_id.clone(),
            editor_focus.clone(),
        ),
        MainSurface::WorkspaceHome => workspace_home::render(
            code_rect,
            state.clone(),
            main_surface_id.clone(),
            editor_focus.clone(),
            terminal_focus.clone(),
            terminal_tabs.clone(),
        ),
        MainSurface::Welcome => welcome::render(
            code_rect,
            state.clone(),
            main_surface_id.clone(),
            editor_focus.clone(),
        ),
    });

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
            explorer_create_input_focus,
            explorer_create_input_id,
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
        terminal_tabs.clone(),
    ));

    if s.context_menu.is_none()
        && s.tab_context_menu.is_none()
        && !show_clone_dialog
        && s.close_request.is_none()
        && let Some(tooltip) = tabs::render_tooltip(vp, tabs_rect, &s)
    {
        root = root.child(tooltip);
    }

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
    if let Some(pos) = s.context_menu {
        root = root.child(context_menu::render(
            vp,
            pos,
            state.clone(),
            terminal_tabs.clone(),
        ));
    }
    if s.tab_context_menu.is_some() {
        root = root.child(tab_context_menu::render(vp, state.clone()));
    }
    if show_clone_dialog {
        root = root.child(clone_repository::render(
            vp,
            state.clone(),
            clone_input_focus,
            clone_input_id,
        ));
    }
    if s.close_request.is_some() {
        root = root.child(close_confirmation::render(
            vp,
            state.clone(),
            close_dialog_id,
            editor_focus.clone(),
        ));
    }
    root = root.child(toast::render(vp, state.clone()));

    root
}
