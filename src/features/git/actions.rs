//! What Source Control actions operate on: checkouts and their handling of
//! local changes, change-block actions, branch listings, and the sync plan
//! for the checked-out branch.

use std::path::PathBuf;
use std::sync::Arc;

use super::{BranchInfo, GitResult, HeadState, RepositorySnapshot, WorktreeInfo};

/// What choosing a branch picker row does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Checkout {
    Switch(String),
    Create(String),
    /// Create a local branch tracking this remote branch.
    Track(String),
    OpenWorktree {
        branch: String,
        path: PathBuf,
    },
}

impl Checkout {
    /// The branch being checked out, as the user chose it.
    pub fn target(&self) -> &str {
        match self {
            Self::Switch(name) | Self::Create(name) | Self::Track(name) => name,
            Self::OpenWorktree { branch, .. } => branch,
        }
    }
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

/// What to do with one change block of an open diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockAction {
    Stage,
    Unstage,
    Discard,
}

/// Branches and worktrees listed for the branch picker.
#[derive(Clone, Debug, PartialEq)]
pub enum BranchListing {
    Loading,
    Ready {
        branches: Arc<[BranchInfo]>,
        worktrees: Arc<[WorktreeInfo]>,
    },
    Failed(String),
}

impl BranchListing {
    pub fn from_result(result: GitResult<(Vec<BranchInfo>, Vec<WorktreeInfo>)>) -> Self {
        match result {
            Ok((branches, worktrees)) => Self::Ready {
                branches: branches.into(),
                worktrees: worktrees.into(),
            },
            Err(error) => Self::Failed(error.user_message()),
        }
    }

    pub fn branches(&self) -> &[BranchInfo] {
        match self {
            Self::Ready { branches, .. } => branches,
            Self::Loading | Self::Failed(_) => &[],
        }
    }

    pub fn worktrees(&self) -> &[WorktreeInfo] {
        match self {
            Self::Ready { worktrees, .. } => worktrees,
            Self::Loading | Self::Failed(_) => &[],
        }
    }
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
