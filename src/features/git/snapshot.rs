//! The repository snapshot the UI renders, its prepared presentation, and
//! the rules for accepting or merging scan results.
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use super::conflict::ConflictRegion;
use super::diff_document::DiffDocument;
use super::error::{GitError, GitResult};
use super::types::*;

#[derive(Clone, Debug, Default)]
pub struct GitPresentation {
    pub repositories: BTreeMap<RepositoryId, RepositoryPresentation>,
    pub diffs: BTreeMap<DiffRequest, Arc<DiffDocument>>,
}

#[derive(Clone, Debug, Default)]
pub struct RepositoryPresentation {
    pub sections: [Vec<PathBuf>; 4],
    pub dirty_count: usize,
    pub decorations: BTreeMap<PathBuf, PathDecoration>,
}

#[derive(Clone, Debug)]
pub struct GitStoreSnapshot {
    pub repositories: Arc<BTreeMap<RepositoryId, RepositorySnapshot>>,
    pub active_repository: Option<RepositoryId>,
    pub operation: Option<OperationState>,
    pub last_error: Option<GitError>,
    pub runtime_label: Option<String>,
    pub generation: u64,
    /// Git line decorations for currently active files, keyed by absolute path
    /// and one-based line number.
    pub line_changes: Arc<BTreeMap<PathBuf, BTreeMap<usize, LineChange>>>,
    /// Conflicts Git reports in currently active unmerged files, keyed by
    /// absolute path.
    pub conflicts: Arc<BTreeMap<PathBuf, Arc<[ConflictRegion]>>>,
    pub diffs: Arc<BTreeMap<DiffRequest, GitResult<DiffContent>>>,
    pub presentation: Arc<GitPresentation>,
    pub initializing: bool,
}

impl Default for GitStoreSnapshot {
    fn default() -> Self {
        Self {
            repositories: Arc::default(),
            active_repository: None,
            operation: None,
            last_error: None,
            runtime_label: None,
            generation: 0,
            line_changes: Arc::default(),
            conflicts: Arc::default(),
            diffs: Arc::default(),
            presentation: Arc::default(),
            initializing: true,
        }
    }
}

impl GitStoreSnapshot {
    pub fn active(&self) -> Option<&RepositorySnapshot> {
        self.active_repository
            .and_then(|id| self.repositories.get(&id))
            .or_else(|| self.repositories.values().next())
    }

    pub fn repository_for_path(&self, path: &Path) -> Option<&RepositorySnapshot> {
        self.repositories
            .values()
            .filter(|repository| repository.relative_path(path).is_some())
            .max_by_key(|repository| repository.worktree_root.components().count())
    }

    pub fn choose_active_for_path(&mut self, path: Option<&Path>) {
        if let Some(path) = path
            && let Some(repository) = self.repository_for_path(path)
        {
            self.active_repository = Some(repository.id);
        } else if self.active_repository.is_none() {
            self.active_repository = self.repositories.keys().next().copied();
        }
    }

    /// Make the repository at `root` active, when it is one of ours.
    pub fn pin_active(&mut self, root: Option<&Path>) {
        if let Some(root) = root
            && let Some(repository) = self
                .repositories
                .values()
                .find(|repository| repository.worktree_root == root)
        {
            self.active_repository = Some(repository.id);
        }
    }

    /// Prevent a slower scan from replacing a newer repository generation.
    /// Scans never know about a running operation, so the current one
    /// survives the replacement.
    pub fn replace_if_newer(&mut self, next: Self) -> bool {
        if next.generation < self.generation {
            return false;
        }
        let operation = self.operation.take();
        *self = Self { operation, ..next };
        true
    }

    /// Apply a scan result. A failed scan keeps the last known repositories
    /// visible instead of emptying the drawer.
    pub fn apply_scan(&mut self, result: GitResult<Self>) -> bool {
        match result {
            Ok(next) => {
                if next.generation < self.generation {
                    return false;
                }
                let changed = !same_shared_scan(self, &next);
                // Advance the generation even when no repaint is needed.
                self.replace_if_newer(next);
                changed
            }
            Err(error) => {
                let changed = self.last_error.as_ref() != Some(&error) || self.initializing;
                self.last_error = Some(error);
                self.initializing = false;
                changed
            }
        }
    }

    /// Build render data and compare large collections only on a worker.
    /// Stable collections keep their Arc identity, making UI comparison O(1).
    pub fn prepare_for_ui(&mut self, previous: Option<&Self>) {
        if let Some(previous) = previous {
            if same_repositories(&self.repositories, &previous.repositories) {
                self.repositories = previous.repositories.clone();
            }
            if self.line_changes == previous.line_changes {
                self.line_changes = previous.line_changes.clone();
            }
            if self.conflicts == previous.conflicts {
                self.conflicts = previous.conflicts.clone();
            }
            if self.diffs == previous.diffs {
                self.diffs = previous.diffs.clone();
            }
            if Arc::ptr_eq(&self.repositories, &previous.repositories)
                && Arc::ptr_eq(&self.diffs, &previous.diffs)
            {
                self.presentation = previous.presentation.clone();
                return;
            }
        }
        let mut presentation = GitPresentation::default();
        for (id, repository) in self.repositories.iter() {
            let mut prepared = RepositoryPresentation::default();
            for (path, file) in repository.files.iter() {
                prepared.dirty_count += usize::from(file.is_changed());
                if file.conflict.is_some() {
                    prepared.sections[0].push(path.clone());
                } else {
                    if file.index != ChangeKind::Unmodified {
                        prepared.sections[1].push(path.clone());
                    }
                    if file.worktree != ChangeKind::Unmodified
                        && file.worktree != ChangeKind::Untracked
                    {
                        prepared.sections[2].push(path.clone());
                    }
                }
                if file.worktree == ChangeKind::Untracked {
                    prepared.sections[3].push(path.clone());
                }
                if let Some(decoration) = PathDecoration::for_state(file) {
                    for ancestor in path.ancestors().filter(|path| !path.as_os_str().is_empty()) {
                        prepared
                            .decorations
                            .entry(ancestor.to_path_buf())
                            .and_modify(|current| *current = (*current).min(decoration))
                            .or_insert(decoration);
                    }
                }
            }
            presentation.repositories.insert(*id, prepared);
        }
        for (request, result) in self.diffs.iter() {
            // Reuse a prepared document when only some other diff changed.
            if let Some(previous) = previous
                && previous.diffs.get(request) == Some(result)
            {
                if let Some(document) = previous.presentation.diffs.get(request) {
                    presentation.diffs.insert(request.clone(), document.clone());
                }
                continue;
            }
            let Ok(content) = result else { continue };
            let document = DiffDocument::from_content(request.clone(), content.clone());
            presentation
                .diffs
                .insert(request.clone(), Arc::new(document));
        }
        self.presentation = Arc::new(presentation);
    }

    pub fn decoration_for_absolute_path(&self, path: &Path) -> Option<PathDecoration> {
        let repository = self.repository_for_path(path)?;
        let relative = repository.relative_path(path)?;
        if relative.as_os_str().is_empty() {
            return None;
        }
        if let Some(decoration) = self
            .presentation
            .repositories
            .get(&repository.id)
            .and_then(|prepared| prepared.decorations.get(&relative))
        {
            return Some(*decoration);
        }
        relative.ancestors().find_map(|ancestor| {
            if repository
                .files
                .get(ancestor)
                .is_some_and(|state| state.worktree == ChangeKind::Untracked)
            {
                Some(PathDecoration::Created)
            } else if repository.ignored.contains(ancestor) {
                Some(PathDecoration::Ignored)
            } else {
                None
            }
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PollTarget {
    pub active_path: Option<PathBuf>,
    pub diffs: Vec<DiffRequest>,
    /// A repository chosen in Source Control, active instead of the one
    /// holding `active_path`.
    pub pinned_repository: Option<PathBuf>,
}

pub fn binary_content() -> DiffContent {
    DiffContent::Unified(super::diff::UnifiedDiff {
        files: vec![super::diff::FileDiff {
            binary: true,
            ..Default::default()
        }],
    })
}

pub fn collect_line_changes(
    diff: &super::diff::UnifiedDiff,
    output: &mut BTreeMap<usize, LineChange>,
) {
    use super::diff::DiffLine;
    for file in &diff.files {
        for hunk in &file.hunks {
            let mut line = hunk.new_start.max(1);
            for item in &hunk.lines {
                match item {
                    DiffLine::Context(_) => line += 1,
                    DiffLine::Addition(_) => {
                        let value = if output.get(&line) == Some(&LineChange::Deleted) {
                            LineChange::Modified
                        } else {
                            LineChange::Added
                        };
                        output.insert(line, value);
                        line += 1;
                    }
                    DiffLine::Deletion(_) => {
                        output.entry(line).or_insert(LineChange::Deleted);
                    }
                    DiffLine::NoNewline => {}
                }
            }
        }
    }
}

pub fn operation(
    kind: OperationKind,
    message: impl Into<String>,
    cancellable: bool,
) -> OperationState {
    OperationState {
        kind,
        started_at: SystemTime::now(),
        progress: None,
        cancellable,
        message: message.into(),
    }
}

pub fn repository_id(path: &Path) -> RepositoryId {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    RepositoryId(hasher.finish())
}

/// Whether two scans saw the same state; generations always differ.
/// Destructured so a new field cannot be left out of the comparison.
pub fn same_scan(last: &GitStoreSnapshot, next: &GitStoreSnapshot) -> bool {
    let GitStoreSnapshot {
        repositories,
        active_repository,
        operation: _,
        last_error,
        runtime_label,
        generation: _,
        line_changes,
        conflicts,
        diffs,
        presentation: _,
        initializing,
    } = last;
    same_repositories(repositories, &next.repositories)
        && *active_repository == next.active_repository
        && *last_error == next.last_error
        && *runtime_label == next.runtime_label
        && *line_changes == next.line_changes
        && *conflicts == next.conflicts
        && *diffs == next.diffs
        && *initializing == next.initializing
}

pub fn same_repositories(
    last: &BTreeMap<RepositoryId, RepositorySnapshot>,
    next: &BTreeMap<RepositoryId, RepositorySnapshot>,
) -> bool {
    last.len() == next.len()
        && last
            .iter()
            .zip(next)
            .all(|((id, repository), (next_id, next_repository))| {
                id == next_id && repository.same_state(next_repository)
            })
}

pub fn same_shared_scan(last: &GitStoreSnapshot, next: &GitStoreSnapshot) -> bool {
    (Arc::ptr_eq(&last.repositories, &next.repositories)
        || (last.repositories.is_empty() && next.repositories.is_empty()))
        && (Arc::ptr_eq(&last.line_changes, &next.line_changes)
            || (last.line_changes.is_empty() && next.line_changes.is_empty()))
        && (Arc::ptr_eq(&last.diffs, &next.diffs)
            || (last.diffs.is_empty() && next.diffs.is_empty()))
        && last.active_repository == next.active_repository
        && last.last_error == next.last_error
        && last.runtime_label == next.runtime_label
        && last.initializing == next.initializing
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::git::error::GitErrorKind;

    #[test]
    fn picks_the_nearest_repository_for_a_file() {
        let parent = RepositorySnapshot {
            id: RepositoryId(1),
            worktree_root: PathBuf::from("/workspace"),
            git_dir: PathBuf::new(),
            common_dir: PathBuf::new(),
            head: HeadState::default(),
            upstream: None,
            ahead: 0,
            behind: 0,
            files: Arc::default(),
            ignored: Default::default(),
            repository_state: RepositoryState::Normal,
            operation_step: None,
            head_oid: None,
            stashes: Default::default(),
            tags: Default::default(),
            worktrees: Default::default(),
            features: RepositoryFeatures::default(),
            generation: 1,
        };
        let child = RepositorySnapshot {
            id: RepositoryId(2),
            worktree_root: PathBuf::from("/workspace/nested"),
            ..parent.clone()
        };
        let mut store = GitStoreSnapshot::default();
        Arc::make_mut(&mut store.repositories).insert(parent.id, parent);
        Arc::make_mut(&mut store.repositories).insert(child.id, child);
        assert_eq!(
            store
                .repository_for_path(Path::new("/workspace/nested/file"))
                .map(|repository| repository.id),
            Some(RepositoryId(2))
        );
    }

    #[test]
    fn older_generations_never_replace_newer_state() {
        let mut current = GitStoreSnapshot {
            generation: 9,
            ..GitStoreSnapshot::default()
        };
        assert!(!current.replace_if_newer(GitStoreSnapshot {
            generation: 8,
            ..GitStoreSnapshot::default()
        }));
        assert_eq!(current.generation, 9);
        assert!(current.replace_if_newer(GitStoreSnapshot {
            generation: 10,
            ..GitStoreSnapshot::default()
        }));
        assert_eq!(current.generation, 10);
    }

    #[test]
    fn scans_keep_the_running_operation() {
        let mut current = GitStoreSnapshot {
            operation: Some(operation(OperationKind::Push, "Pushing…", false)),
            generation: 1,
            ..GitStoreSnapshot::default()
        };
        current.apply_scan(Ok(GitStoreSnapshot {
            generation: 2,
            ..GitStoreSnapshot::default()
        }));
        assert_eq!(current.generation, 2);
        assert!(current.operation.is_some());
    }

    #[test]
    fn a_failed_scan_keeps_known_repositories() {
        let mut current = GitStoreSnapshot {
            generation: 3,
            ..GitStoreSnapshot::default()
        };
        Arc::make_mut(&mut current.repositories).insert(
            RepositoryId(1),
            RepositorySnapshot {
                id: RepositoryId(1),
                worktree_root: PathBuf::from("/workspace"),
                git_dir: PathBuf::new(),
                common_dir: PathBuf::new(),
                head: HeadState::default(),
                upstream: None,
                ahead: 0,
                behind: 0,
                files: Arc::default(),
                ignored: Default::default(),
                repository_state: RepositoryState::Normal,
                operation_step: None,
                head_oid: None,
                stashes: Default::default(),
                tags: Default::default(),
                worktrees: Default::default(),
                features: RepositoryFeatures::default(),
                generation: 3,
            },
        );
        current.apply_scan(Err(GitError::new(GitErrorKind::Other, "status failed")));
        assert_eq!(current.repositories.len(), 1);
        assert_eq!(current.generation, 3);
        assert!(current.last_error.is_some());
        assert!(!current.initializing);
    }

    #[test]
    fn scans_differing_only_in_generation_are_the_same() {
        let repository = RepositorySnapshot {
            id: RepositoryId(1),
            worktree_root: PathBuf::from("/workspace"),
            git_dir: PathBuf::new(),
            common_dir: PathBuf::new(),
            head: HeadState::default(),
            upstream: None,
            ahead: 0,
            behind: 0,
            files: Arc::default(),
            ignored: Default::default(),
            repository_state: RepositoryState::Normal,
            operation_step: None,
            head_oid: None,
            stashes: Default::default(),
            tags: Default::default(),
            worktrees: Default::default(),
            features: RepositoryFeatures::default(),
            generation: 1,
        };
        let scan = |generation, repository: RepositorySnapshot| {
            let mut snapshot = GitStoreSnapshot {
                generation,
                initializing: false,
                ..GitStoreSnapshot::default()
            };
            Arc::make_mut(&mut snapshot.repositories).insert(
                repository.id,
                RepositorySnapshot {
                    generation,
                    ..repository
                },
            );
            snapshot
        };
        let last = scan(1, repository.clone());
        assert!(same_scan(&last, &scan(2, repository.clone())));
        let tracking = RepositorySnapshot {
            upstream: Some(UpstreamState {
                name: "origin/main".into(),
            }),
            ..repository
        };
        assert!(!same_scan(&last, &scan(2, tracking)));
    }

    #[test]
    fn unchanged_scans_repair_state_changed_by_another_writer() {
        let mut current = GitStoreSnapshot {
            initializing: false,
            generation: 1,
            ..Default::default()
        };
        current.operation = Some(operation(OperationKind::Fetch, "Fetching…", false));
        assert!(!current.apply_scan(Ok(GitStoreSnapshot {
            initializing: false,
            generation: 2,
            ..Default::default()
        })));
        assert_eq!(current.generation, 2);
        assert!(current.operation.is_some());
        current.last_error = Some(GitError::new(GitErrorKind::Other, "operation failed"));
        Arc::make_mut(&mut current.line_changes).insert(
            PathBuf::from("old.txt"),
            BTreeMap::from([(1, LineChange::Added)]),
        );
        assert!(current.apply_scan(Ok(GitStoreSnapshot {
            initializing: false,
            generation: 3,
            ..Default::default()
        })));
        assert!(current.last_error.is_none());
        assert!(current.line_changes.is_empty());
        assert!(!current.apply_scan(Ok(GitStoreSnapshot {
            initializing: false,
            generation: 2,
            ..Default::default()
        })));
        assert_eq!(current.generation, 3);
    }

    #[test]
    fn large_snapshots_share_render_data_and_unchanged_scans_do_not_repaint() {
        let files = (0..50_000)
            .map(|index| {
                (
                    PathBuf::from(format!("src/file-{index:05}.txt")),
                    FileState {
                        worktree: ChangeKind::Modified,
                        ..Default::default()
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let repository = RepositorySnapshot {
            id: RepositoryId(1),
            worktree_root: PathBuf::from("/repo"),
            git_dir: PathBuf::new(),
            common_dir: PathBuf::new(),
            head: HeadState::default(),
            upstream: None,
            ahead: 0,
            behind: 0,
            files: files.into(),
            ignored: Default::default(),
            repository_state: RepositoryState::Normal,
            operation_step: None,
            head_oid: None,
            stashes: Default::default(),
            tags: Default::default(),
            worktrees: Default::default(),
            features: RepositoryFeatures::default(),
            generation: 1,
        };
        let mut current = GitStoreSnapshot {
            repositories: BTreeMap::from([(repository.id, repository.clone())]).into(),
            generation: 1,
            initializing: false,
            ..Default::default()
        };
        current.prepare_for_ui(None);
        assert_eq!(
            current.presentation.repositories[&repository.id].sections[2].len(),
            50_000
        );
        assert_eq!(
            current.decoration_for_absolute_path(Path::new("/repo/src")),
            Some(PathDecoration::Modified)
        );
        let render_copy = current.clone();
        assert!(Arc::ptr_eq(
            &current.repositories,
            &render_copy.repositories
        ));
        assert!(Arc::ptr_eq(
            &current.presentation,
            &render_copy.presentation
        ));
        assert!(Arc::ptr_eq(
            &repository.files,
            &render_copy.active().unwrap().files
        ));
        let mut next = GitStoreSnapshot {
            repositories: BTreeMap::from([(
                repository.id,
                RepositorySnapshot {
                    generation: 2,
                    ..repository
                },
            )])
            .into(),
            generation: 2,
            initializing: false,
            ..Default::default()
        };
        next.prepare_for_ui(Some(&current));
        assert!(Arc::ptr_eq(&next.repositories, &current.repositories));
        assert!(Arc::ptr_eq(&next.presentation, &current.presentation));
        assert!(!current.apply_scan(Ok(next)));
        assert_eq!(current.generation, 2);
    }
}
