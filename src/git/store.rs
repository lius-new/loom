use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use super::backend::{CliGitBackend, GitBackend};
use super::command::CancellationToken;
use super::error::{GitError, GitResult};
use super::types::*;
use crate::model::diff_document::DiffDocument;

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
            let document = match content {
                DiffContent::Unified(diff) => DiffDocument::from_unified(
                    request.repository_root.clone(),
                    request.path.clone(),
                    request.target,
                    diff.clone(),
                ),
                DiffContent::Untracked(contents) => DiffDocument::added(
                    request.repository_root.clone(),
                    request.path.clone(),
                    request.target,
                    contents,
                ),
            };
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

/// What the UI tells the status poller between scans. Tab changes retarget
/// line decorations and open diffs without restarting the worker.
#[derive(Clone, Debug, Default)]
pub struct PollControl {
    shared: Arc<(Mutex<PollRequest>, Condvar)>,
}

#[derive(Debug, Default)]
struct PollRequest {
    target: PollTarget,
    revision: u64,
    wake: bool,
    paused: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PollTarget {
    pub active_path: Option<PathBuf>,
    pub diffs: Vec<DiffRequest>,
}

impl PollControl {
    pub(super) fn target(&self) -> PollTarget {
        self.shared
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .target
            .clone()
    }
    /// Stop interval scans while the window is in the background. Resuming
    /// scans immediately, since anything may have changed meanwhile.
    pub fn set_paused(&self, paused: bool) {
        let (request, signal) = &*self.shared;
        let mut request = request.lock().unwrap_or_else(PoisonError::into_inner);
        if request.paused != paused {
            request.paused = paused;
            request.wake |= !paused;
            signal.notify_all();
        }
    }

    /// Retarget line decorations and open diffs without restarting the worker.
    pub fn set_target(&self, target: PollTarget) {
        let (request, signal) = &*self.shared;
        let mut request = request.lock().unwrap_or_else(PoisonError::into_inner);
        if request.target != target {
            request.target = target;
            request.revision += 1;
            request.wake = true;
            signal.notify_all();
        }
    }

    /// Invalidate an in-flight scan and refresh even if the window is paused.
    pub fn request_refresh(&self) {
        let (request, signal) = &*self.shared;
        let mut request = request.lock().unwrap_or_else(PoisonError::into_inner);
        request.revision += 1;
        request.wake = true;
        signal.notify_all();
    }

    /// The path for the scan about to start. That scan satisfies any pending
    /// wake request.
    fn begin_scan(&self) -> (u64, PollTarget) {
        let (request, _) = &*self.shared;
        let mut request = request.lock().unwrap_or_else(PoisonError::into_inner);
        request.wake = false;
        (request.revision, request.target.clone())
    }

    fn is_current(&self, revision: u64) -> bool {
        self.shared
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .revision
            == revision
    }

    /// Sleep until `interval` passes or a scan is requested; while paused,
    /// only a request ends the wait. Returns false once `stopped` is set.
    fn wait(&self, interval: Duration, stopped: &AtomicBool) -> bool {
        let (request, signal) = &*self.shared;
        let deadline = Instant::now() + interval;
        let mut request = request.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if stopped.load(Ordering::Acquire) {
                return false;
            }
            if request.wake {
                return true;
            }
            if request.paused {
                request = signal.wait(request).unwrap_or_else(PoisonError::into_inner);
                continue;
            }
            let now = Instant::now();
            if now >= deadline {
                return true;
            }
            request = signal
                .wait_timeout(request, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Wake every waiting poller so a stopped one can exit. Taking the lock
    /// orders this after a waiter's `stopped` check.
    pub(super) fn notify(&self) {
        let (request, signal) = &*self.shared;
        let _request = request.lock().unwrap_or_else(PoisonError::into_inner);
        signal.notify_all();
    }
}

#[derive(Debug)]
pub struct GitService {
    backend: CliGitBackend,
    generation: AtomicU64,
    write_locks: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
}

impl GitService {
    pub fn new(backend: CliGitBackend) -> Self {
        Self {
            backend,
            generation: AtomicU64::new(0),
            write_locks: Mutex::new(HashMap::new()),
        }
    }

    pub fn backend(&self) -> &CliGitBackend {
        &self.backend
    }

    pub fn refresh(
        &self,
        roots: &[PathBuf],
        active_path: Option<&Path>,
    ) -> GitResult<GitStoreSnapshot> {
        self.refresh_with_mode(roots, active_path, RefreshMode::Detailed)
    }

    fn refresh_fast(
        &self,
        roots: &[PathBuf],
        active_path: Option<&Path>,
    ) -> GitResult<GitStoreSnapshot> {
        self.refresh_with_mode(roots, active_path, RefreshMode::Fast)
    }

    fn refresh_with_mode(
        &self,
        roots: &[PathBuf],
        active_path: Option<&Path>,
        mode: RefreshMode,
    ) -> GitResult<GitStoreSnapshot> {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let discovered = match mode {
            RefreshMode::Fast => super::discovery::discover_fast(self.backend.runner(), roots)?,
            RefreshMode::Detailed => self.backend.discover(roots)?,
        };
        let mut repositories = BTreeMap::new();
        for repository in discovered {
            let id = repository_id(&repository.worktree_root);
            let snapshot = match mode {
                RefreshMode::Fast => self.backend.status_fast(&repository, id, generation)?,
                RefreshMode::Detailed => self.backend.status(&repository, id, generation)?,
            };
            repositories.insert(id, snapshot);
        }
        let runtime = self.backend.runtime();
        let mut snapshot = GitStoreSnapshot {
            repositories: repositories.into(),
            active_repository: None,
            operation: None,
            last_error: None,
            runtime_label: Some(format!(
                "{:?} Git {}.{}.{}",
                runtime.source, runtime.version.major, runtime.version.minor, runtime.version.patch
            )),
            generation,
            line_changes: Arc::default(),
            diffs: Arc::default(),
            presentation: Arc::default(),
            initializing: false,
        };
        snapshot.choose_active_for_path(active_path);
        if mode == RefreshMode::Detailed
            && let Some(path) = active_path
            && let Some(repository) = snapshot.repository_for_path(path)
            && let Some(relative) = repository.relative_path(path)
        {
            let mut changes = BTreeMap::new();
            for target in [DiffTarget::HeadToIndex, DiffTarget::IndexToWorktree] {
                if let Ok(diff) = self.backend.parsed_diff(
                    &repository.worktree_root,
                    target,
                    std::slice::from_ref(&relative),
                ) {
                    collect_line_changes(&diff, &mut changes);
                }
            }
            if !changes.is_empty() {
                Arc::make_mut(&mut snapshot.line_changes).insert(path.to_path_buf(), changes);
            }
        }
        Ok(snapshot)
    }

    pub fn write<T>(
        &self,
        repository: &Path,
        operation: impl FnOnce(&CliGitBackend) -> GitResult<T>,
    ) -> GitResult<T> {
        let lock = {
            let mut locks = self
                .write_locks
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            Arc::clone(
                locks
                    .entry(repository.to_path_buf())
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        operation(&self.backend)
    }

    pub fn clone_repository(
        &self,
        url: &str,
        target: &Path,
        cancellation: &CancellationToken,
    ) -> GitResult<()> {
        self.backend.clone_repository(url, target, cancellation)
    }

    /// Scan `roots` until stopped. The consumer compares each result with its
    /// actual state, since commands may change that state between scans.
    pub fn poll(
        &self,
        roots: &[PathBuf],
        control: &PollControl,
        stopped: &AtomicBool,
        interval: Duration,
        publish: &impl Fn(&PollTarget, GitResult<GitStoreSnapshot>),
    ) {
        let mut previous = None;
        // Publish a useful state as soon as possible. This avoids blocking
        // the drawer on nested-repository discovery, untracked-file
        // traversal, and two active-file diffs.
        let mut mode = RefreshMode::Fast;
        while !stopped.load(Ordering::Acquire) {
            let (revision, target) = control.begin_scan();
            let mut result = match mode {
                RefreshMode::Fast => self.refresh_fast(roots, target.active_path.as_deref()),
                RefreshMode::Detailed => self.refresh(roots, target.active_path.as_deref()),
            };
            if mode == RefreshMode::Detailed
                && let Ok(snapshot) = &mut result
            {
                for request in &target.diffs {
                    let content = self.refresh_diff(request, snapshot);
                    Arc::make_mut(&mut snapshot.diffs).insert(request.clone(), content);
                }
            }
            // A stopped poller's roots are stale; its result must not land.
            if stopped.load(Ordering::Acquire) {
                break;
            }
            if control.is_current(revision) {
                if let Ok(snapshot) = &mut result {
                    snapshot.prepare_for_ui(previous.as_ref());
                    previous = Some(snapshot.clone());
                }
                publish(&target, result);
            }
            if mode == RefreshMode::Fast {
                // Always publish the first detailed result. It carries nested
                // repositories, untracked files, and active-file line changes
                // that are intentionally absent from the fast snapshot.
                mode = RefreshMode::Detailed;
                continue;
            }
            if !control.wait(interval, stopped) {
                break;
            }
        }
    }

    fn refresh_diff(
        &self,
        request: &DiffRequest,
        snapshot: &GitStoreSnapshot,
    ) -> GitResult<DiffContent> {
        let diff = self.backend.parsed_full_diff(
            &request.repository_root,
            request.target,
            std::slice::from_ref(&request.path),
        )?;
        if diff.files.is_empty() && request.target != DiffTarget::HeadToIndex {
            let untracked = if let Some(repository) = snapshot
                .repositories
                .values()
                .find(|repository| repository.worktree_root == request.repository_root)
            {
                // Normal status can collapse an entire untracked directory.
                repository.files.iter().any(|(path, file)| {
                    request.path.starts_with(path) && file.worktree == ChangeKind::Untracked
                })
            } else {
                // Removing a workspace folder keeps its open tabs. Its diff
                // still needs to distinguish untracked content from an empty diff.
                !self
                    .backend
                    .runner()
                    .run(
                        super::command::GitCommand::new()
                            .cwd(&request.repository_root)
                            .args(["ls-files", "--others", "--exclude-standard", "-z", "--"])
                            .args([request.path.as_os_str()])
                            .read_only(),
                    )?
                    .stdout
                    .is_empty()
            };
            if untracked {
                return Ok(DiffContent::Untracked(std::fs::read_to_string(
                    request.repository_root.join(&request.path),
                )?));
            }
        }
        Ok(DiffContent::Unified(diff))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefreshMode {
    Fast,
    Detailed,
}

fn collect_line_changes(diff: &super::diff::UnifiedDiff, output: &mut BTreeMap<usize, LineChange>) {
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

fn repository_id(path: &Path) -> RepositoryId {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    RepositoryId(hasher.finish())
}

/// Whether two scans saw the same state; generations always differ.
/// Destructured so a new field cannot be left out of the comparison.
fn same_scan(last: &GitStoreSnapshot, next: &GitStoreSnapshot) -> bool {
    let GitStoreSnapshot {
        repositories,
        active_repository,
        operation: _,
        last_error,
        runtime_label,
        generation: _,
        line_changes,
        diffs,
        presentation: _,
        initializing,
    } = last;
    same_repositories(repositories, &next.repositories)
        && *active_repository == next.active_repository
        && *last_error == next.last_error
        && *runtime_label == next.runtime_label
        && *line_changes == next.line_changes
        && *diffs == next.diffs
        && *initializing == next.initializing
}

fn same_repositories(
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

fn same_shared_scan(last: &GitStoreSnapshot, next: &GitStoreSnapshot) -> bool {
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
    use crate::git::error::GitErrorKind;

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
    fn changing_the_active_path_wakes_the_poller() {
        let control = PollControl::default();
        let stopped = AtomicBool::new(false);
        let target = PollTarget {
            active_path: Some(PathBuf::from("/workspace/a")),
            ..Default::default()
        };
        control.set_target(target.clone());
        let started = Instant::now();
        assert!(control.wait(Duration::from_secs(60), &stopped));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(control.begin_scan().1, target);
        // The same path is not a new request.
        control.set_target(target);
        assert!(!control.shared.0.lock().unwrap().wake);
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
    fn a_paused_poller_waits_until_resumed() {
        let control = PollControl::default();
        let stopped = Arc::new(AtomicBool::new(false));
        control.set_paused(true);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let waiter = {
            let control = control.clone();
            let stopped = Arc::clone(&stopped);
            std::thread::spawn(move || {
                ready_tx.send(()).unwrap();
                let scanned = control.wait(Duration::from_millis(10), &stopped);
                done_tx.send(scanned).unwrap();
            })
        };
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let while_paused = done_rx.recv_timeout(Duration::from_millis(50));
        control.set_paused(false);
        waiter.join().unwrap();
        assert_eq!(
            while_paused,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        );
        assert!(done_rx.recv_timeout(Duration::from_secs(5)).unwrap());
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
    fn refresh_requests_invalidate_in_flight_scans_even_while_paused() {
        let control = PollControl::default();
        let (revision, _) = control.begin_scan();
        control.set_paused(true);
        control.request_refresh();
        assert!(!control.is_current(revision));
        assert!(control.wait(Duration::from_secs(60), &AtomicBool::new(false)));
        let (revision, _) = control.begin_scan();
        assert!(control.is_current(revision));
        control.set_target(PollTarget {
            active_path: Some(PathBuf::from("new.txt")),
            ..Default::default()
        });
        assert!(!control.is_current(revision));
    }

    #[test]
    fn stopping_ends_the_wait() {
        let control = PollControl::default();
        let stopped = Arc::new(AtomicBool::new(false));
        let waiter = {
            let control = control.clone();
            let stopped = Arc::clone(&stopped);
            std::thread::spawn(move || control.wait(Duration::from_secs(60), &stopped))
        };
        std::thread::sleep(Duration::from_millis(50));
        stopped.store(true, Ordering::Release);
        control.notify();
        assert!(!waiter.join().unwrap());
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
