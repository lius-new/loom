//! Recursive workspace file watching with quiet-window event batching.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use notify::event::ModifyKind;
use notify::{Config, Event, EventKind, PollWatcher, RecommendedWatcher, RecursiveMode, Watcher};

use crate::model::workspace::ReconcileResult;
use crate::state::AppState;

const QUIET_WINDOW: Duration = Duration::from_millis(150);
const MAX_BATCH_LATENCY: Duration = Duration::from_secs(1);
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_BATCH_PATHS: usize = 4_096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileChangeKind {
    Create,
    Modify,
    Remove,
    Rescan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileChange {
    pub path: PathBuf,
    pub kind: FileChangeKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FileChangeBatch {
    pub changes: Vec<FileChange>,
    pub requires_full_rescan: bool,
}

impl FileChangeBatch {
    pub(crate) fn paths(&self) -> impl Iterator<Item = &Path> {
        self.changes.iter().map(|change| change.path.as_path())
    }
}

/// Apply one watcher batch to the two disk-backed models. Directory changes
/// are resolved by rereading cached listings, while open documents use their
/// clean/dirty reconciliation policy.
pub(crate) fn apply_batch(app: &mut AppState, batch: &FileChangeBatch) -> bool {
    let paths = batch.paths().map(Path::to_path_buf).collect::<Vec<_>>();
    let tree_changed = if batch.requires_full_rescan {
        crate::file_tree::refresh_all_loaded_directories(app)
    } else {
        crate::file_tree::refresh_affected_paths(
            app,
            batch
                .changes
                .iter()
                .filter(|change| change.kind != FileChangeKind::Modify)
                .map(|change| change.path.clone()),
        )
    };
    let document_changed = app
        .workspace
        .reconcile_changed_paths(&paths, batch.requires_full_rescan)
        .into_iter()
        .any(|result| {
            matches!(
                result,
                ReconcileResult::Reloaded(_)
                    | ReconcileResult::Conflict(_)
                    | ReconcileResult::Missing(_)
            )
        });
    tree_changed || document_changed
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WatchBackendKind {
    Native,
    Polling,
}

pub(crate) struct FileWatchHandle {
    stop: Sender<()>,
    worker: Option<JoinHandle<()>>,
    pub backend: WatchBackendKind,
}

impl Drop for FileWatchHandle {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

enum WatchBackend {
    Native(RecommendedWatcher),
    Polling(PollWatcher),
}

/// Start watching normalized workspace roots. Native notifications are used
/// when available; a polling watcher is the safe fallback for unsupported
/// filesystems and failed recursive registrations.
pub(crate) fn start(
    roots: Vec<PathBuf>,
    on_batch: impl Fn(FileChangeBatch) + Send + 'static,
) -> notify::Result<FileWatchHandle> {
    let roots = normalized_roots(roots);
    let (event_sender, event_receiver) = mpsc::channel::<notify::Result<Event>>();

    let native = build_native_watcher(&roots, event_sender.clone());
    let (backend, backend_kind) = match native {
        Ok(watcher) => (WatchBackend::Native(watcher), WatchBackendKind::Native),
        Err(error) => {
            eprintln!("native file watching unavailable ({error}); using polling");
            (
                WatchBackend::Polling(build_poll_watcher(&roots, event_sender)?),
                WatchBackendKind::Polling,
            )
        }
    };

    let (stop, stop_receiver) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("loom-file-watcher".into())
        .spawn(move || {
            // Keep the watcher alive for the lifetime of the receiver loop.
            let _backend = backend;
            let mut pending = Vec::new();
            while stop_receiver.try_recv().is_err() {
                match event_receiver.recv_timeout(QUIET_WINDOW) {
                    Ok(event) => {
                        pending.push(event);
                        let started = Instant::now();
                        loop {
                            if stop_receiver.try_recv().is_ok() {
                                return;
                            }
                            let remaining = MAX_BATCH_LATENCY.saturating_sub(started.elapsed());
                            if remaining.is_zero() {
                                break;
                            }
                            match event_receiver.recv_timeout(QUIET_WINDOW.min(remaining)) {
                                Ok(event) => pending.push(event),
                                Err(RecvTimeoutError::Timeout) => break,
                                Err(RecvTimeoutError::Disconnected) => break,
                            }
                        }
                        let batch = coalesce_events(std::mem::take(&mut pending), &roots);
                        if batch.requires_full_rescan || !batch.changes.is_empty() {
                            on_batch(batch);
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .map_err(notify::Error::io)?;

    Ok(FileWatchHandle {
        stop,
        worker: Some(worker),
        backend: backend_kind,
    })
}

fn build_native_watcher(
    roots: &[PathBuf],
    sender: Sender<notify::Result<Event>>,
) -> notify::Result<RecommendedWatcher> {
    let mut watcher = RecommendedWatcher::new(
        move |event| {
            let _ = sender.send(event);
        },
        Config::default(),
    )?;
    watch_roots(&mut watcher, roots)?;
    Ok(watcher)
}

fn build_poll_watcher(
    roots: &[PathBuf],
    sender: Sender<notify::Result<Event>>,
) -> notify::Result<PollWatcher> {
    let mut watcher = PollWatcher::new(
        move |event| {
            let _ = sender.send(event);
        },
        Config::default().with_poll_interval(POLL_INTERVAL),
    )?;
    watch_roots(&mut watcher, roots)?;
    Ok(watcher)
}

fn watch_roots(watcher: &mut impl Watcher, roots: &[PathBuf]) -> notify::Result<()> {
    for root in roots {
        watcher.watch(root, RecursiveMode::Recursive)?;
    }
    Ok(())
}

fn normalized_roots(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut roots = roots
        .into_iter()
        .filter(|root| root.is_dir())
        .map(|root| std::fs::canonicalize(&root).unwrap_or(root))
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();

    let mut reduced = Vec::<PathBuf>::new();
    for root in roots {
        if !reduced.iter().any(|parent| root.starts_with(parent)) {
            reduced.push(root);
        }
    }
    reduced
}

fn coalesce_events(events: Vec<notify::Result<Event>>, roots: &[PathBuf]) -> FileChangeBatch {
    let mut by_path = HashMap::<PathBuf, FileChangeKind>::new();
    let mut requires_full_rescan = false;

    for event in events {
        let Ok(event) = event else {
            requires_full_rescan = true;
            continue;
        };
        if event.need_rescan() {
            requires_full_rescan = true;
        }
        let Some(kind) = change_kind(&event.kind) else {
            continue;
        };
        if kind == FileChangeKind::Rescan {
            requires_full_rescan = true;
        }
        for path in event.paths {
            let path = absolute_event_path(path, roots);
            if !roots.iter().any(|root| path.starts_with(root)) {
                continue;
            }
            let entry = by_path.entry(path).or_insert(kind);
            if kind_priority(kind) > kind_priority(*entry) {
                *entry = kind;
            }
            if by_path.len() > MAX_BATCH_PATHS {
                requires_full_rescan = true;
                by_path.clear();
                break;
            }
        }
    }

    let mut changes = by_path
        .into_iter()
        .map(|(path, kind)| FileChange { path, kind })
        .collect::<Vec<_>>();
    changes.sort_by(|left, right| left.path.cmp(&right.path));
    FileChangeBatch {
        changes,
        requires_full_rescan,
    }
}

fn change_kind(kind: &EventKind) -> Option<FileChangeKind> {
    match kind {
        EventKind::Create(_) => Some(FileChangeKind::Create),
        // Rename notifications contain the old and/or new paths depending on
        // the platform. Treat both as structural hints so their parents are
        // reread without trying to pair the events.
        EventKind::Modify(ModifyKind::Name(_)) => Some(FileChangeKind::Create),
        EventKind::Modify(_) => Some(FileChangeKind::Modify),
        EventKind::Remove(_) => Some(FileChangeKind::Remove),
        EventKind::Any | EventKind::Other => Some(FileChangeKind::Rescan),
        EventKind::Access(_) => None,
    }
}

fn kind_priority(kind: FileChangeKind) -> u8 {
    match kind {
        FileChangeKind::Modify => 1,
        FileChangeKind::Create => 2,
        FileChangeKind::Remove => 3,
        FileChangeKind::Rescan => 4,
    }
}

fn absolute_event_path(path: PathBuf, roots: &[PathBuf]) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        roots.first().map_or(path.clone(), |root| root.join(path))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    fn temp_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "loom-file-watcher-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        std::fs::canonicalize(path).unwrap()
    }

    fn event(kind: EventKind, paths: impl IntoIterator<Item = PathBuf>) -> notify::Result<Event> {
        let mut event = Event::new(kind);
        event.paths.extend(paths);
        Ok(event)
    }

    #[test]
    fn roots_are_deduplicated_and_nested_roots_are_removed() {
        let root = std::env::temp_dir();
        assert_eq!(
            normalized_roots(vec![root.join("missing"), root.clone(), root.clone()]),
            [std::fs::canonicalize(root).unwrap()]
        );
    }

    #[test]
    fn batches_deduplicate_paths_and_keep_strongest_change() {
        let root = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let path = root.join("loom-watch-test.txt");
        let batch = coalesce_events(
            vec![
                event(EventKind::Modify(ModifyKind::Any), [path.clone()]),
                event(EventKind::Create(CreateKind::Any), [path.clone()]),
                event(EventKind::Remove(RemoveKind::Any), [path.clone()]),
            ],
            std::slice::from_ref(&root),
        );
        assert_eq!(
            batch.changes,
            [FileChange {
                path,
                kind: FileChangeKind::Remove
            }]
        );
    }

    #[test]
    fn errors_request_a_full_rescan() {
        let root = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let batch = coalesce_events(
            vec![Err(notify::Error::generic("watch failed"))],
            std::slice::from_ref(&root),
        );
        assert!(batch.requires_full_rescan);
    }

    #[test]
    fn rename_paths_are_structural_changes() {
        let root = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let old = root.join("old.txt");
        let new = root.join("new.txt");
        let batch = coalesce_events(
            vec![event(
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                [old.clone(), new.clone()],
            )],
            std::slice::from_ref(&root),
        );
        assert_eq!(
            batch.changes,
            [
                FileChange {
                    path: new,
                    kind: FileChangeKind::Create,
                },
                FileChange {
                    path: old,
                    kind: FileChangeKind::Create,
                }
            ]
        );
    }

    #[test]
    fn oversized_batches_degrade_to_a_full_rescan() {
        let root = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let events = (0..=MAX_BATCH_PATHS)
            .map(|index| {
                event(
                    EventKind::Create(CreateKind::File),
                    [root.join(format!("generated-{index}.txt"))],
                )
            })
            .collect();
        let batch = coalesce_events(events, std::slice::from_ref(&root));
        assert!(batch.requires_full_rescan);
    }

    #[test]
    fn events_outside_roots_are_filtered() {
        let root = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let outside = PathBuf::from(if cfg!(windows) {
            r"Z:\outside.txt"
        } else {
            "/loom-outside.txt"
        });
        let batch = coalesce_events(
            vec![event(EventKind::Modify(ModifyKind::Any), [outside])],
            std::slice::from_ref(&root),
        );
        assert!(batch.changes.is_empty());
    }

    #[test]
    fn applying_batch_refreshes_tree_and_clean_open_document() {
        let root = temp_directory("apply");
        let open_path = root.join("open.txt");
        let added_path = root.join("added.txt");
        fs::write(&open_path, "before").unwrap();

        let mut app = AppState::new();
        app.workspace_folders.push(root.clone());
        let root_key = root.to_string_lossy().into_owned();
        app.dir_entries
            .insert(root_key.clone(), crate::file_tree::read_directory(&root));
        app.workspace.open_path(open_path.clone(), "before".into());

        fs::write(&open_path, "after and longer").unwrap();
        fs::write(&added_path, "added").unwrap();
        let batch = FileChangeBatch {
            changes: vec![
                FileChange {
                    path: open_path,
                    kind: FileChangeKind::Modify,
                },
                FileChange {
                    path: added_path,
                    kind: FileChangeKind::Create,
                },
            ],
            requires_full_rescan: false,
        };

        assert!(apply_batch(&mut app, &batch));
        assert_eq!(
            app.workspace.active_buffer().unwrap().text(),
            "after and longer"
        );
        assert!(
            app.dir_entries[&root_key]
                .iter()
                .any(|entry| entry.name == "added.txt")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn watcher_reports_external_creation_and_stops_cleanly() {
        let root = temp_directory("service");
        let created = root.join("created.txt");
        let (sender, receiver) = mpsc::channel();
        let watcher = start(vec![root.clone()], move |batch| {
            let _ = sender.send(batch);
        })
        .unwrap();

        fs::write(&created, "created").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut observed = false;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let Ok(batch) = receiver.recv_timeout(remaining) else {
                break;
            };
            if batch.paths().any(|path| path == created) {
                observed = true;
                break;
            }
        }
        assert!(observed, "watcher did not report the created file");

        drop(watcher);
        fs::remove_dir_all(root).unwrap();
    }
}
