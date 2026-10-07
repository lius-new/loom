use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, SystemTime};

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
    pub fn replace_if_newer(&mut self, next: Self) -> bool {
        if next.generation < self.generation {
            return false;
        }
        *self = next;
        true
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

    pub fn start_polling(
        self: &Arc<Self>,
        roots: Vec<PathBuf>,
        active_path: Option<PathBuf>,
        interval: Duration,
        publish: impl Fn(GitStoreSnapshot) + Send + 'static,
    ) -> PollingHandle {
        let (stop_sender, stop_receiver) = mpsc::channel();
        let service = Arc::clone(self);
        let worker = thread::Builder::new()
            .name("loom-git-status".into())
            .spawn(move || {
                let mut previous = None;
                // Publish a useful state as soon as possible. This avoids
                // blocking the drawer on nested-repository discovery,
                // untracked-file traversal, and two active-file diffs.
                publish_refresh(
                    &service,
                    &roots,
                    active_path.as_deref(),
                    true,
                    &mut previous,
                    &publish,
                );
                // Always publish the first detailed result. It carries nested
                // repositories, untracked files, and active-file line changes
                // that are intentionally absent from the fast snapshot.
                previous = None;
                loop {
                    publish_refresh(
                        &service,
                        &roots,
                        active_path.as_deref(),
                        false,
                        &mut previous,
                        &publish,
                    );
                    if stop_receiver.recv_timeout(interval).is_ok() {
                        break;
                    }
                }
            })
            .ok();
        PollingHandle {
            stop_sender: Some(stop_sender),
            worker,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefreshMode {
    Fast,
    Detailed,
}

fn publish_refresh(
    service: &GitService,
    roots: &[PathBuf],
    active_path: Option<&Path>,
    fast: bool,
    previous: &mut Option<u64>,
    publish: &impl Fn(GitStoreSnapshot),
) {
    let result = if fast {
        service.refresh_fast(roots, active_path)
    } else {
        service.refresh(roots, active_path)
    };
    match result {
        Ok(snapshot) => {
            let fingerprint = snapshot_fingerprint(&snapshot);
            if *previous != Some(fingerprint) {
                *previous = Some(fingerprint);
                publish(snapshot);
            }
        }
        Err(error) => publish(GitStoreSnapshot {
            last_error: Some(error),
            generation: service.generation.load(Ordering::Acquire),
            initializing: false,
            ..GitStoreSnapshot::default()
        }),
    }
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

pub struct PollingHandle {
    stop_sender: Option<mpsc::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Drop for PollingHandle {
    fn drop(&mut self) {
        if let Some(sender) = self.stop_sender.take() {
            let _ = sender.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
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

fn snapshot_fingerprint(snapshot: &GitStoreSnapshot) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    snapshot.active_repository.hash(&mut hasher);
    for (id, repository) in &snapshot.repositories {
        id.hash(&mut hasher);
        repository.worktree_root.hash(&mut hasher);
        repository.branch_label().hash(&mut hasher);
        repository.ahead.hash(&mut hasher);
        repository.behind.hash(&mut hasher);
        repository.repository_state.hash(&mut hasher);
        for (path, state) in &repository.files {
            path.hash(&mut hasher);
            format!("{state:?}").hash(&mut hasher);
        }
        repository.ignored.hash(&mut hasher);
    }
    for (path, changes) in &snapshot.line_changes {
        path.hash(&mut hasher);
        for (line, change) in changes {
            line.hash(&mut hasher);
            match change {
                LineChange::Added => 0_u8,
                LineChange::Modified => 1_u8,
                LineChange::Deleted => 2_u8,
            }
            .hash(&mut hasher);
        }
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
