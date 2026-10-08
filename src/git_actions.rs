//! Asynchronous UI-facing Git commands.

use std::path::PathBuf;
use std::thread;

use lgui::prelude::State;

use crate::git::store::operation;
use crate::git::{GitService, GitStoreSnapshot, OperationKind, RepositorySnapshot};
use crate::state::AppState;

pub fn stage_all(state: &State<AppState>, store: &State<GitStoreSnapshot>) -> bool {
    let Some(repository) = active_repository(store) else {
        return false;
    };
    let paths = repository.files.keys().cloned().collect::<Vec<_>>();
    run(
        state,
        store,
        repository,
        OperationKind::Stage,
        "Staging changes…",
        move |service, root| service.write(root, |backend| backend.stage(root, &paths)),
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
    let Ok(service) = crate::git::service() else {
        state.update(|app| app.show_error("No usable Git runtime is available."));
        return false;
    };
    let roots = state.get().workspace_folders.clone();
    let active_path = state.get().workspace.active_path().map(ToOwned::to_owned);
    store.update(move |snapshot| {
        snapshot.operation = Some(operation(kind, message, false));
        snapshot.last_error = None;
    });
    let state = state.clone();
    let store = store.clone();
    thread::Builder::new()
        .name(format!("loom-git-{kind:?}").to_ascii_lowercase())
        .spawn(move || {
            let result = action(&service, &repository.worktree_root);
            match result {
                Ok(()) => {
                    if kind == OperationKind::Commit {
                        state.update(|app| {
                            app.git_commit_input.clear();
                        });
                    }
                    state.update(|app| {
                        let outcomes = app.workspace.reconcile_disk();
                        let conflicts = outcomes
                            .iter()
                            .filter(|outcome| matches!(outcome, crate::model::workspace::ReconcileResult::Conflict(_)))
                            .count();
                        if conflicts > 0 {
                            app.show_toast(format!("{conflicts} open file(s) changed on disk; editor buffers were preserved."));
                        }
                    });
                    let result = service.refresh(&roots, active_path.as_deref());
                    store.update(move |current| {
                        current.apply_scan(result);
                        current.operation = None;
                    });
                }
                Err(error) => {
                    let message = error.user_message();
                    state.update(move |app| app.show_error(message));
                    store.update(move |snapshot| {
                        snapshot.operation = None;
                        snapshot.last_error = Some(error);
                    });
                }
            }
        })
        .is_ok()
}
