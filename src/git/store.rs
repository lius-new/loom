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

#[derive(Clone, Debug)]
pub struct GitStoreSnapshot {
    pub repositories: BTreeMap<RepositoryId, RepositorySnapshot>,
    pub active_repository: Option<RepositoryId>,
    pub operation: Option<OperationState>,
    pub last_error: Option<GitError>,
    pub runtime_label: Option<String>,
    pub generation: u64,
    /// Git line decorations for currently active files, keyed by absolute path
    /// and one-based line number.
    pub line_changes: BTreeMap<PathBuf, BTreeMap<usize, LineChange>>,
    pub initializing: bool,
}

impl Default for GitStoreSnapshot {
    fn default() -> Self {
        Self {
            repositories: BTreeMap::new(),
            active_repository: None,
            operation: None,
            last_error: None,
            runtime_label: None,
            generation: 0,
            line_changes: BTreeMap::new(),
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
    pub fn apply_scan(&mut self, result: GitResult<Self>) {
        match result {
            Ok(next) => {
                self.replace_if_newer(next);
            }
            Err(error) => {
                self.last_error = Some(error);
                self.initializing = false;
            }
        }
    }
}

/// What the UI tells the status poller between scans. The active file only
/// selects which file gets line decorations, so switching tabs updates this
/// instead of restarting the poller.
#[derive(Clone, Debug, Default)]
pub struct PollControl {
    shared: Arc<(Mutex<PollRequest>, Condvar)>,
}

#[derive(Debug, Default)]
struct PollRequest {
    active_path: Option<PathBuf>,
    wake: bool,
    paused: bool,
}

impl PollControl {
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

    /// Point line decorations at `path` and scan again without waiting for
    /// the polling interval.
    pub fn set_active_path(&self, path: Option<PathBuf>) {
        let (request, signal) = &*self.shared;
        let mut request = request.lock().unwrap_or_else(PoisonError::into_inner);
        if request.active_path != path {
            request.active_path = path;
            request.wake = true;
            signal.notify_all();
        }
    }

    /// The path for the scan about to start. That scan satisfies any pending
    /// wake request.
    fn begin_scan(&self) -> Option<PathBuf> {
        let (request, _) = &*self.shared;
        let mut request = request.lock().unwrap_or_else(PoisonError::into_inner);
        request.wake = false;
        request.active_path.clone()
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
            repositories,
            active_repository: None,
            operation: None,
            last_error: None,
            runtime_label: Some(format!(
                "{:?} Git {}.{}.{}",
                runtime.source, runtime.version.major, runtime.version.minor, runtime.version.patch
            )),
            generation,
            line_changes: BTreeMap::new(),
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
                snapshot.line_changes.insert(path.to_path_buf(), changes);
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

    /// Scan `roots` until `stopped` is set, publishing only results that
    /// differ from the previous one.
    pub fn poll(
        &self,
        roots: &[PathBuf],
        control: &PollControl,
        stopped: &AtomicBool,
        interval: Duration,
        publish: &impl Fn(GitResult<GitStoreSnapshot>),
    ) {
        let mut previous = None;
        // Publish a useful state as soon as possible. This avoids blocking
        // the drawer on nested-repository discovery, untracked-file
        // traversal, and two active-file diffs.
        let mut mode = RefreshMode::Fast;
        while !stopped.load(Ordering::Acquire) {
            let active_path = control.begin_scan();
            let result = match mode {
                RefreshMode::Fast => self.refresh_fast(roots, active_path.as_deref()),
                RefreshMode::Detailed => self.refresh(roots, active_path.as_deref()),
            };
            // A stopped poller's roots are stale; its result must not land.
            if stopped.load(Ordering::Acquire) {
                break;
            }
            // An unchanged state or a repeated error must not wake the UI
            // every interval.
            let unchanged = match (&previous, &result) {
                (Some(Ok(last)), Ok(next)) => same_scan(last, next),
                (Some(Err(last)), Err(next)) => last == next,
                _ => false,
            };
            if !unchanged {
                previous = Some(result.clone());
                publish(result);
            }
            if mode == RefreshMode::Fast {
                // Always publish the first detailed result. It carries nested
                // repositories, untracked files, and active-file line changes
                // that are intentionally absent from the fast snapshot.
                mode = RefreshMode::Detailed;
                previous = None;
                continue;
            }
            if !control.wait(interval, stopped) {
                break;
            }
        }
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
        operation,
        last_error,
        runtime_label,
        generation: _,
        line_changes,
        initializing,
    } = last;
    repositories.len() == next.repositories.len()
        && repositories.iter().zip(&next.repositories).all(
            |((id, repository), (next_id, next_repository))| {
                id == next_id && repository.same_state(next_repository)
            },
        )
        && *active_repository == next.active_repository
        && *operation == next.operation
        && *last_error == next.last_error
        && *runtime_label == next.runtime_label
        && *line_changes == next.line_changes
        && *initializing == next.initializing
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
            files: BTreeMap::new(),
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
        store.repositories.insert(parent.id, parent);
        store.repositories.insert(child.id, child);
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
        current.repositories.insert(
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
                files: BTreeMap::new(),
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
        control.set_active_path(Some(PathBuf::from("/workspace/a")));
        let started = Instant::now();
        assert!(control.wait(Duration::from_secs(60), &stopped));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(control.begin_scan(), Some(PathBuf::from("/workspace/a")));
        // The same path is not a new request.
        control.set_active_path(Some(PathBuf::from("/workspace/a")));
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
            files: BTreeMap::new(),
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
            snapshot.repositories.insert(
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
        let waiter = {
            let control = control.clone();
            let stopped = Arc::clone(&stopped);
            std::thread::spawn(move || {
                let started = Instant::now();
                let scanned = control.wait(Duration::from_millis(10), &stopped);
                (scanned, started.elapsed())
            })
        };
        std::thread::sleep(Duration::from_millis(200));
        control.set_paused(false);
        let (scanned, waited) = waiter.join().unwrap();
        assert!(scanned);
        // The 10ms interval passed long before; only resuming ended the wait.
        assert!(waited >= Duration::from_millis(200));
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
}
