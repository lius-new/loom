//! Composes Source Control projections, frame inputs and cross-feature commands.
use crate::app::context::AppContext;
use crate::app::state::{FileDragSource, FileDragState};
use crate::features::git::actions::StashMode;
use crate::features::git::{DiffRequest, GitStoreSnapshot};
use crate::features::keymap::action::Action;
use crate::ui::components::state_access::StateAccess;
use crate::ui::git::commands::{Binding, Command, Frame};
use lgui::prelude::State;

pub(crate) fn binding(state: &AppContext, store: &State<GitStoreSnapshot>) -> Binding {
    let reader = state.clone();
    let dispatch_state = state.clone();
    let store = store.clone();
    Binding::new(
        StateAccess::project(state.model(), |app| &app.git, |app| &mut app.git),
        move || {
            let app = reader.get();
            Frame {
                resizing: app.source_control.resizing,
                resizable: app.layout.application_layout.is_none(),
                terminal_focused: app.terminal.ui.terminal_focused,
                active_diff: app.workspace.active_diff().map(|d| DiffRequest {
                    repository_root: d.repository_root.clone(),
                    path: d.path.clone(),
                    target: d.target,
                }),
                active_diff_focus: app.workspace.active_diff_focus().cloned(),
            }
        },
        move |command| run(&dispatch_state, &store, command),
    )
}

pub(crate) fn run(state: &AppContext, store: &State<GitStoreSnapshot>, command: Command) -> bool {
    match command {
        Command::ClosePeek => {
            state.update(|app| app.editor.editor_hunk_peek = None);
            true
        }
        Command::NavigatePeek {
            pane,
            path,
            root,
            relative,
            line,
            y,
        } => {
            state.update(|app| {
                let (x, _) = app.workspace.scroll(pane);
                app.workspace.set_scroll(pane, x, y);
            });
            super::git_peek::open_at(state, pane, path, root, relative, line);
            true
        }
        Command::RequestDiscardBlock { root, path, block } => {
            super::git::request_discard_block(state, root, path, block);
            true
        }
        Command::OpenCommitDiff { root, oid, path } => {
            super::git::open_commit_diff(state, root, oid, path);
            true
        }
        Command::HoverCommit { root, oid, anchor } => {
            super::git::hover_commit(state, root, oid, anchor);
            true
        }
        Command::HoverTag { root, name, anchor } => {
            super::git::hover_tag(state, store, root, name, anchor);
            true
        }
        Command::Commit => super::git::commit(state, store),
        Command::RequestCreateTag { root } => super::git::request_create_tag(state, store, &root),
        Command::CreateTag { root, name, target } => {
            super::git::create_tag(state, store, &root, name, target)
        }
        Command::RequestDeleteTag { root, name } => {
            super::git::request_delete_tag(state, store, &root, name)
        }
        Command::DeleteTag { root, name } => super::git::delete_tag(state, store, &root, name),
        Command::LoadMoreHistory { root } => {
            if store.get().active().is_some_and(|repository| {
                repository.worktree_root == root
                    && state
                        .get()
                        .git
                        .model
                        .history()
                        .is_some_and(|history| history.head == repository.head_oid)
            }) {
                super::git::load_more_history(state, root);
            }
            true
        }
        Command::ToggleCommit { root, oid } => {
            super::git::toggle_commit(state, root, oid);
            true
        }
        Command::OpenCommitChanges { root, oid, focus } => {
            super::git::open_commit_changes(state, root, oid, focus);
            true
        }
        Command::OpenWorktree { path } => super::git::open_worktree(state, store, path),
        Command::AddWorktreeFolder { path } => {
            super::git::add_worktree_folder(state, path);
            true
        }
        Command::RequestRemoveWorktree {
            repository,
            path,
            force,
        } => {
            super::git::request_remove_worktree(state, repository, path, force);
            true
        }
        Command::ApplyStash { oid, pop } => super::git::apply_stash(state, store, oid, pop),
        Command::RequestDropStash { oid } => super::git::request_drop_stash(state, store, oid),
        Command::RequestAbort => super::git::request_abort(state, store),
        Command::ContinueOperation => super::git::continue_operation(state, store),
        Command::StagePaths { paths } => super::git::stage_paths(state, store, paths),
        Command::UnstagePaths { paths } => super::git::unstage_paths(state, store, paths),
        Command::RequestDiscard { tracked, untracked } => {
            super::git::request_discard(state, store, tracked, untracked)
        }
        Command::ResolveConflict { path, side } => {
            super::git::resolve_conflict(state, store, path, side)
        }
        Command::LoadBranches {
            repository,
            sequence,
        } => {
            super::git::load_branches(state, repository, sequence);
            true
        }
        Command::Checkout {
            root,
            checkout,
            mode,
        } => super::git::checkout(state, store, &root, checkout, mode),
        Command::FetchBranches => super::git::fetch_branches(state, store),
        Command::RequestCreateWorktree {
            repository,
            checkout,
        } => {
            super::git::request_create_worktree(state, repository, checkout);
            true
        }
        Command::CreateWorktree {
            root,
            path,
            checkout,
        } => super::git::create_worktree(state, store, &root, path, checkout),
        Command::RemoveWorktree { root, path, force } => {
            super::git::remove_worktree(state, store, &root, path, force)
        }
        Command::DiscardPaths {
            root,
            tracked,
            untracked,
        } => super::git::discard_paths(state, store, &root, tracked, untracked),
        Command::CommitAll => super::git::commit_all(state, store),
        Command::AbortOperation { root, operation } => {
            super::git::abort_operation(state, store, &root, operation)
        }
        Command::DropStash { root, oid } => super::git::drop_stash(state, store, &root, oid),
        Command::ApplyBlock {
            root,
            path,
            target,
            block,
            action,
        } => super::git::apply_block(state, store, &root, path, target, block, action),
        Command::ChooseWorktreeLocation => {
            super::git::choose_worktree_location(state);
            true
        }
        Command::OpenDiff { request, untracked } => {
            super::git::open_diff(state, request, untracked);
            true
        }
        Command::RevertCommit { root, oid } => super::git::revert_commit(state, store, &root, oid),
        Command::UndoLastCommit => super::git::undo_last_commit(state, store),
        Command::StashAll => super::git::stash_all(state, store),
        Command::SelectRepository { root } => {
            super::git::select_repository(state, store, root);
            true
        }
        Command::Sync => super::git::sync(state, store),

        Command::ResizeStart => {
            state.update(|app| app.source_control.resizing = true);
            true
        }
        Command::Resize { width } => {
            state.update(|app| {
                if app.source_control.resizing {
                    app.source_control.width = width.clamp(
                        crate::ui::theme::SIDEBAR_MIN_W,
                        crate::ui::theme::SIDEBAR_MAX_W,
                    )
                }
            });
            true
        }
        Command::ResizeEnd => {
            state.update(|app| app.source_control.resizing = false);
            true
        }
        Command::ShowSourceControl => {
            state.update(|app| app.source_control.visible = true);
            true
        }
        Command::PressChange {
            request,
            untracked,
            point,
        } => {
            state.update(|app| {
                app.git.ui.menu = None;
                app.editor.tabs.file_drag = Some(FileDragState {
                    path: request.repository_root.join(&request.path),
                    source: FileDragSource::Changes { request, untracked },
                    origin: point,
                    point,
                    active: false,
                    drop: None,
                });
            });
            true
        }
        Command::OpenFile { path } => {
            let mut focus = false;
            state.update(|app| {
                focus = super::explorer::open_tree_file(
                    app,
                    path,
                    2,
                    state.services().and_then(|r| r.file_loader()).as_ref(),
                )
            });
            focus
        }
        Command::CopyPath {
            path,
            relative,
            clipboard,
        } => {
            super::explorer::copy_path(state, clipboard.as_ref(), &path, relative);
            true
        }
        Command::Reveal { path } => {
            super::explorer::reveal_item(
                state,
                &crate::features::explorer::ExplorerContextTarget {
                    path,
                    kind: crate::features::explorer::ExplorerTargetKind::File,
                },
            );
            true
        }
        Command::CloneRepository => super::git_clone::clone_repository(state),
    }
}
pub(crate) fn perform_confirm(
    state: &AppContext,
    store: &State<GitStoreSnapshot>,
    action: Action,
) -> bool {
    match action {
        Action::DialogConfirm => crate::ui::git::confirm::resolve(&binding(state, store), true),
        Action::DialogCancel => crate::ui::git::confirm::resolve(&binding(state, store), false),
        _ => return false,
    }
    true
}
pub(crate) fn perform_stash_switch(
    state: &AppContext,
    store: &State<GitStoreSnapshot>,
    action: Action,
) -> bool {
    let mode = match action {
        Action::DialogConfirm => Some(StashMode::Keep),
        Action::DialogSecondary => Some(StashMode::Carry),
        Action::DialogCancel => None,
        _ => return false,
    };
    crate::ui::git::stash_switch::resolve(&binding(state, store), mode);
    true
}
pub(crate) fn perform_clone(state: &AppContext, action: Action) -> bool {
    match action {
        Action::DialogConfirm => {
            super::git_clone::clone_repository(state);
        }
        Action::DialogCancel => state.update(|app| {
            if !app.git.model.cloning() {
                app.git.ui.clone_visible = false;
                app.git.ui.clone_input_focused = false;
                app.git.ui.clone_error = None;
            }
        }),
        _ => return false,
    }
    true
}

pub(crate) fn confirm_branch(state: &AppContext, store: &State<GitStoreSnapshot>) -> bool {
    crate::ui::git::branch_picker::confirm(&binding(state, store), &store.clone().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::AppState;
    use lgui::application::AppView;
    use lgui::core::UiScale;
    use lgui::prelude::{UiRect, panel};
    use lgui::session::UiSession;
    use std::sync::{Arc, Mutex};

    fn live() -> (AppContext, State<GitStoreSnapshot>, UiSession) {
        let exposed = Arc::new(Mutex::new(None));
        let output = exposed.clone();
        let view: AppView = Arc::new(move |cx| {
            let state = AppContext::detached(cx.state_with(|| {
                let mut app = AppState::new();
                app.workspace
                    .open_path("keep.rs".into(), "unsaved 中文".into());
                app
            }));
            let store = cx.state(GitStoreSnapshot::default());
            *output.lock().unwrap() = Some((state, store));
            panel(UiRect::new(0.0, 0.0, 10.0, 10.0), Default::default())
        });
        let mut session = UiSession::new();
        session.render_view(&view, UiRect::new(0.0, 0.0, 10.0, 10.0), UiScale::ONE);
        let (state, store) = exposed.lock().unwrap().take().unwrap();
        (state, store, session)
    }
    #[test]
    fn projected_form_edits_preserve_workspace_chrome_and_snapshot() {
        let (state, store, _session) = live();
        let view = binding(&state, &store);
        let width = state.get().source_control.width;
        view.update(|git| git.ui.commit_input.set_text("提交 🙂"));
        assert_eq!(state.get().git.ui.commit_input.text(), "提交 🙂");
        assert_eq!(
            state.get().workspace.active_editor().unwrap().text(),
            "unsaved 中文"
        );
        assert_eq!(state.get().source_control.width, width);
        assert_eq!(store.get().generation, 0);
    }
    #[test]
    fn change_press_arms_the_editor_drag_with_its_comparison_identity() {
        let (state, store, _session) = live();
        let request = DiffRequest {
            repository_root: "repo".into(),
            path: "f.rs".into(),
            target: crate::features::git::DiffTarget::HeadToIndex,
        };
        binding(&state, &store).dispatch(Command::PressChange {
            request: request.clone(),
            untracked: false,
            point: (13.0, 29.0),
        });
        let app = state.get();
        let drag = app.editor.tabs.file_drag.as_ref().unwrap();
        assert_eq!(drag.path, std::path::Path::new("repo/f.rs"));
        assert_eq!(drag.origin, (13.0, 29.0));
        assert!(!drag.active);
        assert!(
            matches!(&drag.source,FileDragSource::Changes{request:actual,untracked:false} if *actual==request)
        );
        assert_eq!(
            app.workspace.active_editor().unwrap().text(),
            "unsaved 中文"
        );
    }
    #[test]
    fn drawer_resize_requires_a_drag_and_clamps_the_width() {
        let (state, store, _session) = live();
        let view = binding(&state, &store);
        let original = state.get().source_control.width;
        view.dispatch(Command::Resize { width: 100_000.0 });
        assert_eq!(state.get().source_control.width, original);
        view.dispatch(Command::ResizeStart);
        view.dispatch(Command::Resize { width: 100_000.0 });
        assert_eq!(
            state.get().source_control.width,
            crate::ui::theme::SIDEBAR_MAX_W
        );
        view.dispatch(Command::ResizeEnd);
        view.dispatch(Command::Resize { width: 0.0 });
        assert_eq!(
            state.get().source_control.width,
            crate::ui::theme::SIDEBAR_MAX_W
        );
        assert!(!state.get().source_control.resizing);
    }
}
