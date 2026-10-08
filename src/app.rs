//! Root component: owns `AppState`, computes layout regions, wires the global
//! keymap, and composes every UI module. No module talks to another directly —
//! all cross-cutting state lives in `AppState`.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use lgui::ApplicationHandle;
use lgui::core::{UiElement, UiEventKind, UiEventPayload, UiId};
use lgui::prelude::{Element, RenderCx, UiRect, VisualStyle, group};
use lgui::window::WindowFocusChanged;

use crate::editor::editor_view;
use crate::git::GitStoreSnapshot;
use crate::key_actions::{self, KeyEnv, TerminalEnv};
use crate::settings_persistence::{self, Settings};
use crate::state::{AppState, CloseContinuation, CloseRequest, MainSurface, TabDrop};
use crate::terminal_session::TerminalTabs;
use crate::theme;
use crate::ui::{
    clone_repository, close_confirmation, context_menu, diff_editor, git_panel, keymap_page,
    pane_sash, settings, sidebar, statusbar, tab_context_menu, tabs, terminal, titlebar, toast,
    welcome, workspace_home,
};
use crate::window_geometry;
use crate::workspace_persistence;

pub fn app(cx: &mut RenderCx<'_, '_>) -> Element {
    let state = cx.state_with(AppState::restored);
    let git_store = cx.state_with(GitStoreSnapshot::default);
    let git_poll_control = cx.state_with(crate::git::PollControl::default);
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
    let keymap_search_id = cx.use_stable_id();
    let keymap_search_focus = cx.focus_handle(keymap_search_id.clone());
    let vp = cx.viewport();
    let w = vp.width();
    let h = vp.height();
    window_geometry::observe_viewport(w, h);
    let window_focus_state = state.clone();
    let window_focus_git_poll = git_poll_control.get();
    cx.use_event_once::<WindowFocusChanged>(move |event| {
        if event.window_id.as_str() == "loom" {
            window_geometry::handle_focus_change(event.focused);
            // A background window does not poll; regaining focus rescans.
            window_focus_git_poll.set_paused(!event.focused);
            if !event.focused {
                window_focus_state.try_update(|app| {
                    // A key sequence does not survive leaving the window.
                    let had_pending = !app.pending_keystrokes.is_empty();
                    app.pending_keystrokes.clear();
                    editor_view::finish_scrollbar_drag(app)
                        | tabs::cancel_pointer_interaction(app)
                        | had_pending
                });
            } else {
                window_focus_state.update(|app| {
                    crate::file_tree::refresh_all_loaded_directories(app);
                    app.workspace.reconcile_disk();
                });
            }
        }
    });
    let close_state = state.clone();
    cx.use_event_once::<window_geometry::AppCloseRequested>(move |_| {
        let workspace = close_state.get().workspace;
        let dirty_targets = workspace
            .dirty_file_ids()
            .into_iter()
            .flat_map(|id| workspace.tabs_of(id))
            .collect::<Vec<_>>();
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
    let editor_layout = crate::workspace_actions::saved_editor_layout(&s.workspace);
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
    // Switching tabs only retargets line decorations; the poller keeps running.
    let git_poll = git_poll_control.get();
    let poll_target = git_poll.clone();
    cx.use_effect(git_active_path.clone(), move || {
        poll_target.set_active_path(git_active_path);
        || {}
    });
    let polling_store = git_store.clone();
    cx.use_effect(git_roots.clone(), move || {
        let handle = crate::git::start_polling(
            git_roots,
            git_poll,
            Duration::from_millis(2_000),
            move |result| polling_store.update(move |current| current.apply_scan(result)),
        );
        move || drop(handle)
    });
    cx.use_effect(editor_layout.clone(), move || {
        let _ = workspace_persistence::save_editor_layout(editor_layout.as_ref());
        || {}
    });
    let document_state = state.clone();
    let active_view = (s.workspace.active_pane(), s.workspace.active());
    cx.use_effect(active_view, move || {
        document_state.update(|app| {
            app.editor.drag = None;
            app.editor.menu = None;
            // Leaving a view ends its Vim insert and returns it to Normal mode.
            crate::editor::vim_input::sync_view(app);
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
        vim: s.vim.options.into(),
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
    // Each pane is a tab strip above its surface; the focused pane's rects
    // drive the editor-wide handlers (selection auto-scroll, menus, keys).
    let editor_area = UiRect::new(editor_left, theme::TITLEBAR_H, tabs_right, main_bottom);
    let focused_pane = s.workspace.active_pane();
    let pane_rects = s.workspace.layout().layout(editor_area);
    let strips = pane_rects
        .iter()
        .map(|(pane, rect)| (*pane, pane_strip_rect(*rect)))
        .collect::<Vec<_>>();
    let bodies = pane_rects
        .iter()
        .map(|(pane, rect)| (*pane, pane_body_rect(*rect)))
        .collect::<Vec<_>>();
    let regions = pane_rects
        .iter()
        .map(|(pane, rect)| tabs::PaneRegion {
            pane: *pane,
            strip: pane_strip_rect(*rect),
            body: pane_body_rect(*rect),
        })
        .collect::<Vec<_>>();
    let focused_rect = pane_rects
        .iter()
        .find(|(pane, _)| *pane == focused_pane)
        .map_or(editor_area, |(_, rect)| *rect);
    let code_rect = pane_body_rect(focused_rect);
    let tab_visibility_state = state.clone();
    let tab_visibility_items = strips
        .iter()
        .map(|(pane, strip)| {
            let tabs = s
                .workspace
                .pane(*pane)
                .map_or(&[][..], |pane| pane.items())
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
            let active = s.workspace.pane(*pane).and_then(|pane| pane.active());
            (*pane, tabs, active, strip.width())
        })
        .collect::<Vec<_>>();
    let tab_visibility_strips = strips.clone();
    cx.use_effect((tab_visibility_items, s.show_drawer), move || {
        let snapshot = tab_visibility_state.get();
        let desired = tab_visibility_strips
            .iter()
            .map(|(pane, strip)| (*pane, tabs::active_visible_scroll(*strip, &snapshot, *pane)))
            .collect::<Vec<_>>();
        tab_visibility_state.try_update(move |app| {
            let mut changed = false;
            for (pane, scroll) in desired {
                if (tabs::tab_scroll_x(app, pane) - scroll).abs() > f32::EPSILON {
                    app.tab_scroll.insert(pane, scroll);
                    changed = true;
                }
            }
            changed
        });
        || {}
    });
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
    let key_env = KeyEnv {
        state: state.clone(),
        application: application_context.clone(),
        editor_focus: editor_focus.clone(),
        editor_rect: code_rect,
        terminal: Some(TerminalEnv {
            tabs: terminal_tabs.clone(),
            focus: terminal_focus.clone(),
            controller: terminal_controller.clone(),
        }),
    };
    // A key sequence waiting for its next keystroke times out after a pause;
    // each new keystroke bumps the sequence number and restarts the timer.
    let pending_timer = (!s.pending_keystrokes.is_empty()).then_some(s.pending_keystrokes_seq);
    let pending_env = key_env.clone();
    cx.use_effect(pending_timer, move || {
        let stop = pending_timer.map(|seq| key_actions::start_pending_timer(pending_env, seq));
        move || {
            if let Some(stop) = stop {
                stop();
            }
        }
    });
    let keymap_state = state.clone();
    cx.use_effect((), move || {
        key_actions::reload_user_keymap(&keymap_state);
        let watcher = key_actions::watch_user_keymap(keymap_state);
        move || drop(watcher)
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
    // Every bound key resolves here, in the capture phase, before the focused
    // element sees it (see `key_actions`).
    root = key_actions::attach(root, key_env.clone());

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
    let hover_bodies = bodies.clone();
    root = root.on_event_capture(UiEventKind::PointerMove, move |_ctx, payload| {
        if let UiEventPayload::PointerMove { pointer } = payload {
            let hovered = hover_bodies
                .iter()
                .find(|(_, rect)| rect.contains(pointer.point))
                .map(|(pane, _)| *pane);
            st_editor_hover.try_update(move |app| {
                let changed = app.editor_hovered != hovered
                    || (hovered.is_none()
                        && (app.welcome_hover.is_some() || app.workspace_home_hover.is_some()));
                app.editor_hovered = hovered;
                if hovered.is_none() {
                    app.welcome_hover = None;
                    app.workspace_home_hover = None;
                }
                changed
            });
        }
    });

    // Pressing anywhere in a pane focuses it before the pressed element reacts,
    // so surface handlers can keep acting on the active pane. Open overlays
    // keep focus where it is: their actions target the pane they opened for.
    let st_pane_focus = state.clone();
    let focus_panes = pane_rects.clone();
    root = root.on_event_capture(UiEventKind::PointerDown, move |_ctx, payload| {
        if let UiEventPayload::PointerDown { pointer, .. } = payload
            && let Some(pane) = focus_panes
                .iter()
                .find(|(_, rect)| rect.contains(pointer.point))
                .map(|(pane, _)| *pane)
        {
            st_pane_focus.try_update(move |app| {
                let overlay = app.editor.menu.is_some()
                    || app.context_menu.is_some()
                    || app.tab_context_menu.is_some()
                    || app.close_request.is_some()
                    || app.show_clone_dialog;
                !overlay && app.workspace.activate_pane(pane)
            });
        }
    });

    let st_tabs_pointer = state.clone();
    let pointer_regions = regions.clone();
    root = root.on_event_capture(UiEventKind::PointerMove, move |_ctx, payload| {
        if let UiEventPayload::PointerMove { pointer } = payload {
            st_tabs_pointer.try_update(|app| {
                tabs::update_pointer(
                    app,
                    &pointer_regions,
                    pointer.point.x,
                    pointer.point.y,
                    false,
                )
            });
        }
    });

    let st_tabs_drag = state.clone();
    let drag_regions = regions.clone();
    root = root.on_event_capture(UiEventKind::PointerDrag, move |_ctx, payload| {
        if let UiEventPayload::PointerDrag { pointer } = payload {
            st_tabs_drag.try_update(|app| {
                tabs::update_pointer(app, &drag_regions, pointer.point.x, pointer.point.y, true)
            });
        }
    });

    let st_tabs_pointer_up = state.clone();
    root = root.on_event_capture(UiEventKind::PointerUp, move |_ctx, _payload| {
        st_tabs_pointer_up.try_update(tabs::finish_pointer_interaction);
    });

    // A release that never arrived must not drop a stale file on the next
    // click; a file row press records a fresh drag after this runs.
    let st_stale_file_drag = state.clone();
    root = root.on_event_capture(UiEventKind::PointerDown, move |_ctx, _payload| {
        st_stale_file_drag.try_update(|app| app.file_drag.take().is_some());
    });
    // An Explorer file press becomes a drag into the editor once it moves;
    // its release either opens it like a click or drops it on a pane.
    let st_file_drag = state.clone();
    let file_drag_regions = regions.clone();
    root = root.on_event_capture(UiEventKind::PointerDrag, move |_ctx, payload| {
        if let UiEventPayload::PointerDrag { pointer } = payload {
            st_file_drag.try_update(|app| {
                tabs::update_file_drag(app, &file_drag_regions, pointer.point.x, pointer.point.y)
            });
        }
    });
    let st_file_drop = state.clone();
    let file_drop_focus = editor_focus.clone();
    root = root.on_event_capture(UiEventKind::PointerUp, move |_ctx, _payload| {
        let mut focus = false;
        st_file_drop.try_update(|app| {
            if app.file_drag.is_none() {
                return false;
            }
            focus = crate::workspace_actions::finish_file_drag(app);
            true
        });
        if focus {
            file_drop_focus.focus();
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
    root = root.child(titlebar::render(titlebar_rect, state.clone()));

    for (&(pane, strip), &(_, body)) in strips.iter().zip(bodies.iter()) {
        root = root.child(tabs::render(
            pane,
            strip,
            state.clone(),
            editor_focus.clone(),
        ));
        // Only the focused pane's surface carries the focusable id, so
        // keyboard and IME input always reach the focused pane.
        let surface_id = if pane == focused_pane {
            main_surface_id.clone()
        } else {
            UiId::owned(format!("pane-surface-{}", pane.get()))
        };
        root = root.child(match s.pane_surface(pane) {
            MainSurface::Settings => settings::render(body, state.clone()),
            MainSurface::Keymap => keymap_page::render(
                body,
                state.clone(),
                keymap_search_id.clone(),
                keymap_search_focus.clone(),
            ),
            MainSurface::Diff => {
                diff_editor::render(pane, body, state.clone(), editor_focus.clone())
            }
            MainSurface::Editor => editor_view::render(
                pane,
                body,
                state.clone(),
                git_store.clone(),
                surface_id,
                editor_focus.clone(),
            ),
            MainSurface::Empty => Element::new(move |_cx| {
                UiElement::panel(surface_id, body, VisualStyle::filled(theme::c().bg))
            }),
            MainSurface::WorkspaceHome => workspace_home::render(
                body,
                state.clone(),
                surface_id,
                editor_focus.clone(),
                terminal_focus.clone(),
                terminal_tabs.clone(),
            ),
            MainSurface::Welcome => {
                welcome::render(body, state.clone(), surface_id, editor_focus.clone())
            }
        });
    }
    for sash in s.workspace.layout().sashes(editor_area) {
        root = root.child(pane_sash::render(sash, state.clone()));
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
        && let Some(tooltip) = tabs::render_tooltip(vp, &strips, &s)
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

    // Drag feedback goes last: inserting it earlier would shift the index of
    // the elements after it and drop the pressed element's pointer capture.
    // Preview where a dragged tab or Explorer file would land on a pane.
    let file_drop = s
        .file_drag
        .as_ref()
        .filter(|drag| drag.active)
        .and_then(|drag| drag.drop);
    if let Some(TabDrop::Pane { pane, split }) = s
        .tab_drag
        .as_ref()
        .filter(|drag| drag.active)
        .and_then(|drag| drag.drop)
        .or(file_drop)
        && let Some(region) = regions.iter().find(|region| region.pane == pane)
    {
        root = root.child(lgui::prelude::panel(
            tabs::drop_preview(region.body, split),
            VisualStyle::filled(theme::c().accent).alpha(40),
        ));
    }

    // The dragged Explorer file's name follows the pointer.
    if let Some(drag) = s.file_drag.as_ref().filter(|drag| drag.active) {
        let name = drag.path.file_name().map_or_else(
            || drag.path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let width = tabs::measure(&name, theme::SMALL, 400) + 20.0;
        let left = (drag.point.0 + 12.0).min(vp.right - width - 4.0);
        let top = (drag.point.1 + 8.0).min(vp.bottom - 28.0);
        let label = UiRect::new(left, top, left + width, top + 24.0);
        root = root.child(
            theme::bordered(label, theme::c().surface, theme::c().border, 4.0, 1.0).child(
                lgui::prelude::text(
                    UiRect::new(
                        label.left + 9.0,
                        label.top + 4.0,
                        label.right - 9.0,
                        label.bottom - 3.0,
                    ),
                    name,
                    theme::mono(theme::c().text_soft, theme::SMALL),
                ),
            ),
        );
    }

    root
}

/// The tab strip across the top of a pane.
fn pane_strip_rect(pane: UiRect) -> UiRect {
    UiRect::new(pane.left, pane.top, pane.right, pane.top + theme::TABS_H)
}

/// A pane's surface below its tab strip.
fn pane_body_rect(pane: UiRect) -> UiRect {
    UiRect::new(pane.left, pane.top + theme::TABS_H, pane.right, pane.bottom)
}
