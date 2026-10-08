//! Asynchronous UI-facing Git commands.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use lgui::prelude::State;

use crate::git::store::operation;
use crate::git::{GitService, GitStoreSnapshot, OperationKind, RepositorySnapshot};
use crate::state::AppState;

pub fn open_diff(state: &State<AppState>, request: crate::git::DiffRequest, untracked: bool) {
    let app = state.get();
    let Some(application) = app.git_application else {
        return;
    };
    let roots = app.workspace_folders;
    let active_view = (app.workspace.active_pane(), app.workspace.active());
    let sequence = app.git_open_diff_sequence + 1;
    state.update(|app| {
        app.git_open_diff_sequence = sequence;
        app.git_diff_loading = Some(request.clone());
    });
    let worker_state = state.clone();
    let spawned = thread::Builder::new()
        .name("loom-git-open-diff".into())
        .spawn(move || {
            let result = crate::git::service().and_then(|service| {
                let diff = service.backend().parsed_full_diff(
                    &request.repository_root,
                    request.target,
                    std::slice::from_ref(&request.path),
                )?;
                let document = if diff.files.is_empty()
                    && untracked
                    && request.target != crate::git::DiffTarget::HeadToIndex
                {
                    let contents =
                        std::fs::read_to_string(request.repository_root.join(&request.path))?;
                    crate::model::diff_document::DiffDocument::added(
                        request.repository_root,
                        request.path,
                        request.target,
                        &contents,
                    )
                } else {
                    crate::model::diff_document::DiffDocument::from_unified(
                        request.repository_root,
                        request.path,
                        request.target,
                        diff,
                    )
                };
                Ok(Arc::new(document))
            });
            application.post(move || {
                let mut result = Some(result);
                worker_state.update(|app| {
                    if app.git_open_diff_sequence != sequence {
                        return;
                    }
                    app.git_diff_loading = None;
                    if app.workspace_folders != roots
                        || (app.workspace.active_pane(), app.workspace.active()) != active_view
                    {
                        return;
                    }
                    match result.take().unwrap() {
                        Ok(document) => {
                            app.workspace.open_prepared_diff(document);
                        }
                        Err(error) => app.show_error(error.user_message()),
                    }
                });
                crate::background_ui::retire(result);
            });
        })
        .is_ok();
    if !spawned {
        state.update(|app| {
            app.git_diff_loading = None;
            app.show_error("Could not start Git diff worker.");
        });
    }
}

pub fn stage_all(state: &State<AppState>, store: &State<GitStoreSnapshot>) -> bool {
    let Some(repository) = active_repository(store) else {
        return false;
    };
    let files = repository.files.clone();
    run(
        state,
        store,
        repository,
        OperationKind::Stage,
        "Staging changes…",
        move |service, root| {
            let paths = files.keys().cloned().collect::<Vec<_>>();
            service.write(root, |backend| backend.stage(root, &paths))
        },
    )
}

pub fn stage_path(state: &State<AppState>, store: &State<GitStoreSnapshot>, path: PathBuf) -> bool {
    let Some(repository) = active_repository(store) else {
        return false;
    };
    run(
        state,
        store,
        repository,
        OperationKind::Stage,
        "Staging file…",
        move |service, root| service.write(root, |backend| backend.stage(root, &[path])),
    )
}

pub fn unstage_path(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    path: PathBuf,
) -> bool {
    let Some(repository) = active_repository(store) else {
        return false;
    };
    run(
        state,
        store,
        repository,
        OperationKind::Unstage,
        "Unstaging file…",
        move |service, root| service.write(root, |backend| backend.unstage(root, &[path])),
    )
}

pub fn restore_path(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    path: PathBuf,
) -> bool {
    let Some(repository) = active_repository(store) else {
        return false;
    };
    let absolute = repository.worktree_root.join(&path);
    if state
        .get()
        .workspace
        .dirty_paths()
        .iter()
        .any(|dirty| dirty == &absolute)
    {
        state.update(|app| {
            app.show_toast("Save or discard the editor buffer before restoring this file.")
        });
        return false;
    }
    run(
        state,
        store,
        repository,
        OperationKind::Restore,
        "Restoring file…",
        move |service, root| service.write(root, |backend| backend.restore_worktree(root, &[path])),
    )
}

pub fn commit(state: &State<AppState>, store: &State<GitStoreSnapshot>) -> bool {
    let app = state.get();
    let message = app.git_commit_input.text().trim().to_owned();
    let amend = app.git_commit_amend;
    let signoff = app.git_commit_signoff;
    drop(app);
    if message.is_empty() {
        state.update(|app| app.show_toast("Enter a commit message."));
        return false;
    }
    let Some(repository) = active_repository(store) else {
        return false;
    };
    run(
        state,
        store,
        repository,
        OperationKind::Commit,
        "Committing…",
        move |service, root| {
            service
                .write(root, |backend| {
                    backend.commit(root, &message, amend, signoff)
                })
                .map(|_| ())
        },
    )
}

pub fn fetch(state: &State<AppState>, store: &State<GitStoreSnapshot>) -> bool {
    remote_operation(
        state,
        store,
        OperationKind::Fetch,
        "Fetching…",
        |service, root| service.write(root, |backend| backend.fetch(root, None, true)),
    )
}

pub fn pull(state: &State<AppState>, store: &State<GitStoreSnapshot>, rebase: bool) -> bool {
    let Some(repository) = active_repository(store) else {
        return false;
    };
    if state
        .get()
        .workspace
        .has_dirty_paths_under(&repository.worktree_root)
    {
        state.update(|app| app.show_toast("Save or stash editor changes before pulling."));
        return false;
    }
    run(
        state,
        store,
        repository,
        OperationKind::Pull,
        if rebase {
            "Pulling with rebase…"
        } else {
            "Pulling…"
        },
        move |service, root| service.write(root, |backend| backend.pull(root, rebase)),
    )
}

pub fn push(state: &State<AppState>, store: &State<GitStoreSnapshot>) -> bool {
    remote_operation(
        state,
        store,
        OperationKind::Push,
        "Pushing…",
        |service, root| service.write(root, |backend| backend.push(root, None, false)),
    )
}

fn remote_operation(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    kind: OperationKind,
    message: &'static str,
    action: impl FnOnce(&GitService, &std::path::Path) -> crate::git::GitResult<()> + Send + 'static,
) -> bool {
    let Some(repository) = active_repository(store) else {
        return false;
    };
    run(state, store, repository, kind, message, action)
}

fn active_repository(store: &State<GitStoreSnapshot>) -> Option<RepositorySnapshot> {
    store.get().active().cloned()
}

fn run(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    repository: RepositorySnapshot,
    kind: OperationKind,
    message: &'static str,
    action: impl FnOnce(&GitService, &std::path::Path) -> crate::git::GitResult<()> + Send + 'static,
) -> bool {
    if store.get().operation.is_some() {
        return false;
    }
    let app = state.get();
    let Some(application) = app.git_application else {
        return false;
    };
    let poll_control = app.git_poll_control;
    store.update(move |snapshot| {
        snapshot.operation = Some(operation(kind, message, false));
        snapshot.last_error = None;
    });
    let state = state.clone();
    let store = store.clone();
    let worker_store = store.clone();
    let worker_poll_control = poll_control.clone();
    let spawned = thread::Builder::new()
        .name(format!("loom-git-{kind:?}").to_ascii_lowercase())
        .spawn(move || {
            let result = crate::git::service().and_then(|service| action(&service, &repository.worktree_root));
            let disk_updates = if result.is_ok() {
                state.get().workspace.prepare_disk_updates(&[], true)
            } else { Vec::new() };
            let completion_application = application.clone();
            application.post(move || {
            match result {
                Ok(()) => {
                    if kind == OperationKind::Commit {
                        state.update(|app| {
                            app.git_commit_input.clear();
                        });
                    }
                    state.update(|app| {
                        let expected_updates = disk_updates.len();
                        let outcomes = app.workspace.apply_disk_updates(disk_updates);
                        if outcomes.len() < expected_updates {
                            crate::file_watcher::refresh_async(state.clone(), completion_application);
                        }
                        let conflicts = outcomes
                            .iter()
                            .filter(|outcome| matches!(outcome, crate::model::workspace::ReconcileResult::Conflict(_)))
                            .count();
                        if conflicts > 0 {
                            app.show_toast(format!("{conflicts} open file(s) changed on disk; editor buffers were preserved."));
                        }
                    });
                    worker_store.update(move |current| {
                        current.operation = None;
                    });
                }
                Err(error) => {
                    let message = error.user_message();
                    state.update(move |app| app.show_error(message));
                    worker_store.update(move |snapshot| {
                        snapshot.operation = None;
                        snapshot.last_error = Some(error);
                    });
                }
            }
            // The current worker knows the current roots and open tabs. Never
            // publish a separate scan using the operation's starting context.
            worker_poll_control.request_refresh();
            });
        })
        .is_ok();
    if !spawned {
        store.update(|snapshot| snapshot.operation = None);
        poll_control.request_refresh();
    }
    spawned
}
