//! What Source Control actions operate on: checkouts and their handling of
//! local changes, change-block actions, branch listings, and the sync plan
//! for the checked-out branch.

use std::path::PathBuf;
use std::sync::Arc;

use super::{BranchInfo, GitResult, HeadState, RepositorySnapshot, WorktreeInfo};

/// Validate the user-facing tag name before submitting it to Git.
pub fn valid_tag_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.ends_with('.')
        && !name.contains("..")
        && !name.contains("@{")
        && name
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.') && !part.ends_with(".lock"))
        && !name.chars().any(|c| {
            c.is_control()
                || c.is_whitespace()
                || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
}

#[cfg(test)]
mod tag_tests {
    use super::valid_tag_name;
    #[test]
    fn tag_names_accept_release_paths_and_reject_invalid_ref_components() {
        for name in ["v1.0.0", "releases/2026.10", "标签/发布"] {
            assert!(valid_tag_name(name), "{name}");
        }
        for name in [
            "",
            "-delete",
            "bad name",
            "bad\nname",
            "one..two",
            "one@{two",
            "/v1",
            "v1/",
            "one//two",
            "one/.two",
            "one.lock/two",
            "v1.",
            "v1~",
            "v1^",
            "v1:",
            "v1?",
            "v1*",
            "v1[",
            "v1\\",
        ] {
            assert!(!valid_tag_name(name), "{name}");
        }
    }
}

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
