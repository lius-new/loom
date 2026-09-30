use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::command::{GitCommand, GitCommandRunner};
use super::error::{GitErrorKind, GitResult};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredRepository {
    pub worktree_root: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    pub bare: bool,
}

pub fn discover(
    runner: &GitCommandRunner,
    workspace_roots: &[PathBuf],
) -> GitResult<Vec<DiscoveredRepository>> {
    let mut seen = HashSet::new();
    let mut repositories = Vec::new();
    for root in workspace_roots {
        if let Some(repository) = discover_one(runner, root)?
            && seen.insert(repository.worktree_root.clone())
        {
            repositories.push(repository);
        }
        for child in direct_children(root) {
            if let Some(repository) = discover_one(runner, &child)?
                && seen.insert(repository.worktree_root.clone())
            {
                repositories.push(repository);
            }
        }
    }
    repositories.sort_by(|left, right| left.worktree_root.cmp(&right.worktree_root));
    Ok(repositories)
}

/// Fast startup discovery. Workspace roots that contain `.git` are resolved
/// without spawning a process; only roots opened from inside a parent
/// repository fall back to `rev-parse`. Nested repositories are discovered by
/// the later detailed refresh.
pub fn discover_fast(
    runner: &GitCommandRunner,
    workspace_roots: &[PathBuf],
) -> GitResult<Vec<DiscoveredRepository>> {
    let mut seen = HashSet::new();
    let mut repositories = Vec::new();
    for root in workspace_roots {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        let repository = if root.join(".git").exists() {
            Some(from_git_marker(&root)?)
        } else {
            discover_one(runner, &root)?
        };
        if let Some(repository) = repository
            && seen.insert(repository.worktree_root.clone())
        {
            repositories.push(repository);
        }
    }
    Ok(repositories)
}

fn from_git_marker(root: &Path) -> GitResult<DiscoveredRepository> {
    let marker = root.join(".git");
    let git_dir = if marker.is_dir() {
        marker
    } else {
        let contents = std::fs::read_to_string(&marker)?;
        let value = contents
            .trim()
            .strip_prefix("gitdir:")
            .map(str::trim)
            .ok_or_else(|| {
                super::error::GitError::new(
                    GitErrorKind::InvalidOutput,
                    "The .git file does not contain a gitdir path.",
                )
            })?;
        let path = PathBuf::from(value);
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    };
    let git_dir = std::fs::canonicalize(&git_dir).unwrap_or(git_dir);
    let common_dir = std::fs::read_to_string(git_dir.join("commondir"))
        .ok()
        .map(|value| {
            let path = PathBuf::from(value.trim());
            if path.is_absolute() {
                path
            } else {
                git_dir.join(path)
            }
        })
        .map(|path| std::fs::canonicalize(&path).unwrap_or(path))
        .unwrap_or_else(|| git_dir.clone());
    Ok(DiscoveredRepository {
        worktree_root: root.to_path_buf(),
        git_dir,
        common_dir,
        bare: false,
    })
}

pub fn discover_one(
    runner: &GitCommandRunner,
    path: &Path,
) -> GitResult<Option<DiscoveredRepository>> {
    let command = GitCommand::new()
        .cwd(path)
        .args([
            "rev-parse",
            "--path-format=absolute",
            "--show-toplevel",
            "--absolute-git-dir",
            "--git-common-dir",
            "--is-bare-repository",
        ])
        .read_only();
    let output = match runner.run(command) {
        Ok(output) => output,
        Err(error) if error.kind == GitErrorKind::NotRepository => return Ok(None),
        Err(error) => return Err(error),
    };
    let lines = output
        .stdout_text()?
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if lines.len() < 4 {
        return Ok(None);
    }
    Ok(Some(DiscoveredRepository {
        worktree_root: PathBuf::from(&lines[0]),
        git_dir: PathBuf::from(&lines[1]),
        common_dir: PathBuf::from(&lines[2]),
        bare: lines[3] == "true",
    }))
}

fn direct_children(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_dir())
                .map(|_| entry.path())
                .filter(|path| path.join(".git").exists())
        })
        .collect()
}
