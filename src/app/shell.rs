//! Application shell: composes legacy state and independently owned runtime.
//! Transaction boundaries remain in Workspace; feature migration follows R1.
use crate::ui::git::{
    branch_picker, clone as clone_repository, confirm as git_confirm, menu as git_menu,
    panel as git_panel, stash_switch,
};

use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use lgui::ApplicationHandle;
use lgui::core::{UiElement, UiEventKind, UiEventPayload, UiId};
use lgui::prelude::{Element, RenderCx, UiRect, VisualStyle, component, group};
use lgui::window::WindowFocusChanged;

use crate::app::key_actions::{self, KeyEnv, TerminalEnv};
use crate::app::state::{AppState, CloseContinuation, CloseRequest, MainSurface, TabDrop};
use crate::app::terminal_effects;
use crate::app::window as window_geometry;
use crate::app::workspace_session as workspace_persistence;
use crate::app::{layout_map, terminal_view as terminal};
use crate::domain::application_layout::{ApplicationLayout, Content};
use crate::features::git::GitStoreSnapshot;
use crate::infrastructure::settings_filesystem as settings_persistence;
use crate::ui::editor::view as editor_view;
use crate::ui::terminal::menu as terminal_tab_context_menu;
use crate::ui::theme;
use crate::ui::{tabs, toast};

pub fn app(cx: &mut RenderCx<'_, '_>) -> Element {
    // Pure caret composition frames keep the app's layout, text and hooks cached.
    component(cx.viewport(), |cx, _| render_app(cx))
}

fn render_app(cx: &mut RenderCx<'_, '_>) -> Element {
    let runtime = cx
        .state_with(|| std::sync::Arc::new(super::runtime::AppRuntime::default()))
        .get();
    let state = crate::app::context::AppContext::detached(cx.state_with(AppState::restored))
        .with_runtime(&runtime);
    let git_store = cx.state_with(GitStoreSnapshot::default);
    // Never updated through `State`, so map changes never re-render the app.
    let layout_map_store = cx.state_with(layout_map::MapStore::default).get();
    let git_poll_control = runtime.git_poll_control();
    let terminal_tabs = cx.state_with(|| terminal_effects::restored(workspace_persistence::load()));
    let terminal_tabs = terminal_effects::bind(terminal_tabs, &runtime, &state);
    terminal_effects::install_owner(cx, &runtime, &state, &terminal_tabs);
    let terminal_cursor_blink = cx.state(true);
    let terminal_cursor_visible = terminal_cursor_blink.get();
    let terminal_tab_snapshot = terminal_tabs.get();
    let active_terminal_id = terminal_tab_snapshot.active_id();
    let terminal_controller = terminal_tab_snapshot
        .active()
        .map(|tab| tab.controller.clone());
    let application_context = cx.application();
    let application = application_context.resource::<ApplicationHandle>();
    let loading_state = state.clone();
    let loading_application = application.as_ref().clone();
    let loading_runtime = runtime.clone();
    cx.use_effect((), move || {
        let restore = crate::app::workspace_restore::SessionRestore::new(
            loading_state.clone(),
            loading_application.clone(),
        );
        // Results from a server repaint the editor through the same revision the
        // syntax worker uses.
        let results_state = loading_state.clone();
        let results_ready = crate::infrastructure::ui_dispatch::LatestNotification::new(
            loading_application.clone(),
            move |()| {
                results_state.try_update(|app| {
                    app.editor.syntax_revision = app.editor.syntax_revision.wrapping_add(1);
                    true
                });
            },
        );
        loading_runtime.install_language_services(super::language_services::LanguageServices::new(
            crate::features::language_service::server_set::ServerSet::current(),
            move || results_ready.send(()),
        ));
        loading_runtime.install_highlighter(super::syntax::Highlighter::new(
            loading_state.clone(),
            loading_application.clone(),
        ));
        // The first frame can precede service installation. Schedule another
        // projection so every visible pane can request its initial windows.
        loading_state
            .update(|app| app.editor.syntax_revision = app.editor.syntax_revision.wrapping_add(1));
        let loader =
            crate::app::file_loading::FileLoader::new(loading_state.clone(), loading_application);
        loading_runtime.install_file_loader(loader);
        move || {
            restore.shutdown();
            loading_runtime.clear_file_loader();
            loading_runtime.clear_highlighter();
            loading_runtime.clear_language_services();
        }
    });
    let program_tabs = terminal_tabs.clone();
    let program_application = application.as_ref().clone();
    cx.use_effect(
        terminal_tab_snapshot
            .tabs()
            .iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>(),
        move || terminal_effects::start_programs(program_tabs, program_application),
    );
    let dispatch_runtime = runtime.clone();
    let dispatch_handle = application.as_ref().clone();
    cx.use_effect((), move || {
        dispatch_runtime.set_application(dispatch_handle);
        move || dispatch_runtime.clear_application()
    });
    let notification_state = state.clone();
    let notification_store = git_store.clone();
    let notification_application = application.as_ref().clone();
    let scan_notifications = cx
        .state_with(move || {
            crate::infrastructure::ui_dispatch::LatestNotification::new(
                notification_application,
                move |notification: super::git_poll::GitScanNotification| {
                    super::git_poll::deliver(
                        &notification_state,
                        &notification_store,
                        notification,
                    );
                },
            )
        })
        .get();
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
    let branch_picker_input_id = cx.use_stable_id();
    let branch_picker_input_focus = cx.focus_handle(branch_picker_input_id.clone());
    let stash_switch_id = cx.use_stable_id();
    let stash_switch_focus = cx.focus_handle(stash_switch_id.clone());
    let git_confirm_id = cx.use_stable_id();
    let git_confirm_focus = cx.focus_handle(git_confirm_id.clone());
    let git_tag_input_focus = cx.focus_handle(UiId::new(git_confirm::TAG_INPUT_ID));
    let close_dialog_id = cx.use_stable_id();
    let close_dialog_focus = cx.focus_handle(close_dialog_id.clone());
    let keymap_search_id = cx.use_stable_id();
    let keymap_search_focus = cx.focus_handle(keymap_search_id.clone());
    let search_query_id = cx.use_stable_id();
    let search_query_focus = cx.focus_handle(search_query_id.clone());
    let search_include_id = cx.use_stable_id();
    let search_include_focus = cx.focus_handle(search_include_id.clone());
    let search_exclude_id = cx.use_stable_id();
    let search_exclude_focus = cx.focus_handle(search_exclude_id.clone());
    let search_results_id = cx.use_stable_id();
    let search_results_focus = cx.focus_handle(search_results_id.clone());
    let vp = cx.viewport();
    let w = vp.width();
    let h = vp.height();
    window_geometry::observe_viewport(w, h);
    let window_focus_state = state.clone();
    let window_focus_git_poll = git_poll_control.clone();
    let window_focus_application = application.as_ref().clone();
    let window_focus_map = layout_map_store.clone();
    cx.use_event_once::<WindowFocusChanged>(move |event| {
        if event.window_id.as_str() == "loom" {
            window_geometry::handle_focus_change(event.focused);
            // A background window does not poll; regaining focus rescans.
            window_focus_git_poll.set_paused(!event.focused);
            if !event.focused {
                window_focus_map.cancel();
                window_focus_state.try_update(|app| {
                    // A key sequence does not survive leaving the window.
                    let had_pending = !app.keymap.pending().is_empty();
                    app.keymap.clear_pending();
                    let split_drag = app.editor.diff_split.drag.take().is_some()
                        | app.terminal.ui.terminal_split_drag.take().is_some()
                        | app.layout.application_sash_drag.take().is_some();
                    crate::app::editor::finish_scrollbar_drag(app)
                        | super::tabs::cancel_pointer_interaction(app)
                        | had_pending
                        | split_drag
                });
            } else {
                crate::app::file_watcher::refresh_async(
                    window_focus_state.clone(),
                    window_focus_application.clone(),
                );
            }
        }
    });
    let close_state = state.clone();
    let close_terminals = terminal_tabs.clone();
    cx.use_event_once::<window_geometry::AppCloseRequested>(move |_| {
        let app = close_state.get();
        terminal_effects::save(
            &close_terminals,
            app.default_terminal_open(),
            app.terminal_chrome.terminal_h,
        );
        let workspace = app.workspace;
        let dirty_targets = workspace
            .dirty_file_ids()
            .into_iter()
            .flat_map(|id| workspace.tabs_of(id))
            .collect::<Vec<_>>();
        if dirty_targets.is_empty() {
            if crate::app::workspace_commands::save_editor_session(&close_state) {
                window_manager.close("loom");
            }
        } else {
            close_state.update(move |app| {
                app.editor.close_request = Some(CloseRequest {
                    targets: dirty_targets,
                    continuation: CloseContinuation::ExitApplication,
                });
                app.editor.tabs.tab_context_menu = None;
                app.terminal.ui.terminal_tab_context_menu = None;
                app.git.ui.clone_visible = false;
                app.git.ui.branch_picker = None;
                app.git.ui.stash_switch = None;
                app.git.ui.confirm = None;
                app.git.ui.menu = None;
                app.editor.menu = None;
            });
        }
    });
    cx.use_effect((), || {
        window_geometry::remember_native_window();
        || {}
    });

    // Snapshot toggles for this frame.
    let content_area = UiRect::new(
        0.0,
        theme::TITLEBAR_H,
        w.max(1.0),
        (h - theme::STATUS_H).max(theme::TITLEBAR_H + 1.0),
    );
    let session_ids = terminal_tab_snapshot
        .tabs()
        .iter()
        .map(|t| t.id)
        .collect::<Vec<_>>();
    let session_groups = terminal_tab_snapshot
        .tabs()
        .iter()
        .map(|t| (t.id, t.group_id))
        .collect::<Vec<_>>();
    state.try_update(|app| {
        let before = app.layout.application_layout.clone();
        let layout = app.layout.application_layout.get_or_insert_with(|| {
            ApplicationLayout::initial(
                content_area.into(),
                app.source_control.width,
                app.drawer.width,
                app.terminal_chrome.terminal_h,
            )
        });
        layout.sync(&session_groups, active_terminal_id);
        let changed = before.as_ref() != Some(layout);
        changed
    });
    let s = state.get();
    let syntax_runtime = runtime.clone();
    let syntax_open = s.workspace.ordered_documents();
    cx.use_effect(syntax_open.clone(), move || {
        if let Some(highlighter) = syntax_runtime.highlighter() {
            highlighter.worker.retain(syntax_open);
        }
        || {}
    });
    layout_map::refresh(&s, &layout_map_store, content_area, session_ids.clone());
    theme::set(s.preferences.values().theme);
    crate::ui::fonts::set(
        &s.preferences.values().editor_font,
        &s.preferences.values().terminal_font,
    );
    let editor_layout = crate::app::workspace_commands::saved_app_editor_layout(&s);
    let git_roots = s.explorer.model.roots().to_vec();
    let git_target = super::git_poll::poll_target(&s);
    // Keep the history of the active repository's HEAD while it is shown.
    let history_key = super::git_poll::history_key(&s, &git_store.get());
    let history_state = state.clone();
    cx.use_effect(history_key.clone(), move || {
        if let Some((root, head)) = history_key {
            crate::app::git::load_history(&history_state, root, head);
        }
        || {}
    });
    let file_watch_state = state.clone();
    let file_watch_roots = git_roots.clone();
    let file_watch_application = application.as_ref().clone();
    cx.use_effect(file_watch_roots.clone(), move || {
        let handle = if file_watch_roots.is_empty() {
            None
        } else {
            let batch_state = file_watch_state.clone();
            let error_roots = file_watch_roots.clone();
            let error_state = file_watch_state.clone();
            let error_application = file_watch_application.clone();
            match crate::app::file_watcher::start_async_scoped(file_watch_roots, move |batch, lease| {
                let prepared = crate::app::file_watcher::prepare_batch(&batch_state.get(), &batch);
                let state = batch_state.clone();
                crate::app::file_watcher::notify_prepared_scoped(state, file_watch_application.clone(), prepared, lease);
            }, move |error, lease| {
                let state = error_state.clone();
                let roots = error_roots.clone();
                error_application.post(move || {
                    if lease.cancelled() {return;}
                    state.try_update(|app| {
                        if app.explorer.model.roots() != roots { return false }
                        app.show_error(format!("File watching is unavailable; Explorer will refresh when Loom regains focus: {error}"));
                        true
                    });
                });
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
    let git_poll = git_poll_control.clone();
    let poll_target = git_poll.clone();
    cx.use_effect(git_target.clone(), move || {
        poll_target.set_target(git_target);
        || {}
    });
    cx.use_effect(git_roots.clone(), move || {
        let handle = super::git_poll::start(git_roots, git_poll, move |notification| {
            scan_notifications.send(notification)
        });
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
            crate::app::editor_vim::sync_view(app);
        });
    });
    terminal_effects::install_save(cx, &terminal_tabs, &s);
    let show_drawer = s.drawer.visible;
    let search_key = crate::app::search::key(&s);
    let search_state = state.clone();
    let search_application = application.as_ref().clone();
    cx.use_effect(search_key.clone(), move || {
        crate::app::search::start(search_state, search_application, search_key)
    });
    let search_visible = s.drawer.visible && s.drawer.mode == crate::ui::drawer::DrawerMode::Search;
    let query_focus = search_query_focus.clone();
    cx.use_effect((search_visible, s.search.focus_request), move || {
        if search_visible {
            query_focus.focus();
        }
        || {}
    });
    let result_focus = search_results_focus.clone();
    let focus_requested = s.search.results_focus_request;
    cx.use_effect((search_visible, focus_requested), move || {
        if search_visible && focus_requested > 0 {
            result_focus.focus();
        }
        || {}
    });
    let source_control_open = s.source_control.visible;
    cx.use_effect(source_control_open, move || {
        let _ = workspace_persistence::save_source_control_open(source_control_open);
        || {}
    });
    let settings = s.settings();
    let scale_windows = application_context.windows();
    cx.use_effect(settings.ui_scale, move || {
        scale_windows.set_scale_preference(window_geometry::scale_preference(settings.ui_scale));
        || {}
    });
    cx.use_effect(settings.clone(), move || {
        let _ = settings_persistence::save(&settings);
        || {}
    });
    let show_clone_dialog = s.git.ui.clone_visible;
    let editing_explorer_entry =
        s.explorer.model.create_request().is_some() || s.explorer.model.rename_request().is_some();

    // ---- Region layout ------------------------------------------------
    let titlebar_rect = UiRect::new(0.0, 0.0, w, theme::TITLEBAR_H);
    let statusbar_rect = UiRect::new(0.0, h - theme::STATUS_H, w, h);
    let application_layout = s.layout.application_layout.as_ref().unwrap();
    let layout_visibility = layout_map::visibility(&s);
    let application_snapshot = application_layout.snapshot(content_area.into(), layout_visibility);
    let source_control_left = s.source_control.visible;
    // Each pane is a tab strip above its surface; the focused pane's rects
    // drive the editor-wide handlers (selection auto-scroll, menus, keys).
    let editor_area = application_layout
        .content_rect(&application_snapshot, &Content::Editor)
        .map(UiRect::from)
        .unwrap_or(content_area);
    let focused_pane = s.workspace.active_pane();
    let pane_rects: Vec<_> = s
        .workspace
        .layout()
        .layout(editor_area.into())
        .into_iter()
        .map(|(pane, rect)| (pane, UiRect::from(rect)))
        .collect();
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
    let external_drop_state = state.clone();
    let external_drop_focus = editor_focus.clone();
    let external_drop_application = application.as_ref().clone();
    let external_drop_panes = pane_rects.clone();
    cx.use_effect(pane_rects.clone(), move || {
        let cleanup = crate::infrastructure::file_drop::install(w, move |paths, (x, y)| {
            let Some(pane) = external_drop_panes
                .iter()
                .find(|(_, rect)| rect.contains(lgui::core::Point::new(x, y)))
                .map(|(pane, _)| *pane)
            else {
                return;
            };
            let state = external_drop_state.clone();
            let focus = external_drop_focus.clone();
            // Defer until the native window callback has returned.
            external_drop_application.post(move || {
                let loader = state.services().and_then(|runtime| runtime.file_loader());
                let mut opened = false;
                state.try_update(|app| {
                    if app.editor.close_request.is_some()
                        || app.git.ui.confirm.is_some()
                        || app.git.ui.clone_visible
                        || app.git.ui.stash_switch.is_some()
                    {
                        return false;
                    }
                    let paths = paths.into_iter().filter(|path| !path.is_dir()).collect();
                    opened = super::file_loading::request_files(app, pane, paths, loader.as_ref());
                    opened
                });
                if opened {
                    focus.focus();
                }
            });
        });
        cleanup
    });
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
    cx.use_effect((tab_visibility_items, s.drawer.visible), move || {
        let snapshot = tab_visibility_state.get();
        let desired = tab_visibility_strips
            .iter()
            .map(|(pane, strip)| {
                (
                    *pane,
                    super::tabs::active_visible_scroll(*strip, &snapshot, *pane),
                )
            })
            .collect::<Vec<_>>();
        tab_visibility_state.try_update(move |app| {
            let mut changed = false;
            for (pane, scroll) in desired {
                if (super::tabs::tab_scroll_x(app, pane) - scroll).abs() > f32::EPSILON {
                    app.editor.tabs.tab_scroll.insert(pane, scroll);
                    changed = true;
                }
            }
            changed
        });
        || {}
    });
    let source_control_rect = application_layout
        .content_rect(&application_snapshot, &Content::Git)
        .map(UiRect::from)
        .unwrap_or_default();
    let explorer_rect = application_layout
        .content_rect(&application_snapshot, &Content::Files)
        .map(UiRect::from)
        .unwrap_or_default();
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
                        drag_state
                            .try_update(|app| crate::app::editor::drag_scroll_tick(app, code_rect));
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
    let terminal_cwd = {
        s.explorer
            .model
            .roots()
            .first()
            .cloned()
            .or_else(|| std::env::current_dir().ok())
    };

    let terminal_starts = terminal_effects::visible_starts(
        application_layout,
        &application_snapshot,
        &terminal_tab_snapshot,
        &terminal_cwd,
    );
    let terminal_start_key = terminal_starts
        .iter()
        .map(|(id, _, cwd, size)| (*id, cwd.clone(), *size))
        .collect::<Vec<_>>();
    let terminal_start_application = application.clone();
    let active_visible_terminal =
        active_terminal_id.filter(|id| application_layout.terminal_visible(*id, layout_visibility));
    let terminal_visible = active_visible_terminal.is_some();
    cx.use_effect(terminal_start_key, move || {
        terminal_effects::start_visible(terminal_starts, terminal_cwd, terminal_start_application)
    });
    let mounted_terminal_focus = terminal_focus.clone();
    terminal_tab_context_menu::restore_focus_on_dismiss(
        cx,
        s.terminal.ui.terminal_tab_context_menu.is_some()
            || s.terminal.ui.terminal_workspace_picker.is_some(),
        terminal_visible,
        terminal_focus.clone(),
        editor_focus.clone(),
    );
    cx.use_effect(active_visible_terminal, move || {
        if terminal_visible {
            mounted_terminal_focus.focus();
        }
    });
    let mounted_clone_focus = clone_input_focus.clone();
    cx.use_effect(show_clone_dialog, move || {
        if show_clone_dialog {
            mounted_clone_focus.focus();
        }
    });
    crate::ui::hold_focus(
        cx,
        s.git
            .ui
            .branch_picker
            .as_ref()
            .map(|picker| picker.restore_terminal),
        branch_picker_input_focus.clone(),
        terminal_focus.clone(),
        editor_focus.clone(),
    );
    crate::ui::hold_focus(
        cx,
        s.git
            .ui
            .stash_switch
            .as_ref()
            .map(|request| request.restore_terminal),
        stash_switch_focus.clone(),
        terminal_focus.clone(),
        editor_focus.clone(),
    );
    crate::ui::hold_focus(
        cx,
        s.git
            .ui
            .confirm
            .as_ref()
            .map(|confirm| confirm.restore_terminal),
        if s.git.ui.confirm.as_ref().is_some_and(|confirm| {
            matches!(
                confirm.action,
                git_confirm::GitConfirmAction::CreateTag { .. }
            )
        }) {
            git_tag_input_focus.clone()
        } else {
            git_confirm_focus.clone()
        },
        terminal_focus.clone(),
        editor_focus.clone(),
    );
    // The close confirmation takes keyboard focus while open so Enter/Esc go
    // to the dialog rather than the editor; it hands focus back on resolve.
    let showing_close_request = s.editor.close_request.is_some();
    let mounted_close_focus = close_dialog_focus.clone();
    cx.use_effect(showing_close_request, move || {
        if showing_close_request {
            mounted_close_focus.focus();
        }
    });
    let mounted_create_focus = explorer_create_input_focus.clone();
    let finished_create_focus = editor_focus.clone();
    let pending_tree_state = state.clone();
    let pending_tree_application = application.as_ref().clone();
    cx.use_effect(editing_explorer_entry, move || {
        if editing_explorer_entry {
            mounted_create_focus.focus();
        } else {
            if pending_tree_state.get().explorer.model.pending_refresh() {
                crate::app::file_watcher::refresh_async(
                    pending_tree_state.clone(),
                    pending_tree_application,
                );
            }
            finished_create_focus.focus();
        }
    });
    // Each toast id gets its own timer; a newer toast restarts it, and
    // `dismiss_toast` ignores a timer that outlived its toast.
    let toast_timer = s.chrome.toast.as_ref().map(|toast| (toast.id, toast.kind));
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
        git_store: git_store.clone(),
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
    let pending_timer = (!s.keymap.pending().is_empty()).then_some(s.keymap.pending_seq());
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
    let project_state = state.clone();
    cx.use_effect(s.explorer.model.roots().to_vec(), move || {
        project_state.update(|app| app.refresh_project_settings());
        || {}
    });
    let settings_state = state.clone();
    cx.use_effect((), move || {
        // The state already holds the file's values; this reports problems.
        key_actions::reload_user_settings(&settings_state);
        let watcher = key_actions::watch_user_settings(settings_state);
        move || drop(watcher)
    });
    // Restarts the daily checks when the setting changes.
    let check_for_updates = s.preferences.values().check_for_updates;
    let update_state = state.clone();
    cx.use_effect(check_for_updates, move || {
        super::update::start(update_state, check_for_updates)
    });
    let blink_focused =
        s.terminal.ui.terminal_focused && s.preferences.values().terminal_cursor_blink;
    let blink_state = terminal_cursor_blink.clone();
    cx.use_effect(
        (terminal_visible, blink_focused, active_visible_terminal),
        move || terminal_effects::blink(blink_state, terminal_visible, blink_focused),
    );

    // ---- Root (global shortcut listener) ------------------------------
    let mut root = group(vp);
    root = layout_map::attach(
        root,
        state.clone(),
        layout_map_store.clone(),
        terminal_tabs.clone(),
        content_area,
        terminal_focus.clone(),
    );
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
                let changed = app.explorer.ui.hovered != hovered
                    || (!hovered && app.explorer.ui.hovered_path.is_some());
                app.explorer.ui.hovered = hovered;
                if !hovered {
                    app.explorer.ui.hovered_path = None;
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
                let mut changed = app.git.ui.hovered != hovered;
                app.git.ui.hovered = hovered;
                // Leaving the drawer drops its row hover; the drawer only
                // tracks moves inside itself.
                if !hovered && app.git.ui.hover.is_some() {
                    app.git.ui.hover = None;
                    app.git.ui.clear_hover_cards();
                    changed = true;
                }
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
                let mut changed = app.editor.editor_hovered != hovered
                    || (hovered.is_none()
                        && (app.chrome.welcome_hover.is_some()
                            || app.chrome.workspace_home_hover.is_some()));
                app.editor.editor_hovered = hovered;
                // A pane's gutter only sees moves inside it; drop its hover
                // once the pointer leaves the pane.
                if app
                    .editor
                    .editor_gutter_hover
                    .is_some_and(|(pane, _)| Some(pane) != hovered)
                {
                    app.editor.editor_gutter_hover = None;
                    changed = true;
                }
                if app
                    .editor
                    .editor_diagnostic_hover
                    .as_ref()
                    .is_some_and(|(pane, _)| Some(*pane) != hovered)
                {
                    app.editor.editor_diagnostic_hover = None;
                    changed = true;
                }
                if hovered.is_none() {
                    app.chrome.welcome_hover = None;
                    app.chrome.workspace_home_hover = None;
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
                    || app.explorer.ui.menu.is_some()
                    || app.editor.tabs.tab_context_menu.is_some()
                    || app.terminal.ui.terminal_tab_context_menu.is_some()
                    || app.terminal.ui.terminal_workspace_picker.is_some()
                    || app.git.ui.branch_picker.is_some()
                    || app.git.ui.stash_switch.is_some()
                    || app.git.ui.menu.is_some()
                    || app.git.ui.confirm.is_some()
                    || app.editor.close_request.is_some()
                    || app.git.ui.clone_visible;
                !overlay && app.workspace.activate_pane(pane)
            });
        }
    });

    let st_tabs_pointer = state.clone();
    let pointer_regions = regions.clone();
    root = root.on_event_capture(UiEventKind::PointerMove, move |_ctx, payload| {
        if let UiEventPayload::PointerMove { pointer } = payload {
            st_tabs_pointer.try_update(|app| {
                super::tabs::update_pointer(
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
                super::tabs::update_pointer(
                    app,
                    &drag_regions,
                    pointer.point.x,
                    pointer.point.y,
                    true,
                )
            });
        }
    });

    let st_tabs_pointer_up = state.clone();
    root = root.on_event_capture(UiEventKind::PointerUp, move |_ctx, _payload| {
        st_tabs_pointer_up.try_update(super::tabs::finish_pointer_interaction);
    });

    // A release that never arrived must not drop a stale file on the next
    // click; a file row press records a fresh drag after this runs.
    let st_stale_file_drag = state.clone();
    root = root.on_event_capture(UiEventKind::PointerDown, move |_ctx, _payload| {
        st_stale_file_drag.try_update(|app| app.editor.tabs.file_drag.take().is_some());
    });
    // A drawer file press becomes a drag into the editor once it moves;
    // its release either opens it like a click or drops it on a pane.
    let st_file_drag = state.clone();
    let file_drag_regions = regions.clone();
    root = root.on_event_capture(UiEventKind::PointerDrag, move |_ctx, payload| {
        if let UiEventPayload::PointerDrag { pointer } = payload {
            st_file_drag.try_update(|app| {
                super::tabs::update_file_drag(
                    app,
                    &file_drag_regions,
                    pointer.point.x,
                    pointer.point.y,
                )
            });
        }
    });
    let st_file_drop = state.clone();
    let file_drop_focus = editor_focus.clone();
    root = root.on_event_capture(UiEventKind::PointerUp, move |_ctx, _payload| {
        if crate::app::explorer::finish_drawer_file_drag(&st_file_drop) {
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
                if app.main_surface() == MainSurface::Diff {
                    crate::app::diff_view::drag_scrollbars(
                        app,
                        code_rect,
                        pointer.point.x,
                        pointer.point.y,
                    )
                } else {
                    crate::app::editor::drag_scrollbars(
                        app,
                        code_rect,
                        pointer.point.x,
                        pointer.point.y,
                    )
                }
            });
        }
    });
    let st_editor_scrollbar_up = state.clone();
    root = root.on_event_capture(UiEventKind::PointerUp, move |_ctx, _payload| {
        st_editor_scrollbar_up.try_update(crate::app::editor::finish_scrollbar_drag);
    });

    // ---- Compose chrome (back-to-front) -------------------------------
    root = root.child(super::chrome::title(
        titlebar_rect,
        state.clone(),
        crate::ui::git::badge::attach(
            group(titlebar_rect),
            titlebar_rect,
            titlebar_rect.left + 8.0,
            super::git_view::binding(&state, &git_store),
            git_store.clone().into(),
        ),
    ));

    for (&(pane, strip), &(_, body)) in strips.iter().zip(bodies.iter()) {
        // Only the focused pane's surface carries the focusable id, so
        // keyboard and IME input always reach the focused pane.
        let surface_id = if pane == focused_pane {
            main_surface_id.clone()
        } else {
            UiId::owned(format!("pane-surface-{}", pane.get()))
        };
        root = root.child(match s.pane_surface(pane) {
            MainSurface::Settings => super::settings::render(body, state.clone()),
            MainSurface::Keymap => super::keymap_view::render(
                body,
                state.clone(),
                keymap_search_id.clone(),
                keymap_search_focus.clone(),
            ),
            MainSurface::Diff => crate::app::diff_view::render(
                pane,
                body,
                state.clone(),
                super::git_view::binding(&state, &git_store),
                editor_focus.clone(),
                super::editor_view::binding(state.clone(), git_store.clone(), pane, body),
            ),
            MainSurface::Editor => editor_view::render(
                pane,
                body,
                super::editor_view::binding(state.clone(), git_store.clone(), pane, body),
                surface_id,
                editor_focus.clone(),
            ),
            MainSurface::Empty => Element::new(move |_cx| {
                UiElement::panel(surface_id, body, VisualStyle::filled(theme::c().bg))
            }),
            MainSurface::WorkspaceHome => super::chrome::workspace_home(
                body,
                state.clone(),
                surface_id,
                editor_focus.clone(),
                terminal_focus.clone(),
                terminal_tabs.clone(),
            ),
            MainSurface::Welcome => {
                super::chrome::welcome(body, state.clone(), surface_id, editor_focus.clone())
            }
        });
        // The surface's fractional top edge must not paint over the tab border.
        root = root.child(super::tabs::render(
            pane,
            strip,
            state.clone(),
            editor_focus.clone(),
        ));
    }
    for sash in s.workspace.layout().sashes(editor_area.into()) {
        root = root.child(super::pane_chrome::render(sash, state.clone()));
    }
    if source_control_left {
        root = root.child(git_panel::render(
            source_control_rect,
            super::git_view::binding(&state, &git_store),
            git_store.clone().into(),
            editor_focus.clone(),
            commit_input_focus,
            commit_input_id,
        ));
    }
    let explorer_services = super::explorer_view::Services {
        application: application_context.clone(),
        editor_focus: editor_focus.clone(),
        terminal_tabs: terminal_tabs.clone(),
    };
    if show_drawer {
        if s.drawer.mode == crate::ui::drawer::DrawerMode::Search {
            root = root.child(super::search::render(
                explorer_rect,
                state.clone(),
                application.as_ref().clone(),
                editor_focus.clone(),
                code_rect,
                crate::ui::search::view::Inputs {
                    ids: [search_query_id, search_include_id, search_exclude_id],
                    focus: [
                        search_query_focus,
                        search_include_focus,
                        search_exclude_focus,
                    ],
                    results_id: search_results_id,
                    results_focus: search_results_focus,
                },
            ));
        } else {
            root = root.child(super::explorer_view::render(
                explorer_rect,
                &state,
                &git_store,
                &explorer_services,
                crate::ui::explorer::commands::EditInput {
                    id: explorer_create_input_id,
                    focus: explorer_create_input_focus,
                },
            ));
        }
    }

    for (region, content, rect) in &application_snapshot.regions {
        if !matches!(content, Content::Terminal { .. }) {
            continue;
        }
        root = root.child(terminal::render_region(
            (*rect).into(),
            state.clone(),
            layout_map_store.clone(),
            editor_focus.clone(),
            terminal_focus.clone(),
            terminal_id.clone(),
            terminal_tabs.clone(),
            application.clone(),
            terminal_cursor_visible,
            *region,
        ));
    }
    for divider in &application_snapshot.dividers {
        root = root.child(layout_map::render_divider(divider.clone(), state.clone()));
    }

    root = root.child(super::chrome::status(
        statusbar_rect,
        state.clone(),
        editor_focus.clone(),
        terminal_focus.clone(),
        terminal_tabs.clone(),
    ));

    if s.explorer.ui.menu.is_none()
        && s.editor.tabs.tab_context_menu.is_none()
        && s.terminal.ui.terminal_tab_context_menu.is_none()
        && s.terminal.ui.terminal_workspace_picker.is_none()
        && s.git.ui.branch_picker.is_none()
        && s.git.ui.stash_switch.is_none()
        && s.git.ui.menu.is_none()
        && s.git.ui.confirm.is_none()
        && !show_clone_dialog
        && s.editor.close_request.is_none()
        && let Some(tooltip) = super::tabs::render_tooltip(vp, &strips, &s)
    {
        root = root.child(tooltip);
    }
    if s.explorer.ui.menu.is_none()
        && s.git.ui.menu.is_none()
        && s.git.ui.confirm.is_none()
        && s.git.ui.branch_picker.is_none()
        && s.git.ui.stash_switch.is_none()
        && !show_clone_dialog
        && s.editor.close_request.is_none()
        && let Some(card) = git_panel::render_commit_card(vp, &s.git).or_else(|| {
            git_store
                .get()
                .active()
                .and_then(|repository| crate::ui::git::tag_card::render(vp, &s.git, repository))
        })
    {
        root = root.child(card);
    }

    // ---- Overlays ------------------------------------------------------
    if s.chrome.language_panel {
        let panel_state = state.clone();
        root = root.child(crate::ui::language_panel::render(
            vp,
            &super::chrome::language_panel_frame(&state),
            std::sync::Arc::new(move |event| match event {
                crate::ui::language_panel::Event::Close => {
                    panel_state.update(|app| app.chrome.language_panel = false);
                }
                crate::ui::language_panel::Event::Restart => {
                    if let Some(services) =
                        panel_state.services().and_then(|r| r.language_services())
                    {
                        services.restart();
                    }
                }
                crate::ui::language_panel::Event::RetryDownloads => {
                    crate::infrastructure::language_packs::resources().retry_downloads();
                }
            }),
        ));
    }
    if let Some(pos) = s.editor.menu {
        root = root.child(super::editor_view::menu(
            vp,
            code_rect,
            pos,
            state.clone(),
            editor_focus.clone(),
        ));
    }
    if let Some(pos) = s.explorer.ui.menu {
        root = root.child(super::explorer_view::render_menu(
            vp,
            pos,
            &state,
            &explorer_services,
        ));
    }
    if s.editor.tabs.tab_context_menu.is_some() {
        root = root.child(super::tab_menu::render(vp, state.clone()));
    }
    if s.terminal.ui.terminal_tab_context_menu.is_some()
        || s.terminal.ui.terminal_workspace_picker.is_some()
    {
        root = root.child(crate::app::terminal_view::menu(
            vp,
            state.clone(),
            terminal_tabs.clone(),
            application.clone(),
            terminal_focus.clone(),
            editor_focus.clone(),
        ));
    }
    if let Some(picker) = &s.git.ui.branch_picker {
        root = root.child(branch_picker::render(
            vp,
            titlebar_rect.bottom,
            picker,
            s.git.ui.branch_picker_input.text(),
            super::git_view::binding(&state, &git_store),
            git_store.clone().into(),
            branch_picker_input_focus.clone(),
            branch_picker_input_id.clone(),
        ));
    }
    if let Some(request) = &s.git.ui.stash_switch {
        root = root.child(stash_switch::render(
            vp,
            request,
            super::git_view::binding(&state, &git_store),
            git_store.clone().into(),
            stash_switch_id.clone(),
        ));
    }
    if let Some(menu) = &s.git.ui.menu {
        root = root.child(git_menu::render(
            vp,
            menu,
            super::git_view::binding(&state, &git_store),
            git_store.clone().into(),
            editor_focus.clone(),
        ));
    }
    if let Some(confirm) = &s.git.ui.confirm {
        root = root.child(git_confirm::render(
            vp,
            confirm,
            super::git_view::binding(&state, &git_store),
            git_store.clone().into(),
            git_confirm_id.clone(),
            git_tag_input_focus.clone(),
        ));
    }
    if show_clone_dialog {
        root = root.child(clone_repository::render(
            vp,
            super::git_view::binding(&state, &git_store),
            clone_input_focus,
            clone_input_id,
        ));
    }
    if s.editor.close_request.is_some() {
        root = root.child(super::close_dialog::render(
            vp,
            state.clone(),
            close_dialog_id,
            editor_focus.clone(),
        ));
    }
    root = root.child(toast::render(
        vp,
        super::presentation::toast(&state),
        super::presentation::dismiss_toast(state.clone()),
    ));

    // Drag feedback goes last: inserting it earlier would shift the index of
    // the elements after it and drop the pressed element's pointer capture.
    // Preview where a dragged tab or drawer file would land on a pane.
    let file_drop = s
        .editor
        .tabs
        .file_drag
        .as_ref()
        .filter(|drag| drag.active)
        .and_then(|drag| drag.drop);
    if let Some(TabDrop::Pane { pane, split }) = s
        .editor
        .tabs
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

    // The dragged drawer file's name follows the pointer.
    if let Some(drag) = s.editor.tabs.file_drag.as_ref().filter(|drag| drag.active) {
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

    root.child(layout_map::render(
        content_area,
        state.clone(),
        layout_map_store.clone(),
        terminal_tabs.clone(),
        terminal_focus.clone(),
    ))
}

/// The tab strip across the top of a pane.
fn pane_strip_rect(pane: UiRect) -> UiRect {
    UiRect::new(pane.left, pane.top, pane.right, pane.top + theme::TABS_H)
}

/// A pane's surface below its tab strip.
fn pane_body_rect(pane: UiRect) -> UiRect {
    UiRect::new(pane.left, pane.top + theme::TABS_H, pane.right, pane.bottom)
}
