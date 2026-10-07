use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::command::{CancellationToken, GitCommand, GitCommandRunner, nul_pathspec};
use super::diff::{UnifiedDiff, parse_unified};
use super::discovery::{self, DiscoveredRepository};
use super::error::{GitError, GitErrorKind, GitResult};
use super::parser::{parse_branches, parse_log, parse_numstat, parse_porcelain_v2};
use super::runtime::GitRuntime;
use super::types::*;

pub trait GitBackend: Send + Sync {
    fn runtime(&self) -> &GitRuntime;
    fn discover(&self, roots: &[PathBuf]) -> GitResult<Vec<DiscoveredRepository>>;
    fn status(
        &self,
        repository: &DiscoveredRepository,
        id: RepositoryId,
        generation: u64,
    ) -> GitResult<RepositorySnapshot>;
    fn status_fast(
        &self,
        repository: &DiscoveredRepository,
        id: RepositoryId,
        generation: u64,
    ) -> GitResult<RepositorySnapshot>;
    fn clone_repository(
        &self,
        url: &str,
        target: &Path,
        cancellation: &CancellationToken,
    ) -> GitResult<()>;
}

#[derive(Clone, Debug)]
pub struct CliGitBackend {
    runner: GitCommandRunner,
}

impl CliGitBackend {
    pub fn new(runtime: GitRuntime) -> Self {
        Self {
            runner: GitCommandRunner::new(runtime),
        }
    }

    pub fn runner(&self) -> &GitCommandRunner {
        &self.runner
    }

    pub fn diff(
        &self,
        repository: &Path,
        target: DiffTarget,
        paths: &[PathBuf],
    ) -> GitResult<String> {
        self.diff_with_context(repository, target, paths, None)
    }

    /// Produce a whole-file diff suitable for a document-style diff editor.
    /// A very large context keeps unchanged lines in the same parsed hunk
    /// without requiring separate `git show` calls for the index and HEAD.
    pub fn full_diff(
        &self,
        repository: &Path,
        target: DiffTarget,
        paths: &[PathBuf],
    ) -> GitResult<String> {
        self.diff_with_context(repository, target, paths, Some(1_000_000))
    }

    fn diff_with_context(
        &self,
        repository: &Path,
        target: DiffTarget,
        paths: &[PathBuf],
        context: Option<usize>,
    ) -> GitResult<String> {
        let mut args = vec![OsString::from("diff"), OsString::from("--no-ext-diff")];
        if let Some(context) = context {
            args.push(OsString::from(format!("--unified={context}")));
        }
        match target {
            DiffTarget::HeadToIndex => args.push("--cached".into()),
            DiffTarget::IndexToWorktree => {}
            DiffTarget::HeadToWorktree => args.push("HEAD".into()),
        }
        args.push("--".into());
        args.extend(paths.iter().map(|path| path.as_os_str().to_owned()));
        self.runner
            .run(GitCommand::new().cwd(repository).args(args).read_only())?
            .stdout_text()
    }

    pub fn parsed_diff(
        &self,
        repository: &Path,
        target: DiffTarget,
        paths: &[PathBuf],
    ) -> GitResult<UnifiedDiff> {
        parse_unified(&self.diff(repository, target, paths)?)
    }

    pub fn parsed_full_diff(
        &self,
        repository: &Path,
        target: DiffTarget,
        paths: &[PathBuf],
    ) -> GitResult<UnifiedDiff> {
        parse_unified(&self.full_diff(repository, target, paths)?)
    }

    pub fn stage(&self, repository: &Path, paths: &[PathBuf]) -> GitResult<()> {
        self.pathspec_command(repository, &["add"], paths)?;
        Ok(())
    }

    pub fn unstage(&self, repository: &Path, paths: &[PathBuf]) -> GitResult<()> {
        if self.runtime().capabilities.restore {
            self.pathspec_command(repository, &["restore", "--staged"], paths)?;
        } else {
            self.pathspec_command(repository, &["reset", "HEAD"], paths)?;
        }
        Ok(())
    }

    pub fn restore_worktree(&self, repository: &Path, paths: &[PathBuf]) -> GitResult<()> {
        if !self.runtime().capabilities.restore {
            return Err(GitError::new(
                GitErrorKind::Unsupported,
                "This Git runtime does not support safe restore.",
            ));
        }
        self.pathspec_command(repository, &["restore"], paths)?;
        Ok(())
    }

    pub fn apply_index_patch(
        &self,
        repository: &Path,
        patch: &[u8],
        reverse: bool,
        expected_index_tree: Option<&str>,
    ) -> GitResult<()> {
        if let Some(expected) = expected_index_tree {
            let current = self.index_tree(repository)?;
            if current.trim() != expected.trim() {
                return Err(GitError::new(
                    GitErrorKind::Conflict,
                    "The index changed after this diff was generated. Refresh and try again.",
                ));
            }
        }
        let mut args = vec!["apply", "--cached", "--recount", "--whitespace=nowarn"];
        if reverse {
            args.push("--reverse");
        }
        args.push("-");
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(args)
                .stdin(patch.to_vec()),
        )?;
        Ok(())
    }

    pub fn apply_worktree_patch(
        &self,
        repository: &Path,
        patch: &[u8],
        reverse: bool,
    ) -> GitResult<()> {
        let mut args = vec!["apply", "--recount", "--whitespace=nowarn"];
        if reverse {
            args.push("--reverse");
        }
        args.push("-");
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(args)
                .stdin(patch.to_vec()),
        )?;
        Ok(())
    }

    pub fn index_tree(&self, repository: &Path) -> GitResult<String> {
        self.runner
            .run(
                GitCommand::new()
                    .cwd(repository)
                    .args(["write-tree"])
                    .read_only(),
            )?
            .stdout_text()
            .map(|value| value.trim().to_owned())
    }

    pub fn commit(
        &self,
        repository: &Path,
        message: &str,
        amend: bool,
        signoff: bool,
    ) -> GitResult<String> {
        let mut args = vec!["commit", "--file=-"];
        if amend {
            args.push("--amend");
        }
        if signoff {
            args.push("--signoff");
        }
        let output = self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(args)
                .stdin(message.as_bytes().to_vec()),
        )?;
        output.stdout_text()
    }

    pub fn undo_commit(&self, repository: &Path) -> GitResult<()> {
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(["reset", "--soft", "HEAD^"]),
        )?;
        Ok(())
    }

    pub fn branches(&self, repository: &Path) -> GitResult<Vec<BranchInfo>> {
        let output = self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args([
                    "for-each-ref",
                    "--format=%(refname)%09%(refname:short)%09%(HEAD)%09%(objectname)%09%(upstream:short)",
                    "refs/heads",
                    "refs/remotes",
                ])
                .read_only(),
        )?;
        parse_branches(&output.stdout)
    }

    pub fn tags(&self, repository: &Path) -> GitResult<Vec<String>> {
        let text = self
            .runner
            .run(
                GitCommand::new()
                    .cwd(repository)
                    .args(["tag", "--list", "--sort=-creatordate"])
                    .read_only(),
            )?
            .stdout_text()?;
        Ok(text.lines().map(str::to_owned).collect())
    }

    pub fn switch(&self, repository: &Path, branch: &str) -> GitResult<()> {
        validate_ref(branch)?;
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(["switch", "--", branch]),
        )?;
        Ok(())
    }

    pub fn create_branch(
        &self,
        repository: &Path,
        branch: &str,
        start: Option<&str>,
    ) -> GitResult<()> {
        validate_ref(branch)?;
        if let Some(start) = start {
            validate_ref(start)?;
        }
        let mut args = vec![
            OsString::from("switch"),
            OsString::from("-c"),
            branch.into(),
        ];
        if let Some(start) = start {
            args.push(start.into());
        }
        self.runner
            .run(GitCommand::new().cwd(repository).args(args))?;
        Ok(())
    }

    pub fn delete_branch(&self, repository: &Path, branch: &str, force: bool) -> GitResult<()> {
        validate_ref(branch)?;
        self.runner.run(GitCommand::new().cwd(repository).args([
            "branch",
            if force { "-D" } else { "-d" },
            "--",
            branch,
        ]))?;
        Ok(())
    }

    pub fn rename_branch(&self, repository: &Path, old: &str, new: &str) -> GitResult<()> {
        validate_ref(old)?;
        validate_ref(new)?;
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(["branch", "--move", "--", old, new]),
        )?;
        Ok(())
    }

    pub fn create_tag(&self, repository: &Path, tag: &str, target: Option<&str>) -> GitResult<()> {
        validate_ref(tag)?;
        let mut args = vec![OsString::from("tag"), OsString::from("--"), tag.into()];
        if let Some(target) = target {
            validate_ref(target)?;
            args.push(target.into());
        }
        self.runner
            .run(GitCommand::new().cwd(repository).args(args))?;
        Ok(())
    }

    pub fn delete_tag(&self, repository: &Path, tag: &str) -> GitResult<()> {
        validate_ref(tag)?;
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(["tag", "--delete", "--", tag]),
        )?;
        Ok(())
    }

    pub fn history(
        &self,
        repository: &Path,
        revision: Option<&str>,
        path: Option<&Path>,
        skip: usize,
        limit: usize,
    ) -> GitResult<Vec<CommitSummary>> {
        let mut args = vec![
            OsString::from("log"),
            OsString::from("--date-order"),
            OsString::from(format!("--skip={skip}")),
            OsString::from(format!("--max-count={}", limit.clamp(1, 500))),
            OsString::from("--decorate=short"),
            OsString::from("--format=%x1e%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%D"),
        ];
        if let Some(revision) = revision {
            validate_ref(revision)?;
            args.push(revision.into());
        }
        if let Some(path) = path {
            args.push("--follow".into());
            args.push("--".into());
            args.push(path.as_os_str().to_owned());
        }
        let text = self
            .runner
            .run(GitCommand::new().cwd(repository).args(args).read_only())?
            .stdout_text()?;
        parse_log(&text)
    }

    pub fn show(&self, repository: &Path, revision: &str) -> GitResult<String> {
        validate_ref(revision)?;
        self.runner
            .run(
                GitCommand::new()
                    .cwd(repository)
                    .args(["show", "--no-ext-diff", "--format=fuller", "--", revision])
                    .read_only(),
            )?
            .stdout_text()
    }

    pub fn stash_list(&self, repository: &Path) -> GitResult<Vec<StashInfo>> {
        let output = self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(["stash", "list", "--format=%gd%x1f%H%x1f%gs%x00"])
                .read_only(),
        )?;
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
            .map(|record| {
                let text = String::from_utf8_lossy(record);
                let fields = text.trim().split('\x1f').collect::<Vec<_>>();
                if fields.len() != 3 {
                    return Err(GitError::new(
                        GitErrorKind::InvalidOutput,
                        "Invalid stash output.",
                    ));
                }
                let index = fields[0]
                    .strip_prefix("stash@{")
                    .and_then(|value| value.strip_suffix('}'))
                    .and_then(|value| value.parse().ok())
                    .unwrap_or_default();
                Ok(StashInfo {
                    index,
                    oid: fields[1].to_owned(),
                    message: fields[2].to_owned(),
                })
            })
            .collect()
    }

    pub fn stash_push(
        &self,
        repository: &Path,
        message: Option<&str>,
        include_untracked: bool,
        keep_index: bool,
    ) -> GitResult<String> {
        let mut args = vec![OsString::from("stash"), OsString::from("push")];
        if include_untracked {
            args.push("--include-untracked".into());
        }
        if keep_index {
            args.push("--keep-index".into());
        }
        if let Some(message) = message {
            args.extend([OsString::from("--message"), message.into()]);
        }
        self.runner
            .run(GitCommand::new().cwd(repository).args(args))?
            .stdout_text()
    }

    pub fn stash_apply(&self, repository: &Path, index: usize, pop: bool) -> GitResult<()> {
        let reference = format!("stash@{{{index}}}");
        self.runner.run(GitCommand::new().cwd(repository).args([
            "stash",
            if pop { "pop" } else { "apply" },
            "--index",
            &reference,
        ]))?;
        Ok(())
    }

    pub fn stash_drop(&self, repository: &Path, index: usize) -> GitResult<()> {
        let reference = format!("stash@{{{index}}}");
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(["stash", "drop", &reference]),
        )?;
        Ok(())
    }

    pub fn worktrees(&self, repository: &Path) -> GitResult<Vec<WorktreeInfo>> {
        let output = self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(["worktree", "list", "--porcelain", "-z"])
                .read_only(),
        )?;
        parse_worktrees(&output.stdout)
    }

    pub fn add_worktree(
        &self,
        repository: &Path,
        path: &Path,
        branch: Option<&str>,
        create_branch: bool,
    ) -> GitResult<()> {
        let mut args = vec![OsString::from("worktree"), OsString::from("add")];
        if create_branch {
            let branch = branch
                .ok_or_else(|| GitError::new(GitErrorKind::Other, "A branch name is required."))?;
            validate_ref(branch)?;
            args.extend([OsString::from("-b"), branch.into()]);
        }
        args.push(path.as_os_str().to_owned());
        if let Some(branch) = branch.filter(|_| !create_branch) {
            validate_ref(branch)?;
            args.push(branch.into());
        }
        self.runner
            .run(GitCommand::new().cwd(repository).args(args))?;
        Ok(())
    }

    pub fn remove_worktree(&self, repository: &Path, path: &Path, force: bool) -> GitResult<()> {
        let mut args = vec![OsString::from("worktree"), OsString::from("remove")];
        if force {
            args.push("--force".into());
        }
        args.push(path.as_os_str().to_owned());
        self.runner
            .run(GitCommand::new().cwd(repository).args(args))?;
        Ok(())
    }

    pub fn remotes(&self, repository: &Path) -> GitResult<Vec<RemoteInfo>> {
        let text = self
            .runner
            .run(
                GitCommand::new()
                    .cwd(repository)
                    .args(["remote", "-v"])
                    .read_only(),
            )?
            .stdout_text()?;
        let mut values = BTreeMap::<String, RemoteInfo>::new();
        for line in text.lines() {
            let mut fields = line.split_whitespace();
            let (Some(name), Some(url), Some(kind)) = (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let remote = values.entry(name.to_owned()).or_insert_with(|| RemoteInfo {
                name: name.to_owned(),
                fetch_url: None,
                push_url: None,
            });
            match kind {
                "(fetch)" => remote.fetch_url = Some(url.to_owned()),
                "(push)" => remote.push_url = Some(url.to_owned()),
                _ => {}
            }
        }
        Ok(values.into_values().collect())
    }

    pub fn add_remote(&self, repository: &Path, name: &str, url: &str) -> GitResult<()> {
        validate_ref(name)?;
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(["remote", "add", "--", name, url]),
        )?;
        Ok(())
    }

    pub fn remove_remote(&self, repository: &Path, name: &str) -> GitResult<()> {
        validate_ref(name)?;
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(["remote", "remove", "--", name]),
        )?;
        Ok(())
    }

    pub fn fetch(&self, repository: &Path, remote: Option<&str>, prune: bool) -> GitResult<()> {
        let mut args = vec!["fetch", "--progress"];
        if prune {
            args.push("--prune");
        }
        if let Some(remote) = remote {
            validate_ref(remote)?;
            args.push(remote);
        }
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(args)
                .allow_prompt()
                .timeout(Duration::from_secs(60 * 30)),
        )?;
        Ok(())
    }

    pub fn pull(&self, repository: &Path, rebase: bool) -> GitResult<()> {
        let mut args = vec!["pull", "--progress"];
        if rebase {
            args.push("--rebase");
        }
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(args)
                .allow_prompt()
                .timeout(Duration::from_secs(60 * 30)),
        )?;
        Ok(())
    }

    pub fn push(
        &self,
        repository: &Path,
        set_upstream: Option<(&str, &str)>,
        force_with_lease: bool,
    ) -> GitResult<()> {
        let mut args = vec![OsString::from("push"), OsString::from("--progress")];
        if force_with_lease {
            args.push("--force-with-lease".into());
        }
        if let Some((remote, branch)) = set_upstream {
            validate_ref(remote)?;
            validate_ref(branch)?;
            args.extend([
                OsString::from("--set-upstream"),
                remote.into(),
                branch.into(),
            ]);
        }
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(args)
                .allow_prompt()
                .timeout(Duration::from_secs(60 * 30)),
        )?;
        Ok(())
    }

    pub fn continue_operation(&self, repository: &Path, state: RepositoryState) -> GitResult<()> {
        let args = match state {
            RepositoryState::MergeInProgress => vec!["merge", "--continue"],
            RepositoryState::RebaseInProgress => vec!["rebase", "--continue"],
            RepositoryState::CherryPickInProgress => vec!["cherry-pick", "--continue"],
            RepositoryState::RevertInProgress => vec!["revert", "--continue"],
            _ => {
                return Err(GitError::new(
                    GitErrorKind::Unsupported,
                    "No Git operation is active.",
                ));
            }
        };
        self.runner
            .run(GitCommand::new().cwd(repository).args(args))?;
        Ok(())
    }

    pub fn abort_operation(&self, repository: &Path, state: RepositoryState) -> GitResult<()> {
        let args = match state {
            RepositoryState::MergeInProgress => vec!["merge", "--abort"],
            RepositoryState::RebaseInProgress => vec!["rebase", "--abort"],
            RepositoryState::CherryPickInProgress => vec!["cherry-pick", "--abort"],
            RepositoryState::RevertInProgress => vec!["revert", "--abort"],
            _ => {
                return Err(GitError::new(
                    GitErrorKind::Unsupported,
                    "No Git operation is active.",
                ));
            }
        };
        self.runner
            .run(GitCommand::new().cwd(repository).args(args))?;
        Ok(())
    }

    pub fn merge(&self, repository: &Path, revision: &str) -> GitResult<()> {
        validate_ref(revision)?;
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(["merge", "--", revision]),
        )?;
        Ok(())
    }

    pub fn rebase(&self, repository: &Path, revision: &str) -> GitResult<()> {
        validate_ref(revision)?;
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(["rebase", "--", revision]),
        )?;
        Ok(())
    }

    pub fn cherry_pick(&self, repository: &Path, revision: &str) -> GitResult<()> {
        validate_ref(revision)?;
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(["cherry-pick", "--", revision]),
        )?;
        Ok(())
    }

    pub fn revert(&self, repository: &Path, revision: &str) -> GitResult<()> {
        validate_ref(revision)?;
        self.runner.run(GitCommand::new().cwd(repository).args([
            "revert",
            "--no-edit",
            "--",
            revision,
        ]))?;
        Ok(())
    }

    pub fn conflict_stages(&self, repository: &Path) -> GitResult<Vec<ConflictStages>> {
        let output = self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(["ls-files", "--unmerged", "-z"])
                .read_only(),
        )?;
        parse_conflict_stages(&output.stdout)
    }

    pub fn read_blob(&self, repository: &Path, oid: &str) -> GitResult<Vec<u8>> {
        if oid.len() < 4 || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(GitError::new(GitErrorKind::Other, "Invalid Git object id."));
        }
        Ok(self
            .runner
            .run(
                GitCommand::new()
                    .cwd(repository)
                    .args(["cat-file", "blob", oid])
                    .read_only(),
            )?
            .stdout)
    }

    pub fn blame(&self, repository: &Path, path: &Path) -> GitResult<Vec<BlameLine>> {
        let output = self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args([
                    OsString::from("blame"),
                    OsString::from("--line-porcelain"),
                    OsString::from("--"),
                    path.as_os_str().to_owned(),
                ])
                .read_only(),
        )?;
        parse_blame(&String::from_utf8_lossy(&output.stdout))
    }

    pub fn submodule_update(&self, repository: &Path, recursive: bool) -> GitResult<()> {
        let mut args = vec!["submodule", "update", "--init"];
        if recursive {
            args.push("--recursive");
        }
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(args)
                .allow_prompt()
                .timeout(Duration::from_secs(60 * 30)),
        )?;
        Ok(())
    }

    pub fn lfs_pull(&self, repository: &Path) -> GitResult<()> {
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(["lfs", "pull"])
                .allow_prompt()
                .timeout(Duration::from_secs(60 * 30)),
        )?;
        Ok(())
    }

    pub fn sparse_checkout_set(&self, repository: &Path, patterns: &[String]) -> GitResult<()> {
        if patterns.is_empty() {
            return Err(GitError::new(
                GitErrorKind::Other,
                "At least one sparse path is required.",
            ));
        }
        let mut args = vec![
            OsString::from("sparse-checkout"),
            OsString::from("set"),
            OsString::from("--"),
        ];
        args.extend(patterns.iter().map(OsString::from));
        self.runner
            .run(GitCommand::new().cwd(repository).args(args))?;
        Ok(())
    }

    pub fn sparse_checkout_disable(&self, repository: &Path) -> GitResult<()> {
        self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(["sparse-checkout", "disable"]),
        )?;
        Ok(())
    }

    fn pathspec_command(
        &self,
        repository: &Path,
        prefix: &[&str],
        paths: &[PathBuf],
    ) -> GitResult<()> {
        if paths.is_empty() {
            return Ok(());
        }
        if self.runtime().capabilities.pathspec_from_file {
            let mut args = prefix.iter().map(OsString::from).collect::<Vec<_>>();
            args.extend([
                OsString::from("--pathspec-from-file=-"),
                OsString::from("--pathspec-file-nul"),
            ]);
            self.runner.run(
                GitCommand::new()
                    .cwd(repository)
                    .args(args)
                    .stdin(nul_pathspec(paths)),
            )?;
        } else {
            let mut args = prefix.iter().map(OsString::from).collect::<Vec<_>>();
            args.push("--".into());
            args.extend(paths.iter().map(|path| path.as_os_str().to_owned()));
            self.runner
                .run(GitCommand::new().cwd(repository).args(args))?;
        }
        Ok(())
    }
}

impl GitBackend for CliGitBackend {
    fn runtime(&self) -> &GitRuntime {
        self.runner.runtime()
    }

    fn discover(&self, roots: &[PathBuf]) -> GitResult<Vec<DiscoveredRepository>> {
        discovery::discover(&self.runner, roots)
    }

    fn status(
        &self,
        repository: &DiscoveredRepository,
        id: RepositoryId,
        generation: u64,
    ) -> GitResult<RepositorySnapshot> {
        // `normal` reports an untracked directory as one entry instead of
        // walking every file below it. The tree can expand that directory from
        // the filesystem when the user needs it, while polling stays cheap.
        self.status_with_untracked(repository, id, generation, "normal")
    }

    fn status_fast(
        &self,
        repository: &DiscoveredRepository,
        id: RepositoryId,
        generation: u64,
    ) -> GitResult<RepositorySnapshot> {
        // The first paint should not wait for an untracked-file traversal.
        // A detailed refresh follows immediately on the same worker.
        self.status_with_untracked(repository, id, generation, "no")
    }

    fn clone_repository(
        &self,
        url: &str,
        target: &Path,
        cancellation: &CancellationToken,
    ) -> GitResult<()> {
        if url.trim().is_empty() {
            return Err(GitError::new(
                GitErrorKind::Other,
                "Repository URL is empty.",
            ));
        }
        self.runner.run_cancellable(
            GitCommand::new()
                .args([
                    OsString::from("clone"),
                    OsString::from("--progress"),
                    OsString::from("--"),
                    OsString::from(url),
                    target.as_os_str().to_owned(),
                ])
                .allow_prompt()
                .timeout(Duration::from_secs(60 * 60)),
            cancellation,
        )?;
        Ok(())
    }
}

impl CliGitBackend {
    fn status_with_untracked(
        &self,
        repository: &DiscoveredRepository,
        id: RepositoryId,
        generation: u64,
        untracked: &str,
    ) -> GitResult<RepositorySnapshot> {
        let mut args = vec![
            OsString::from("status"),
            OsString::from("--porcelain=v2"),
            OsString::from("--branch"),
            OsString::from("-z"),
            OsString::from(format!("--untracked-files={untracked}")),
        ];
        // `matching` lists an ignored directory once without walking it, which
        // keeps `target/` or `node_modules/` cheap. Git rejects it with `no`.
        if untracked != "no" {
            args.push(OsString::from("--ignored=matching"));
        }
        let output = self.runner.run(
            GitCommand::new()
                .cwd(&repository.worktree_root)
                .args(args)
                .read_only(),
        )?;
        let mut parsed = parse_porcelain_v2(&output.stdout)?;
        // Keep the fast first paint process-free beyond status itself. The
        // detailed refresh enriches visible rows with real line statistics.
        if untracked != "no" {
            if let Ok(index_stats) = self.diff_stats(&repository.worktree_root, true) {
                for (path, stat) in index_stats {
                    if let Some(file) = parsed.files.get_mut(&path) {
                        file.index_stat = Some(stat);
                    }
                }
            }
            if let Ok(worktree_stats) = self.diff_stats(&repository.worktree_root, false) {
                for (path, stat) in worktree_stats {
                    if let Some(file) = parsed.files.get_mut(&path) {
                        file.worktree_stat = Some(stat);
                    }
                }
            }
        }
        let repository_state = repository_state(repository, &parsed.head);
        let features = repository_features(repository);
        Ok(RepositorySnapshot {
            id,
            worktree_root: repository.worktree_root.clone(),
            git_dir: repository.git_dir.clone(),
            common_dir: repository.common_dir.clone(),
            head: parsed.head,
            upstream: parsed.upstream,
            ahead: parsed.ahead,
            behind: parsed.behind,
            files: parsed.files,
            ignored: parsed.ignored,
            repository_state,
            features,
            generation,
        })
    }

    fn diff_stats(
        &self,
        repository: &Path,
        staged: bool,
    ) -> GitResult<std::collections::BTreeMap<PathBuf, DiffStat>> {
        let mut args = vec![
            OsString::from("diff"),
            OsString::from("--numstat"),
            OsString::from("-z"),
            OsString::from("--no-ext-diff"),
        ];
        if staged {
            args.push(OsString::from("--cached"));
        }
        args.push(OsString::from("--"));
        let output = self.runner.run(
            GitCommand::new()
                .cwd(repository)
                .args(args)
                .read_only(),
        )?;
        parse_numstat(&output.stdout)
    }
}

fn repository_state(repository: &DiscoveredRepository, head: &HeadState) -> RepositoryState {
    let git = &repository.git_dir;
    let common = &repository.common_dir;
    if git.join("MERGE_HEAD").exists() {
        RepositoryState::MergeInProgress
    } else if git.join("rebase-merge").exists() || git.join("rebase-apply").exists() {
        RepositoryState::RebaseInProgress
    } else if git.join("CHERRY_PICK_HEAD").exists() {
        RepositoryState::CherryPickInProgress
    } else if git.join("REVERT_HEAD").exists() {
        RepositoryState::RevertInProgress
    } else if common.join("BISECT_LOG").exists() {
        RepositoryState::Bisecting
    } else {
        match head {
            HeadState::Detached(_) => RepositoryState::DetachedHead,
            HeadState::Unborn(_) => RepositoryState::UnbornBranch,
            HeadState::Branch(_) => RepositoryState::Normal,
        }
    }
}

fn validate_ref(value: &str) -> GitResult<()> {
    if value.is_empty()
        || value.starts_with('-')
        || value.contains(['\0', '\n', '\r'])
        || value == ".."
    {
        Err(GitError::new(GitErrorKind::Other, "Invalid Git reference."))
    } else {
        Ok(())
    }
}

fn parse_worktrees(input: &[u8]) -> GitResult<Vec<WorktreeInfo>> {
    let mut values = Vec::new();
    let mut current = None::<WorktreeInfo>;
    for record in input
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let text = String::from_utf8_lossy(record);
        if let Some(path) = text.strip_prefix("worktree ") {
            if let Some(previous) = current.take() {
                values.push(previous);
            }
            current = Some(WorktreeInfo {
                path: PathBuf::from(path),
                oid: String::new(),
                branch: None,
                bare: false,
                detached: false,
                locked: None,
            });
        } else if let Some(current) = current.as_mut() {
            if let Some(value) = text.strip_prefix("HEAD ") {
                current.oid = value.to_owned();
            } else if let Some(value) = text.strip_prefix("branch ") {
                current.branch = Some(value.trim_start_matches("refs/heads/").to_owned());
            } else if text == "bare" {
                current.bare = true;
            } else if text == "detached" {
                current.detached = true;
            } else if let Some(value) = text.strip_prefix("locked") {
                current.locked = Some(value.trim().to_owned());
            }
        }
    }
    if let Some(current) = current {
        values.push(current);
    }
    Ok(values)
}

fn repository_features(repository: &DiscoveredRepository) -> RepositoryFeatures {
    let attributes = std::fs::read_to_string(repository.worktree_root.join(".gitattributes"))
        .unwrap_or_default();
    RepositoryFeatures {
        submodules: repository.worktree_root.join(".gitmodules").is_file(),
        lfs: attributes.lines().any(|line| line.contains("filter=lfs")),
        sparse_checkout: repository
            .common_dir
            .join("info")
            .join("sparse-checkout")
            .is_file(),
        shallow: repository.common_dir.join("shallow").is_file(),
    }
}

fn parse_conflict_stages(input: &[u8]) -> GitResult<Vec<ConflictStages>> {
    let mut values = BTreeMap::<PathBuf, ConflictStages>::new();
    for record in input
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
            return Err(GitError::new(
                GitErrorKind::InvalidOutput,
                "Invalid conflict stage output.",
            ));
        };
        let header = String::from_utf8_lossy(&record[..tab]);
        let mut fields = header.split_whitespace();
        let _mode = fields.next();
        let oid = fields.next().ok_or_else(|| {
            GitError::new(GitErrorKind::InvalidOutput, "Missing conflict object id.")
        })?;
        let stage: u8 = fields
            .next()
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| GitError::new(GitErrorKind::InvalidOutput, "Missing conflict stage."))?;
        let path = path_from_git_bytes(&record[tab + 1..]);
        let value = values
            .entry(path.clone())
            .or_insert_with(|| ConflictStages {
                path,
                ..ConflictStages::default()
            });
        match stage {
            1 => value.base = Some(oid.to_owned()),
            2 => value.ours = Some(oid.to_owned()),
            3 => value.theirs = Some(oid.to_owned()),
            _ => {}
        }
    }
    Ok(values.into_values().collect())
}

fn parse_blame(input: &str) -> GitResult<Vec<BlameLine>> {
    let mut result = Vec::new();
    let mut lines = input.lines().peekable();
    while let Some(header) = lines.next() {
        let mut fields = header.split_whitespace();
        let Some(oid) = fields.next() else { continue };
        if oid.len() < 4 || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        let original_line = fields
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let final_line = fields
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let count = fields
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1);
        let mut author = String::new();
        let mut author_time = 0;
        let mut summary = String::new();
        for line in lines.by_ref() {
            if line.starts_with('\t') {
                break;
            }
            if let Some(value) = line.strip_prefix("author ") {
                author = value.to_owned();
            } else if let Some(value) = line.strip_prefix("author-time ") {
                author_time = value.parse().unwrap_or_default();
            } else if let Some(value) = line.strip_prefix("summary ") {
                summary = value.to_owned();
            }
        }
        for offset in 0..count {
            result.push(BlameLine {
                oid: oid.to_owned(),
                original_line: original_line + offset,
                final_line: final_line + offset,
                author: author.clone(),
                author_time,
                summary: summary.clone(),
            });
        }
    }
    Ok(result)
}

#[cfg(unix)]
fn path_from_git_bytes(value: &[u8]) -> PathBuf {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(OsString::from_vec(value.to_vec()))
}

#[cfg(not(unix))]
fn path_from_git_bytes(value: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(value).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::runtime::GitRuntimeManager;

    fn repository() -> Option<(PathBuf, CliGitBackend)> {
        let runtime = GitRuntimeManager::default().resolve().ok()?;
        let root = std::env::temp_dir().join(format!(
            "loom-git-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).ok()?;
        let backend = CliGitBackend::new(runtime);
        backend
            .runner()
            .run(GitCommand::new().cwd(&root).args(["init", "-b", "main"]))
            .ok()?;
        backend
            .runner()
            .run(
                GitCommand::new()
                    .cwd(&root)
                    .args(["config", "user.name", "Loom Test"]),
            )
            .ok()?;
        backend
            .runner()
            .run(GitCommand::new().cwd(&root).args([
                "config",
                "user.email",
                "loom@example.invalid",
            ]))
            .ok()?;
        Some((root, backend))
    }

    #[test]
    fn real_repository_status_stage_commit_history_diff_and_blame() {
        let Some((root, backend)) = repository() else {
            return;
        };
        std::fs::write(root.join("tracked.txt"), "one\n").unwrap();
        backend
            .stage(&root, &[PathBuf::from("tracked.txt")])
            .unwrap();
        backend.commit(&root, "initial", false, false).unwrap();
        std::fs::write(root.join("tracked.txt"), "one\ntwo\n").unwrap();
        std::fs::write(root.join("untracked.txt"), "new\n").unwrap();
        std::fs::write(root.join(".gitignore"), "build/\n").unwrap();
        std::fs::create_dir_all(root.join("build/out")).unwrap();
        std::fs::write(root.join("build/out/app"), "bin\n").unwrap();

        let discovered = backend.discover(std::slice::from_ref(&root)).unwrap();
        let status = backend.status(&discovered[0], RepositoryId(1), 7).unwrap();
        assert_eq!(status.head, HeadState::Branch("main".into()));
        assert_eq!(
            status.files[&PathBuf::from("tracked.txt")].worktree,
            ChangeKind::Modified
        );
        assert_eq!(
            status.files[&PathBuf::from("untracked.txt")].worktree,
            ChangeKind::Untracked
        );
        assert!(status.ignored.contains(&PathBuf::from("build")));
        assert!(!status.files.contains_key(&PathBuf::from("build")));
        assert!(
            backend
                .diff(&root, DiffTarget::IndexToWorktree, &[])
                .unwrap()
                .contains("+two")
        );
        assert_eq!(
            backend.history(&root, None, None, 0, 20).unwrap()[0].subject,
            "initial"
        );
        assert!(
            backend
                .branches(&root)
                .unwrap()
                .iter()
                .any(|branch| branch.current && branch.name == "main")
        );
        assert_eq!(
            backend.blame(&root, Path::new("tracked.txt")).unwrap()[0].author,
            "Loom Test"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn groups_conflict_index_stages_by_path() {
        let parsed = parse_conflict_stages(
            b"100644 aaaa 1\tfile.txt\x00100644 bbbb 2\tfile.txt\x00100644 cccc 3\tfile.txt\0",
        )
        .unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].base.as_deref(), Some("aaaa"));
        assert_eq!(parsed[0].ours.as_deref(), Some("bbbb"));
        assert_eq!(parsed[0].theirs.as_deref(), Some("cccc"));
    }
}
