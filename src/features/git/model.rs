//! Source Control business state kept between scans: the history list and
//! the expanded commit, cached commit details, the diff open in flight, the
//! repository chosen by the user, and a clone in progress.
//!
//! Fields are private; each transition keeps request sequences and caches
//! consistent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{ChangeKind, CommitDetails, CommitSummary, DiffRequest};

/// Recent commits of a repository's HEAD.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHistory {
    pub repository: PathBuf,
    /// The commit HEAD was at when these were listed.
    pub head: Option<String>,
    pub commits: Result<Vec<CommitSummary>, String>,
}

/// A history commit expanded to list the files it changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpandedCommit {
    pub oid: String,
    /// `None` while loading.
    pub files: Option<Result<Vec<(ChangeKind, PathBuf)>, String>>,
}

/// Commit details are immutable; the cache only bounds memory.
const COMMIT_DETAIL_CACHE: usize = 64;

#[derive(Clone, Debug, Default)]
pub struct GitModel {
    history: Option<GitHistory>,
    history_commit: Option<ExpandedCommit>,
    commit_details: HashMap<String, Result<CommitDetails, String>>,
    open_diff_sequence: u64,
    diff_loading: Option<DiffRequest>,
    /// A repository chosen in Source Control, and the active file when it
    /// was chosen: the choice holds until that file changes.
    pinned_repository: Option<(PathBuf, Option<PathBuf>)>,
    cloning: bool,
}

impl GitModel {
    // --- history ----------------------------------------------------------

    pub fn history(&self) -> Option<&GitHistory> {
        self.history.as_ref()
    }

    pub fn set_history(&mut self, history: GitHistory) {
        self.history = Some(history);
    }

    pub fn history_commit(&self) -> Option<&ExpandedCommit> {
        self.history_commit.as_ref()
    }

    /// Expand `oid` with its files still loading, or collapse it when it is
    /// already expanded. Returns whether it is expanded now.
    pub fn toggle_commit(&mut self, oid: String) -> bool {
        if self
            .history_commit
            .as_ref()
            .is_some_and(|commit| commit.oid == oid)
        {
            self.history_commit = None;
            false
        } else {
            self.history_commit = Some(ExpandedCommit { oid, files: None });
            true
        }
    }

    /// Fill in the files of `oid` when it is still the expanded commit.
    pub fn set_commit_files(
        &mut self,
        oid: &str,
        files: Result<Vec<(ChangeKind, PathBuf)>, String>,
    ) {
        if let Some(commit) = self
            .history_commit
            .as_mut()
            .filter(|commit| commit.oid == oid)
        {
            commit.files = Some(files);
        }
    }

    pub fn collapse_commit(&mut self) {
        self.history_commit = None;
    }

    pub fn commit_detail(&self, oid: &str) -> Option<&Result<CommitDetails, String>> {
        self.commit_details.get(oid)
    }

    pub fn cache_commit_detail(&mut self, oid: String, details: Result<CommitDetails, String>) {
        if self.commit_details.len() >= COMMIT_DETAIL_CACHE {
            self.commit_details.clear();
        }
        self.commit_details.insert(oid, details);
    }

    // --- diff opening -----------------------------------------------------

    /// Start opening a diff tab; any earlier open still in flight becomes
    /// stale. Returns the new open's sequence.
    pub fn begin_diff_open(&mut self, loading: DiffRequest) -> u64 {
        self.open_diff_sequence += 1;
        self.diff_loading = Some(loading);
        self.open_diff_sequence
    }

    /// Invalidate any open in flight, e.g. because its tab already exists.
    pub fn cancel_diff_open(&mut self) {
        self.open_diff_sequence += 1;
        self.diff_loading = None;
    }

    /// Finish open `sequence`. Returns false, changing nothing, when a newer
    /// open superseded it.
    pub fn finish_diff_open(&mut self, sequence: u64) -> bool {
        if self.open_diff_sequence != sequence {
            return false;
        }
        self.diff_loading = None;
        true
    }

    /// The worker could not start; nothing is loading.
    pub fn abandon_diff_open(&mut self) {
        self.diff_loading = None;
    }

    pub fn diff_loading(&self) -> Option<&DiffRequest> {
        self.diff_loading.as_ref()
    }

    // --- repository choice ------------------------------------------------

    /// Make `root` the Source Control repository while `active_path` stays
    /// the active file, collapsing any expanded commit.
    pub fn pin_repository(&mut self, root: PathBuf, active_path: Option<PathBuf>) {
        self.pinned_repository = Some((root, active_path));
        self.history_commit = None;
    }

    /// The pinned repository, while the active file is still the one it
    /// was chosen with.
    pub fn pinned_for(&self, active_path: Option<&Path>) -> Option<&PathBuf> {
        self.pinned_repository
            .as_ref()
            .filter(|(_, pinned_path)| pinned_path.as_deref() == active_path)
            .map(|(root, _)| root)
    }

    /// Forget the choice when it named `root`.
    pub fn unpin(&mut self, root: &Path) {
        if self
            .pinned_repository
            .as_ref()
            .is_some_and(|(pinned, _)| pinned == root)
        {
            self.pinned_repository = None;
        }
    }

    pub fn has_pinned_repository(&self) -> bool {
        self.pinned_repository.is_some()
    }

    // --- clone --------------------------------------------------------------

    pub fn cloning(&self) -> bool {
        self.cloning
    }

    /// Returns false when a clone is already running.
    pub fn begin_clone(&mut self) -> bool {
        !std::mem::replace(&mut self.cloning, true)
    }

    pub fn finish_clone(&mut self) {
        self.cloning = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::git::DiffTarget;

    fn request(path: &str) -> DiffRequest {
        DiffRequest {
            repository_root: PathBuf::from("repo"),
            path: PathBuf::from(path),
            target: DiffTarget::IndexToWorktree,
        }
    }

    #[test]
    fn a_newer_diff_open_makes_the_earlier_one_stale() {
        let mut model = GitModel::default();
        let first = model.begin_diff_open(request("a.rs"));
        let second = model.begin_diff_open(request("b.rs"));

        assert!(!model.finish_diff_open(first));
        assert_eq!(model.diff_loading(), Some(&request("b.rs")));
        assert!(model.finish_diff_open(second));
        assert!(model.diff_loading().is_none());
    }

    #[test]
    fn cancelling_an_open_rejects_its_late_result() {
        let mut model = GitModel::default();
        let open = model.begin_diff_open(request("a.rs"));
        model.cancel_diff_open();

        assert!(model.diff_loading().is_none());
        assert!(!model.finish_diff_open(open));
    }

    #[test]
    fn toggling_a_commit_expands_then_collapses_and_ignores_late_files() {
        let mut model = GitModel::default();
        assert!(model.toggle_commit("a".into()));
        model.set_commit_files("b", Ok(Vec::new()));
        assert_eq!(model.history_commit().unwrap().files, None);
        model.set_commit_files("a", Ok(vec![(ChangeKind::Added, "x.rs".into())]));
        assert!(model.history_commit().unwrap().files.is_some());

        assert!(!model.toggle_commit("a".into()));
        assert!(model.history_commit().is_none());
        model.set_commit_files("a", Ok(Vec::new()));
        assert!(model.history_commit().is_none());
    }

    #[test]
    fn a_pinned_repository_holds_until_the_active_file_changes() {
        let mut model = GitModel::default();
        model.toggle_commit("a".into());
        model.pin_repository("repo".into(), Some("repo/a.rs".into()));

        assert!(model.history_commit().is_none());
        assert_eq!(
            model.pinned_for(Some(Path::new("repo/a.rs"))),
            Some(&PathBuf::from("repo"))
        );
        assert_eq!(model.pinned_for(Some(Path::new("repo/b.rs"))), None);
        model.unpin(Path::new("other"));
        assert!(model.has_pinned_repository());
        model.unpin(Path::new("repo"));
        assert!(!model.has_pinned_repository());
    }

    #[test]
    fn the_commit_detail_cache_is_bounded() {
        let mut model = GitModel::default();
        for index in 0..COMMIT_DETAIL_CACHE {
            model.cache_commit_detail(format!("{index}"), Err("x".into()));
        }
        assert!(model.commit_detail("0").is_some());
        model.cache_commit_detail("new".into(), Err("x".into()));
        assert!(model.commit_detail("0").is_none());
        assert!(model.commit_detail("new").is_some());
    }

    #[test]
    fn only_one_clone_runs_at_a_time() {
        let mut model = GitModel::default();
        assert!(model.begin_clone());
        assert!(!model.begin_clone());
        model.finish_clone();
        assert!(!model.cloning());
    }
}
