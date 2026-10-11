//! Asynchronous UI-facing Git commands.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use lgui::prelude::State;

use crate::git::error::GitErrorKind;
use crate::git::store::operation;
use crate::git::{
    CliGitBackend, ConflictSide, GitError, GitResult, GitService, GitStoreSnapshot, HeadState,
    OperationKind, RemoteInfo, RepositorySnapshot, RepositoryState,
};
use crate::state::AppState;
use crate::ui::branch_picker::{BranchListing, Checkout};
use crate::ui::git_confirm::{GitConfirm, GitConfirmAction};
use crate::ui::stash_switch::StashSwitchRequest;

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

/// The active repository's change sections: conflicts, staged, unstaged and
/// untracked, as the Source Control drawer lists them.
fn sections(store: &State<GitStoreSnapshot>, repository: &RepositorySnapshot) -> [Vec<PathBuf>; 4] {
    store
        .get()
        .presentation
        .repositories
        .get(&repository.id)
        .map(|prepared| prepared.sections.clone())
        .unwrap_or_default()
}

/// Whether any of `paths` (relative to `root`) has unsaved editor changes.
fn unsaved(state: &State<AppState>, root: &Path, paths: &[PathBuf]) -> bool {
    let dirty = state.get().workspace.dirty_paths();
    paths
        .iter()
        .any(|path| dirty.iter().any(|dirty| dirty == &root.join(path)))
}

fn count_label(paths: &[PathBuf], one: &str) -> String {
    match paths {
        [path] => format!("{one} {}…", file_name(path)),
        _ => format!("{one} {} files…", paths.len()),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

pub fn stage_paths(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    paths: Vec<PathBuf>,
) -> bool {
    let Some(repository) = active_repository(store) else {
        return false;
    };
    if paths.is_empty() {
        return false;
    }
    let message = count_label(&paths, "Staging");
    run(
        state,
        store,
        repository,
        OperationKind::Stage,
        message,
        move |service, root| service.write(root, |backend| backend.stage(root, &paths)),
    )
}

pub fn unstage_paths(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    paths: Vec<PathBuf>,
) -> bool {
    let Some(repository) = active_repository(store) else {
        return false;
    };
    if paths.is_empty() {
        return false;
    }
    let message = count_label(&paths, "Unstaging");
    run(
        state,
        store,
        repository,
        OperationKind::Unstage,
        message,
        move |service, root| service.write(root, |backend| backend.unstage(root, &paths)),
    )
}

/// Ask before discarding: worktree changes to `tracked` files are lost and
/// `untracked` files go to the system trash.
pub fn request_discard(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    tracked: Vec<PathBuf>,
    untracked: Vec<PathBuf>,
) -> bool {
    let Some(repository) = active_repository(store) else {
        return false;
    };
    let root = repository.worktree_root;
    let all = tracked
        .iter()
        .chain(&untracked)
        .cloned()
        .collect::<Vec<_>>();
    if all.is_empty() {
        return false;
    }
    if unsaved(state, &root, &all) {
        state.update(|app| {
            app.show_toast("Save or revert the open editor changes before discarding.")
        });
        return false;
    }
    let title = match all.as_slice() {
        [path] if untracked.is_empty() => format!("Discard changes to {}?", file_name(path)),
        [path] => format!("Delete {}?", file_name(path)),
        _ => format!("Discard changes to {} files?", all.len()),
    };
    let message = match (tracked.is_empty(), untracked.is_empty()) {
        (false, true) => "Your changes will be lost. This cannot be undone.",
        (true, false) => "Untracked files are moved to the trash.",
        _ => "Changes to tracked files will be lost; untracked files are moved to the trash.",
    };
    state.update(move |app| {
        app.git_confirm = Some(GitConfirm {
            title,
            message: message.to_owned(),
            confirm_label: if untracked.is_empty() {
                "Discard"
            } else {
                "Discard and Delete"
            },
            destructive: true,
            action: GitConfirmAction::Discard {
                repository: root,
                tracked,
                untracked,
            },
            restore_terminal: app.terminal_focused,
        });
    });
    true
}

/// Discard confirmed changes: restore `tracked` files from the index and
/// move `untracked` files to the trash.
pub fn discard_paths(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    root: &Path,
    tracked: Vec<PathBuf>,
    untracked: Vec<PathBuf>,
) -> bool {
    let Some(repository) = store.get().repository_for_path(root).cloned() else {
        return false;
    };
    let all = tracked
        .iter()
        .chain(&untracked)
        .cloned()
        .collect::<Vec<_>>();
    let message = count_label(&all, "Discarding");
    run_with(
        state,
        store,
        repository,
        OperationKind::Restore,
        message,
        move |service, root| {
            service.write(root, |backend| {
                backend.restore_worktree(root, &tracked)?;
                let mut deleted = Vec::new();
                for path in &untracked {
                    let path = root.join(path);
                    match crate::workspace_actions::move_path_to_trash(&path) {
                        Ok(true) => deleted.push(path),
                        Ok(false) => {}
                        Err(error) => return Err(GitError::new(GitErrorKind::Io, error)),
                    }
                }
                Ok(deleted)
            })
        },
        |app, deleted| {
            for path in deleted {
                crate::workspace_actions::apply_deleted_path_state(app, &path);
            }
        },
    )
}

/// Commit the staged changes. With nothing staged, offer to stage every
/// tracked change first; amending needs no staged changes.
pub fn commit(state: &State<AppState>, store: &State<GitStoreSnapshot>) -> bool {
    let app = state.get();
    let message = app.git_commit_input.text().trim().to_owned();
    let amend = app.git_commit_amend;
    drop(app);
    if message.is_empty() && !amend {
        state.update(|app| app.show_toast("Enter a commit message."));
        return false;
    }
    let Some(repository) = active_repository(store) else {
        return false;
    };
    let [conflicts, staged, unstaged, untracked] = sections(store, &repository);
    if !conflicts.is_empty() {
        state.update(|app| app.show_toast("Resolve and stage the conflicted files first."));
        return false;
    }
    if staged.is_empty() && !amend {
        if unstaged.is_empty() {
            let hint = if untracked.is_empty() {
                "There are no changes to commit."
            } else {
                "Stage the new files you want to commit."
            };
            state.update(move |app| app.show_toast(hint));
            return false;
        }
        let count = unstaged.len();
        state.update(move |app| {
            app.git_confirm = Some(GitConfirm {
                title: "Commit all changes?".into(),
                message: format!(
                    "Nothing is staged. Stage the {count} changed file{} and commit? Untracked \
                     files are not included.",
                    if count == 1 { "" } else { "s" }
                ),
                confirm_label: "Stage and Commit",
                destructive: false,
                action: GitConfirmAction::StageAllAndCommit,
                restore_terminal: app.terminal_focused,
            });
        });
        return false;
    }
    commit_paths(state, store, repository, message, Vec::new())
}

/// Stage every tracked change, then commit.
pub fn commit_all(state: &State<AppState>, store: &State<GitStoreSnapshot>) -> bool {
    let message = state.get().git_commit_input.text().trim().to_owned();
    let Some(repository) = active_repository(store) else {
        return false;
    };
    let [_, _, unstaged, _] = sections(store, &repository);
    commit_paths(state, store, repository, message, unstaged)
}

/// Stage `stage`, then commit with the drawer's options.
fn commit_paths(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    repository: RepositorySnapshot,
    message: String,
    stage: Vec<PathBuf>,
) -> bool {
    let app = state.get();
    let amend = app.git_commit_amend;
    let signoff = app.git_commit_signoff;
    drop(app);
    run_with(
        state,
        store,
        repository,
        OperationKind::Commit,
        if amend {
            "Amending…"
        } else {
            "Committing…"
        },
        move |service, root| {
            service.write(root, |backend| {
                backend.stage(root, &stage)?;
                backend.commit(root, &message, amend, signoff)
            })
        },
        |app, _| app.git_commit_amend = false,
    )
}

/// The active repository, unless an open editor buffer under it has unsaved
/// changes that an operation rewriting the worktree could conflict with.
fn repository_without_unsaved(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    doing: &str,
) -> Option<RepositorySnapshot> {
    let repository = active_repository(store)?;
    if state
        .get()
        .workspace
        .has_dirty_paths_under(&repository.worktree_root)
    {
        let message = format!("Save or revert the open editor changes before {doing}.");
        state.update(move |app| app.show_toast(message));
        return None;
    }
    Some(repository)
}

/// Continue the merge, rebase, cherry-pick or revert in progress.
pub fn continue_operation(state: &State<AppState>, store: &State<GitStoreSnapshot>) -> bool {
    let Some(repository) = repository_without_unsaved(state, store, "continuing") else {
        return false;
    };
    let operation = repository.repository_state;
    run(
        state,
        store,
        repository,
        OperationKind::Other,
        "Continuing…",
        move |service, root| {
            service.write(root, |backend| backend.continue_operation(root, operation))
        },
    )
}

/// Ask before aborting the operation in progress, which discards its work.
pub fn request_abort(state: &State<AppState>, store: &State<GitStoreSnapshot>) -> bool {
    let Some(repository) = repository_without_unsaved(state, store, "aborting") else {
        return false;
    };
    let operation = repository.repository_state;
    let (title, message) = match operation {
        RepositoryState::MergeInProgress => (
            "Abort the merge?",
            "The branch returns to where it was before the merge.",
        ),
        RepositoryState::RebaseInProgress => (
            "Abort the rebase?",
            "The branch returns to where it was before the rebase.",
        ),
        RepositoryState::CherryPickInProgress => (
            "Abort the cherry-pick?",
            "The branch returns to where it was before the cherry-pick.",
        ),
        RepositoryState::RevertInProgress => (
            "Abort the revert?",
            "The branch returns to where it was before the revert.",
        ),
        RepositoryState::Bisecting => (
            "End the bisect?",
            "Git checks out the branch you started bisecting from.",
        ),
        _ => return false,
    };
    let root = repository.worktree_root;
    state.update(move |app| {
        app.git_confirm = Some(GitConfirm {
            title: title.into(),
            message: format!("{message} Conflict resolutions made so far are lost."),
            confirm_label: if operation == RepositoryState::Bisecting {
                "End Bisect"
            } else {
                "Abort"
            },
            destructive: true,
            action: GitConfirmAction::AbortOperation {
                repository: root,
                operation,
            },
            restore_terminal: app.terminal_focused,
        });
    });
    true
}

/// Abort a confirmed operation in progress.
pub fn abort_operation(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    root: &Path,
    operation: RepositoryState,
) -> bool {
    let Some(repository) = store.get().repository_for_path(root).cloned() else {
        return false;
    };
    run(
        state,
        store,
        repository,
        OperationKind::Other,
        "Aborting…",
        move |service, root| {
            service.write(root, |backend| backend.abort_operation(root, operation))
        },
    )
}

/// Resolve a conflicted file with one side's version and stage it.
pub fn resolve_conflict(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    path: PathBuf,
    side: ConflictSide,
) -> bool {
    let Some(repository) = active_repository(store) else {
        return false;
    };
    if unsaved(
        state,
        &repository.worktree_root,
        std::slice::from_ref(&path),
    ) {
        state.update(|app| {
            app.show_toast("Save or revert the open editor changes before resolving.")
        });
        return false;
    }
    let message = count_label(std::slice::from_ref(&path), "Resolving");
    run(
        state,
        store,
        repository,
        OperationKind::Other,
        message,
        move |service, root| {
            service.write(root, |backend| backend.resolve_conflict(root, &path, side))
        },
    )
}

/// Stash every change, untracked files included.
pub fn stash_all(state: &State<AppState>, store: &State<GitStoreSnapshot>) -> bool {
    let Some(repository) = repository_without_unsaved(state, store, "stashing") else {
        return false;
    };
    if repository.files.is_empty() {
        state.update(|app| app.show_toast("There are no changes to stash."));
        return false;
    }
    run(
        state,
        store,
        repository,
        OperationKind::Stash,
        "Stashing…",
        |service, root| {
            service.write(root, |backend| {
                backend.stash_push(root, None, true, false).map(|_| ())
            })
        },
    )
}

/// The current index of the stash `oid`; indices shift as stashes come and go.
fn stash_index(backend: &CliGitBackend, root: &Path, oid: &str) -> GitResult<usize> {
    backend
        .stash_list(root)?
        .into_iter()
        .find(|stash| stash.oid == oid)
        .map(|stash| stash.index)
        .ok_or_else(|| GitError::new(GitErrorKind::Other, "The stash no longer exists."))
}

/// Apply the stash `oid`, removing it when `pop`.
pub fn apply_stash(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    oid: String,
    pop: bool,
) -> bool {
    let doing = if pop {
        "popping a stash"
    } else {
        "applying a stash"
    };
    let Some(repository) = repository_without_unsaved(state, store, doing) else {
        return false;
    };
    run(
        state,
        store,
        repository,
        OperationKind::Stash,
        if pop {
            "Popping stash…"
        } else {
            "Applying stash…"
        },
        move |service, root| {
            service.write(root, |backend| {
                let index = stash_index(backend, root, &oid)?;
                backend.stash_apply(root, index, pop)
            })
        },
    )
}

/// Ask before dropping the stash `oid`.
pub fn request_drop_stash(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    oid: String,
) -> bool {
    let Some(repository) = active_repository(store) else {
        return false;
    };
    let Some(stash) = repository.stashes.iter().find(|stash| stash.oid == oid) else {
        return false;
    };
    let message = format!("\"{}\" is deleted. This cannot be undone.", stash.message);
    let root = repository.worktree_root;
    state.update(move |app| {
        app.git_confirm = Some(GitConfirm {
            title: "Drop this stash?".into(),
            message,
            confirm_label: "Drop",
            destructive: true,
            action: GitConfirmAction::DropStash {
                repository: root,
                oid,
            },
            restore_terminal: app.terminal_focused,
        });
    });
    true
}

/// Drop a confirmed stash.
pub fn drop_stash(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    root: &Path,
    oid: String,
) -> bool {
    let Some(repository) = store.get().repository_for_path(root).cloned() else {
        return false;
    };
    run(
        state,
        store,
        repository,
        OperationKind::Stash,
        "Dropping stash…",
        move |service, root| {
            service.write(root, |backend| {
                let index = stash_index(backend, root, &oid)?;
                backend.stash_drop(root, index)
            })
        },
    )
}

/// Undo the last commit, keeping its changes staged, and offer its message
/// for the next commit.
pub fn undo_last_commit(state: &State<AppState>, store: &State<GitStoreSnapshot>) -> bool {
    let Some(repository) = active_repository(store) else {
        return false;
    };
    run_with(
        state,
        store,
        repository,
        OperationKind::Other,
        "Undoing last commit…",
        |service, root| service.write(root, |backend| backend.undo_commit(root)),
        |app, message| {
            if app.git_commit_input.text().trim().is_empty() {
                // The input is single-line: keep the subject.
                app.git_commit_input
                    .set_text(message.lines().next().unwrap_or_default());
            }
            app.show_success("Undid the last commit. Its changes are staged.");
        },
    )
}

/// What the title bar's sync button does for the checked-out branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncPlan {
    /// Push a branch with no upstream and make the remote branch its upstream.
    Publish,
    /// Up to date as far as we know: look for new remote commits.
    Fetch,
    /// Behind the upstream: pull, then push local commits if also ahead.
    Pull {
        then_push: bool,
    },
    Push,
}

/// The sync action for `repository`, or `None` without a branch to sync.
pub fn sync_plan(repository: &RepositorySnapshot) -> Option<SyncPlan> {
    let HeadState::Branch(_) = repository.head else {
        return None;
    };
    if repository.upstream.is_none() {
        return Some(SyncPlan::Publish);
    }
    Some(match (repository.ahead, repository.behind) {
        (0, 0) => SyncPlan::Fetch,
        (_, 0) => SyncPlan::Push,
        (ahead, _) => SyncPlan::Pull {
            then_push: ahead > 0,
        },
    })
}

/// Run the active repository's [`SyncPlan`].
pub fn sync(state: &State<AppState>, store: &State<GitStoreSnapshot>) -> bool {
    let Some(repository) = active_repository(store) else {
        return false;
    };
    let (Some(plan), HeadState::Branch(branch)) = (sync_plan(&repository), repository.head.clone())
    else {
        return false;
    };
    if matches!(plan, SyncPlan::Pull { .. })
        && state
            .get()
            .workspace
            .has_dirty_paths_under(&repository.worktree_root)
    {
        state.update(|app| app.show_toast("Save or discard editor changes before pulling."));
        return false;
    }
    let (kind, message) = match plan {
        SyncPlan::Publish => (OperationKind::Push, format!("Publishing {branch}…")),
        SyncPlan::Fetch => (OperationKind::Fetch, "Fetching…".to_owned()),
        SyncPlan::Pull { then_push: true } => (OperationKind::Pull, "Syncing…".to_owned()),
        SyncPlan::Pull { then_push: false } => (OperationKind::Pull, "Pulling…".to_owned()),
        SyncPlan::Push => (OperationKind::Push, "Pushing…".to_owned()),
    };
    run(
        state,
        store,
        repository,
        kind,
        message,
        move |service, root| {
            service.write(root, |backend| match plan {
                SyncPlan::Publish => {
                    let remotes = backend.remotes(root)?;
                    let remote = publish_remote(&remotes).ok_or_else(|| {
                        GitError::new(
                            GitErrorKind::Other,
                            "Add a remote before publishing this branch.",
                        )
                    })?;
                    backend.push(root, Some((remote, &branch)), false)
                }
                SyncPlan::Fetch => backend.fetch(root, None, true),
                SyncPlan::Pull { then_push } => {
                    // Plain `git pull` honors the user's pull.rebase setting.
                    backend.pull(root, false)?;
                    if then_push {
                        backend.push(root, None, false)?;
                    }
                    Ok(())
                }
                SyncPlan::Push => backend.push(root, None, false),
            })
        },
    )
}

/// The remote a new branch is published to: `origin`, else the first one.
fn publish_remote(remotes: &[RemoteInfo]) -> Option<&str> {
    remotes
        .iter()
        .find(|remote| remote.name == "origin")
        .or_else(|| remotes.first())
        .map(|remote| remote.name.as_str())
}

/// List `repository`'s branches into the open branch picker opened as
/// `sequence`. Read-only, so it runs beside any write operation.
pub fn load_branches(state: &State<AppState>, repository: PathBuf, sequence: u64) {
    let Some(application) = state.get().git_application else {
        return;
    };
    let worker_state = state.clone();
    let spawned = thread::Builder::new()
        .name("loom-git-branches".into())
        .spawn(move || {
            let result =
                crate::git::service().and_then(|service| service.backend().branches(&repository));
            application.post(move || {
                worker_state.update(move |app| {
                    if let Some(picker) = app
                        .branch_picker
                        .as_mut()
                        .filter(|picker| picker.sequence == sequence)
                    {
                        picker.listing = BranchListing::from_result(result);
                    }
                });
            });
        })
        .is_ok();
    if !spawned {
        state.update(move |app| {
            if let Some(picker) = app.branch_picker.as_mut() {
                picker.listing = BranchListing::Failed("Could not start Git worker.".into());
            }
        });
    }
}

/// Fetch every remote, then reload the open branch picker's list.
pub fn fetch_branches(state: &State<AppState>, store: &State<GitStoreSnapshot>) -> bool {
    let Some(picker) = state.get().branch_picker else {
        return false;
    };
    let Some(repository) = store.get().repository_for_path(&picker.repository).cloned() else {
        return false;
    };
    let sequence = picker.sequence;
    run_with(
        state,
        store,
        repository,
        OperationKind::Fetch,
        "Fetching…",
        |service, root| {
            service.write(root, |backend| {
                backend.fetch(root, None, true)?;
                backend.branches(root)
            })
        },
        move |app, branches| {
            if let Some(picker) = app
                .branch_picker
                .as_mut()
                .filter(|picker| picker.sequence == sequence)
            {
                picker.listing = BranchListing::from_result(Ok(branches));
            }
        },
    )
}

/// How a checkout treats uncommitted changes that are in its way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StashMode {
    /// Leave them alone; Git refuses a switch that would overwrite them.
    Refuse,
    /// Stash them and leave them in the stash.
    Keep,
    /// Stash them and reapply them on the new branch.
    Carry,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CheckoutOutcome {
    Switched,
    /// Switched; the changes wait in the stash.
    Stashed,
    /// Switched, but reapplying the changes conflicted. The stash is kept.
    CarryConflicted,
    /// Refused: these files have changes the switch would overwrite.
    Blocked(Vec<String>),
}

/// Check out `checkout` with `backend`, handling local changes per `mode`.
/// A switch that fails after stashing restores the stash, so a failure
/// never leaves the changes stashed away.
pub fn checkout_with(
    backend: &CliGitBackend,
    root: &Path,
    checkout: &Checkout,
    mode: StashMode,
) -> GitResult<CheckoutOutcome> {
    let switch = || match checkout {
        Checkout::Switch(name) => backend.switch(root, name),
        Checkout::Create(name) => backend.create_branch(root, name, None),
        Checkout::Track(remote) => backend.switch_tracking(root, remote),
    };
    if mode == StashMode::Refuse {
        return match switch() {
            Ok(()) => Ok(CheckoutOutcome::Switched),
            Err(error) if error.kind == GitErrorKind::DirtyWorktree => {
                Ok(CheckoutOutcome::Blocked(overwritten_files(&error)))
            }
            Err(error) => Err(error),
        };
    }
    // `git stash push` succeeds without stashing when there is nothing to
    // stash; only a new top entry is ours to pop.
    let top = |backend: &CliGitBackend| {
        backend
            .stash_list(root)
            .map(|stashes| stashes.first().map(|stash| stash.oid.clone()))
    };
    let before = top(backend)?;
    let message = format!("Loom: before switching to {}", checkout.target());
    backend.stash_push(root, Some(&message), true, false)?;
    let stashed = top(backend)? != before;
    if let Err(error) = switch() {
        if stashed && let Err(restore) = backend.stash_pop(root) {
            return Err(GitError::new(
                GitErrorKind::Other,
                "Could not switch branches; your changes are kept in the stash.",
            )
            .with_detail(format!(
                "{}\n{}",
                error.user_message(),
                restore.user_message()
            )));
        }
        return Err(error);
    }
    if !stashed {
        return Ok(CheckoutOutcome::Switched);
    }
    if mode == StashMode::Keep {
        return Ok(CheckoutOutcome::Stashed);
    }
    Ok(match backend.stash_pop(root) {
        Ok(()) => CheckoutOutcome::Switched,
        Err(_) => CheckoutOutcome::CarryConflicted,
    })
}

/// The files Git lists, tab-indented, as blocking an operation.
fn overwritten_files(error: &GitError) -> Vec<String> {
    error
        .detail
        .as_deref()
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.strip_prefix('\t'))
        .map(|path| path.trim().to_owned())
        .filter(|path| !path.is_empty())
        .collect()
}

/// Check out `checkout` in the repository at `root`.
pub fn checkout(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    root: &Path,
    checkout: Checkout,
    mode: StashMode,
) -> bool {
    let Some(repository) = store.get().repository_for_path(root).cloned() else {
        return false;
    };
    if state
        .get()
        .workspace
        .has_dirty_paths_under(&repository.worktree_root)
    {
        state.update(|app| {
            app.show_toast("Save or discard editor changes before switching branches.")
        });
        return false;
    }
    if store.get().operation.is_some() {
        state.update(|app| app.show_toast("Wait for the running Git operation to finish."));
        return false;
    }
    let target = checkout.target().to_owned();
    let message = match (&checkout, mode) {
        (Checkout::Create(name), _) => format!("Creating {name}…"),
        (_, StashMode::Refuse) => format!("Switching to {target}…"),
        (_, StashMode::Keep | StashMode::Carry) => format!("Stashing and switching to {target}…"),
    };
    let repository_root = repository.worktree_root.clone();
    let blocked_checkout = checkout.clone();
    run_with(
        state,
        store,
        repository,
        OperationKind::Checkout,
        message,
        move |service, root| {
            service.write(root, |backend| {
                checkout_with(backend, root, &checkout, mode)
            })
        },
        move |app, outcome| match outcome {
            CheckoutOutcome::Switched => {}
            CheckoutOutcome::Stashed => {
                app.show_success(format!(
                    "Switched to {target}. Your changes are in the stash."
                ));
            }
            CheckoutOutcome::CarryConflicted => {
                app.show_error(format!(
                    "Switched to {target}, but your changes conflict with it. Resolve the \
                     conflicts; the stash was kept."
                ));
            }
            CheckoutOutcome::Blocked(files) => {
                app.stash_switch = Some(StashSwitchRequest {
                    repository: repository_root,
                    checkout: blocked_checkout,
                    files,
                    restore_terminal: app.terminal_focused,
                });
            }
        },
    )
}

fn active_repository(store: &State<GitStoreSnapshot>) -> Option<RepositorySnapshot> {
    store.get().active().cloned()
}

fn run(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    repository: RepositorySnapshot,
    kind: OperationKind,
    message: impl Into<String>,
    action: impl FnOnce(&GitService, &std::path::Path) -> crate::git::GitResult<()> + Send + 'static,
) -> bool {
    run_with(state, store, repository, kind, message, action, |_, ()| {})
}

/// [`run`], handing the action's result to `on_success` on the UI thread.
fn run_with<T: Send + 'static>(
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    repository: RepositorySnapshot,
    kind: OperationKind,
    message: impl Into<String>,
    action: impl FnOnce(&GitService, &std::path::Path) -> crate::git::GitResult<T> + Send + 'static,
    on_success: impl FnOnce(&mut AppState, T) + Send + 'static,
) -> bool {
    if store.get().operation.is_some() {
        return false;
    }
    let app = state.get();
    let Some(application) = app.git_application else {
        return false;
    };
    let poll_control = app.git_poll_control;
    let message = message.into();
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
                Ok(value) => {
                    state.update(move |app| on_success(app, value));
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::git::{RepositoryFeatures, RepositoryId, RepositoryState, UpstreamState};

    fn repository(head: HeadState, upstream: bool, ahead: u32, behind: u32) -> RepositorySnapshot {
        RepositorySnapshot {
            id: RepositoryId(1),
            worktree_root: PathBuf::from("repo"),
            git_dir: PathBuf::from("repo/.git"),
            common_dir: PathBuf::from("repo/.git"),
            head,
            upstream: upstream.then(|| UpstreamState {
                name: "origin/main".into(),
            }),
            ahead,
            behind,
            files: Arc::new(BTreeMap::new()),
            ignored: Arc::new(BTreeSet::new()),
            repository_state: RepositoryState::Normal,
            operation_step: None,
            stashes: Default::default(),
            features: RepositoryFeatures::default(),
            generation: 0,
        }
    }

    #[test]
    fn sync_plan_follows_the_upstream_state() {
        let branch = || HeadState::Branch("main".into());
        assert_eq!(
            sync_plan(&repository(branch(), false, 0, 0)),
            Some(SyncPlan::Publish)
        );
        assert_eq!(
            sync_plan(&repository(branch(), true, 0, 0)),
            Some(SyncPlan::Fetch)
        );
        assert_eq!(
            sync_plan(&repository(branch(), true, 2, 0)),
            Some(SyncPlan::Push)
        );
        assert_eq!(
            sync_plan(&repository(branch(), true, 0, 3)),
            Some(SyncPlan::Pull { then_push: false })
        );
        assert_eq!(
            sync_plan(&repository(branch(), true, 2, 3)),
            Some(SyncPlan::Pull { then_push: true })
        );
        assert_eq!(
            sync_plan(&repository(HeadState::Detached("abc".into()), false, 0, 0)),
            None
        );
        assert_eq!(
            sync_plan(&repository(HeadState::Unborn("main".into()), false, 0, 0)),
            None
        );
    }

    #[test]
    fn publishes_to_origin_or_the_first_remote() {
        let remote = |name: &str| RemoteInfo {
            name: name.into(),
            fetch_url: None,
            push_url: None,
        };
        assert_eq!(publish_remote(&[]), None);
        assert_eq!(
            publish_remote(&[remote("fork"), remote("origin")]),
            Some("origin")
        );
        assert_eq!(
            publish_remote(&[remote("fork"), remote("upstream")]),
            Some("fork")
        );
    }

    #[test]
    fn lists_the_files_git_says_would_be_overwritten() {
        let error = GitError::from_stderr(
            b"error: Your local changes to the following files would be overwritten by checkout:\n\
\tsrc/main.rs\n\tREADME.md\nPlease commit your changes or stash them before you switch branches.\nAborting\n",
            Some(1),
        );
        assert_eq!(error.kind, GitErrorKind::DirtyWorktree);
        assert_eq!(overwritten_files(&error), ["src/main.rs", "README.md"]);
    }

    #[test]
    fn real_repository_switches_with_stashed_changes() {
        let Some((root, backend)) = crate::git::backend::tests::repository() else {
            return;
        };
        let file = root.join("file.txt");
        let commit = |message: &str| {
            backend.stage(&root, &[PathBuf::from("file.txt")]).unwrap();
            backend.commit(&root, message, false, false).unwrap();
        };
        let current = || {
            backend
                .branches(&root)
                .unwrap()
                .into_iter()
                .find(|branch| branch.current)
                .map(|branch| branch.name)
                .unwrap()
        };
        std::fs::write(&file, "base\n").unwrap();
        commit("base");
        backend.create_branch(&root, "other", None).unwrap();
        std::fs::write(&file, "other\n").unwrap();
        commit("other");
        backend.switch(&root, "main").unwrap();
        let other = Checkout::Switch("other".into());
        let main = Checkout::Switch("main".into());

        // Nothing to stash: the stash modes just switch.
        assert_eq!(
            checkout_with(&backend, &root, &other, StashMode::Carry).unwrap(),
            CheckoutOutcome::Switched
        );
        assert!(backend.stash_list(&root).unwrap().is_empty());
        backend.switch(&root, "main").unwrap();

        // A change the switch would overwrite blocks it.
        std::fs::write(&file, "local\n").unwrap();
        assert_eq!(
            checkout_with(&backend, &root, &other, StashMode::Refuse).unwrap(),
            CheckoutOutcome::Blocked(vec!["file.txt".into()])
        );
        assert_eq!(current(), "main");

        // Keep: switched, the change waits in the stash.
        assert_eq!(
            checkout_with(&backend, &root, &other, StashMode::Keep).unwrap(),
            CheckoutOutcome::Stashed
        );
        assert_eq!(current(), "other");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "other\n");
        assert_eq!(backend.stash_list(&root).unwrap().len(), 1);

        // Carry: back on main the stashed change reapplies cleanly.
        backend.stash_drop(&root, 0).unwrap();
        backend.switch(&root, "main").unwrap();
        std::fs::write(root.join("new.txt"), "untracked\n").unwrap();
        assert_eq!(
            checkout_with(&backend, &root, &other, StashMode::Carry).unwrap(),
            CheckoutOutcome::Switched
        );
        assert_eq!(current(), "other");
        assert_eq!(
            std::fs::read_to_string(root.join("new.txt")).unwrap(),
            "untracked\n"
        );
        assert!(backend.stash_list(&root).unwrap().is_empty());

        // Carry onto a branch the change conflicts with keeps the stash.
        std::fs::write(&file, "conflicting\n").unwrap();
        assert_eq!(
            checkout_with(&backend, &root, &main, StashMode::Carry).unwrap(),
            CheckoutOutcome::CarryConflicted
        );
        assert_eq!(current(), "main");
        assert_eq!(backend.stash_list(&root).unwrap().len(), 1);

        // A failed switch restores what it stashed.
        backend
            .runner()
            .run(
                crate::git::command::GitCommand::new()
                    .cwd(&root)
                    .args(["reset", "--hard"]),
            )
            .unwrap();
        backend.stash_drop(&root, 0).unwrap();
        std::fs::write(&file, "kept\n").unwrap();
        let missing = Checkout::Switch("missing".into());
        assert!(checkout_with(&backend, &root, &missing, StashMode::Keep).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "kept\n");
        assert!(backend.stash_list(&root).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }
}
