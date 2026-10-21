//! LSP disk-event leases over the editor's generic notify source. All traversal
//! and reconciliation runs off the coordinator and UI threads.
use crate::features::language_service::{
    uri,
    watch::{Watch, path_text},
};
use crate::infrastructure::file_watcher::{self, FileChangeBatch, FileChangeKind};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

const MAX_ROOTS: usize = 16;
const MAX_ENTRIES: usize = 200_000;
const SCAN_BUDGET: Duration = Duration::from_secs(10);
const RECOVERY_INTERVAL: Duration = Duration::from_secs(30);

type Listener = dyn Fn(FileChangeBatch) + Send + Sync;
struct Source {
    listeners: Arc<Mutex<Vec<Weak<Listener>>>>,
    _watch: file_watcher::FileWatchHandle,
}

/// Physical roots are shared across instances and candidate replacements. A
/// subscription holds its source alive; the last lease drops the native watch.
struct SourceLease {
    _source: Arc<Source>,
    _listener: Arc<Listener>,
}

fn source_leases(
    roots: &[PathBuf],
    listener: impl Fn(FileChangeBatch) + Send + Sync + 'static,
) -> Result<Vec<SourceLease>, String> {
    static SOURCES: OnceLock<Mutex<BTreeMap<PathBuf, Weak<Source>>>> = OnceLock::new();
    let pool = SOURCES.get_or_init(Mutex::default);
    let listener: Arc<Listener> = Arc::new(listener);
    let mut leases = Vec::new();
    for root in roots {
        let root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
        let mut sources = pool.lock().unwrap();
        sources.retain(|_, source| source.strong_count() != 0);
        let source = if let Some(source) = sources.get(&root).and_then(Weak::upgrade) {
            source
        } else {
            if sources.len() >= 64 {
                return Err("global LSP watch root budget exceeded (64)".into());
            }
            let listeners = Arc::new(Mutex::new(Vec::<Weak<Listener>>::new()));
            let callbacks = listeners.clone();
            let watch = file_watcher::start_unfiltered(vec![root.clone()], move |batch| {
                let listeners = {
                    let mut callbacks = callbacks.lock().unwrap();
                    callbacks.retain(|callback| callback.strong_count() != 0);
                    callbacks
                        .iter()
                        .filter_map(Weak::upgrade)
                        .collect::<Vec<_>>()
                };
                for listener in listeners {
                    listener(batch.clone());
                }
            })
            .map_err(|e| e.to_string())?;
            let source = Arc::new(Source {
                listeners,
                _watch: watch,
            });
            sources.insert(root, Arc::downgrade(&source));
            source
        };
        source
            .listeners
            .lock()
            .unwrap()
            .push(Arc::downgrade(&listener));
        leases.push(SourceLease {
            _source: source,
            _listener: listener.clone(),
        });
    }
    Ok(leases)
}

enum Command {
    Activate,
    Checkpoint(Sender<Checkpoint>),
    Batch(FileChangeBatch),
    Stop,
}

pub(crate) struct Handle {
    tx: Sender<Command>,
    stopped: Arc<AtomicBool>,
}

impl Handle {
    pub fn activate(&self) {
        let _ = self.tx.send(Command::Activate);
    }

    fn checkpoint(&self) -> mpsc::Receiver<Checkpoint> {
        let (tx, inbox) = mpsc::channel();
        let _ = self.tx.send(Command::Checkpoint(tx));
        inbox
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = self.tx.send(Command::Stop);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Metadata {
    directory: bool,
    length: u64,
    modified: Option<SystemTime>,
}

type Snapshot = BTreeMap<PathBuf, Metadata>;

struct Checkpoint {
    snapshot: Snapshot,
    watches: Vec<Watch>,
}

/// Continue the old lease's baseline for paths covered by both subscriptions.
/// Newly watched paths start at today's baseline, without synthetic creations.
fn inherit(baseline: &mut Snapshot, checkpoint: Checkpoint, watches: &[Watch], roots: &[PathBuf]) {
    let continuing = |path: &Path| {
        checkpoint
            .watches
            .iter()
            .any(|watch| watch.pattern.matches(path))
            && watches.iter().any(|watch| watch.pattern.matches(path))
    };
    baseline.retain(|path, _| !continuing(path));
    baseline.extend(
        checkpoint.snapshot.into_iter().filter(|(path, _)| {
            roots.iter().any(|root| path.starts_with(root)) && continuing(path)
        }),
    );
}

fn roots(watches: &[Watch]) -> Result<Vec<PathBuf>, String> {
    let mut roots = watches
        .iter()
        .filter(|watch| watch.kind != 0)
        .map(|watch| {
            let mut root = watch.pattern.root.clone();
            if watch.pattern.allow_parent {
                while !root.is_dir() {
                    if !root.pop() {
                        return Err("absolute glob has no existing ancestor".to_owned());
                    }
                }
            }
            Ok(root)
        })
        .collect::<Result<Vec<_>, String>>()?;
    roots.sort();
    roots.dedup();
    let mut reduced = Vec::<PathBuf>::new();
    for root in roots {
        if !root.is_absolute() || !root.is_dir() {
            return Err(format!(
                "watch root is not an existing directory: {}",
                root.display()
            ));
        }
        let root = uri::file_path(&uri::file_uri(&root).ok_or("watch root is not a file path")?)
            .ok_or("invalid watch root")?;
        let text = path_text(&root).ok_or("invalid watch root")?;
        if !reduced.iter().any(|parent| {
            path_text(parent).is_some_and(|parent| {
                text == parent || text.starts_with(&(parent.trim_end_matches('/').to_owned() + "/"))
            })
        }) {
            reduced.push(root);
        }
    }
    if reduced.len() > MAX_ROOTS {
        return Err(format!("watch root budget exceeded (maximum {MAX_ROOTS})"));
    }
    Ok(reduced)
}

fn metadata(path: &Path) -> Result<Option<(Metadata, bool)>, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            let directory = metadata.is_dir();
            Ok(Some((
                Metadata {
                    directory,
                    length: if directory { 0 } else { metadata.len() },
                    modified: if directory {
                        None
                    } else {
                        metadata.modified().ok()
                    },
                },
                directory && !metadata.file_type().is_symlink(),
            )))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

#[cfg(test)]
fn scan(roots: &[PathBuf], stopped: &AtomicBool) -> Result<Snapshot, String> {
    scan_tree(roots, stopped, true, &[])
}

fn scan_tree(
    roots: &[PathBuf],
    stopped: &AtomicBool,
    follow_root_symlinks: bool,
    watches: &[Watch],
) -> Result<Snapshot, String> {
    let started = Instant::now();
    let mut todo = roots.to_vec();
    let mut snapshot = Snapshot::new();
    while let Some(path) = todo.pop() {
        if stopped.load(Ordering::Acquire) {
            return Err("watch registration cancelled".into());
        }
        if !watches.is_empty()
            && !watches
                .iter()
                .any(|watch| watch.pattern.matches(&path) || watch.pattern.may_descend(&path))
        {
            continue;
        }
        if snapshot.len() >= MAX_ENTRIES || started.elapsed() > SCAN_BUDGET {
            return Err("watch scan budget exceeded (200000 entries / 10 seconds)".into());
        }
        let Some((mut metadata, mut recurse)) = metadata(&path)? else {
            continue;
        };
        // A symlink supplied as a root is an explicit lease. Do not follow
        // symlinks discovered recursively, which could escape that lease.
        if follow_root_symlinks && roots.contains(&path) && !recurse && path.is_dir() {
            metadata.directory = true;
            metadata.length = 0;
            metadata.modified = None;
            recurse = true;
        }
        if recurse
            && (watches.is_empty() || watches.iter().any(|watch| watch.pattern.may_descend(&path)))
        {
            let entries = std::fs::read_dir(&path)
                .map_err(|e| format!("cannot scan {}: {e}", path.display()))?;
            for entry in entries {
                todo.push(entry.map_err(|e| e.to_string())?.path());
                if todo.len() + snapshot.len() > MAX_ENTRIES {
                    return Err("watch scan entry budget exceeded".into());
                }
            }
        }
        snapshot.insert(path, metadata);
    }
    Ok(snapshot)
}

fn changes(
    before: &Snapshot,
    after: &Snapshot,
    hints: &FileChangeBatch,
    watches: &[Watch],
) -> Vec<Value> {
    let mut changes = BTreeMap::<String, u8>::new();
    let touched = hints
        .changes
        .iter()
        .filter(|hint| hint.kind != FileChangeKind::Rescan)
        .map(|hint| &hint.path)
        .collect::<BTreeSet<_>>();
    for (path, old) in before {
        let kind = match after.get(path) {
            None => 3,
            Some(new) if new != old => 2,
            // A native modify event also covers replacements with equal timestamps.
            Some(_) if touched.contains(path) => 2,
            _ => continue,
        };
        if watches.iter().any(|watch| watch.matches(path, kind))
            && let Some(uri) = uri::file_uri(path)
        {
            changes.insert(uri, kind);
        }
    }
    for path in after.keys().filter(|path| !before.contains_key(*path)) {
        if watches.iter().any(|watch| watch.matches(path, 1))
            && let Some(uri) = uri::file_uri(path)
        {
            changes.insert(uri, 1);
        }
    }
    changes
        .into_iter()
        .map(|(uri, kind)| json!({"uri": uri, "type": kind}))
        .collect()
}

/// Build a candidate snapshot without changing the known baseline on failure.
#[cfg(test)]
fn reconcile(
    before: &Snapshot,
    roots: &[PathBuf],
    hints: &FileChangeBatch,
    stopped: &AtomicBool,
) -> Result<Snapshot, String> {
    reconcile_matching(before, roots, hints, stopped, &[])
}

fn reconcile_matching(
    before: &Snapshot,
    roots: &[PathBuf],
    hints: &FileChangeBatch,
    stopped: &AtomicBool,
    watches: &[Watch],
) -> Result<Snapshot, String> {
    if hints.requires_full_rescan {
        return scan_tree(roots, stopped, true, watches);
    }
    let mut after = before.clone();
    let mut paths = hints
        .changes
        .iter()
        .map(|hint| hint.path.clone())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    for path in paths {
        if !roots.iter().any(|root| path.starts_with(root)) {
            continue;
        }
        if !watches.is_empty()
            && !watches
                .iter()
                .any(|watch| watch.pattern.matches(&path) || watch.pattern.may_descend(&path))
        {
            continue;
        }
        // A directory rename has only two hints on some backends; reconcile its
        // entire subtree so closed files also get old/delete + new/create.
        after.retain(|known, _| !known.starts_with(&path));
        after.extend(scan_tree(
            std::slice::from_ref(&path),
            stopped,
            roots.contains(&path),
            watches,
        )?);
        if after.len() > MAX_ENTRIES {
            return Err("watch snapshot budget exceeded".into());
        }
    }
    Ok(after)
}

/// Success is reported only after roots are leased and the baseline is read.
/// Events remain buffered until the coordinator commits and activates the lease.
pub(crate) fn prepare(
    watches: Vec<Watch>,
    previous: Option<&Handle>,
    on_ready: impl FnOnce(Result<(), String>) + Send + 'static,
    on_changes: impl Fn(Vec<Value>) + Send + 'static,
    on_error: impl Fn(String) + Send + 'static,
) -> std::io::Result<Handle> {
    // Pause the previous stream at an ordered checkpoint. Commit transfers its
    // baseline; rollback reactivates it and reconciles everything buffered.
    let checkpoint = previous.map(Handle::checkpoint);
    let (tx, inbox) = mpsc::channel();
    let sender = tx.clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let cancellation = stopped.clone();
    thread::Builder::new()
        .name("loom-lsp-watch".into())
        .spawn(move || {
            let setup = (|| {
                let checkpoint = checkpoint
                    .map(|inbox| {
                        inbox
                            .recv_timeout(Duration::from_secs(5))
                            .map_err(|_| "previous watch lease did not checkpoint".to_owned())
                    })
                    .transpose()?;
                let roots = roots(&watches)?;
                // Preflight bounds before a polling backend can allocate its tree.
                let _ = scan_tree(&roots, &cancellation, true, &watches)?;
                let aliases = roots
                    .iter()
                    .map(|logical| {
                        let physical = std::fs::canonicalize(logical).map_err(|e| e.to_string())?;
                        let physical = uri::file_path(
                            &uri::file_uri(&physical).ok_or("invalid canonical root")?,
                        )
                        .ok_or("invalid canonical root")?;
                        Ok::<_, String>((physical, logical.clone()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let source = source_leases(&roots, move |mut batch| {
                    for hint in &mut batch.changes {
                        if let Some(path) =
                            uri::file_uri(&hint.path).and_then(|uri| uri::file_path(&uri))
                        {
                            hint.path = aliases
                                .iter()
                                .find_map(|(physical, logical)| {
                                    path.strip_prefix(physical)
                                        .ok()
                                        .map(|relative| logical.join(relative))
                                })
                                .unwrap_or(path);
                        }
                    }
                    let _ = sender.send(Command::Batch(batch));
                })?;
                let mut baseline = scan_tree(&roots, &cancellation, true, &watches)?;
                if let Some(checkpoint) = checkpoint {
                    inherit(&mut baseline, checkpoint, &watches, &roots);
                }
                if roots.iter().any(|root| !root.is_dir()) {
                    return Err("a watch root disappeared during registration".into());
                }
                Ok::<_, String>((roots, source, baseline))
            })();
            let (roots, _source, mut baseline) = match setup {
                Ok(setup) => setup,
                Err(error) => {
                    on_ready(Err(error));
                    return;
                }
            };
            on_ready(Ok(()));
            let mut active = false;
            let mut buffered = Vec::new();
            let mut recovery_due = Instant::now() + RECOVERY_INTERVAL;
            while !cancellation.load(Ordering::Acquire) {
                let command = match inbox
                    .recv_timeout(recovery_due.saturating_duration_since(Instant::now()))
                {
                    Ok(command) => command,
                    Err(mpsc::RecvTimeoutError::Timeout) => Command::Batch(FileChangeBatch {
                        changes: Vec::new(),
                        requires_full_rescan: true,
                    }),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                let batch = match command {
                    Command::Stop => break,
                    Command::Checkpoint(reply) => {
                        active = false;
                        let _ = reply.send(Checkpoint {
                            snapshot: baseline.clone(),
                            watches: watches.clone(),
                        });
                        continue;
                    }
                    Command::Activate => {
                        active = true;
                        let changes = buffered
                            .drain(..)
                            .flat_map(|batch: FileChangeBatch| batch.changes)
                            .collect();
                        FileChangeBatch {
                            changes,
                            requires_full_rescan: true,
                        }
                    }
                    Command::Batch(batch) if !active => {
                        if buffered.len() < 64 {
                            buffered.push(batch);
                        }
                        recovery_due = Instant::now() + RECOVERY_INTERVAL;
                        continue;
                    }
                    Command::Batch(batch) => batch,
                };
                recovery_due = Instant::now() + RECOVERY_INTERVAL;
                match reconcile_matching(&baseline, &roots, &batch, &cancellation, &watches) {
                    Ok(after) => {
                        let events = changes(&baseline, &after, &batch, &watches);
                        baseline = after;
                        if !events.is_empty() && !cancellation.load(Ordering::Acquire) {
                            on_changes(events);
                        }
                    }
                    Err(error) if !cancellation.load(Ordering::Acquire) => {
                        on_error(format!(
                            "file watch reconciliation failed; retrying: {error}"
                        ));
                        recovery_due = Instant::now() + Duration::from_secs(2);
                    }
                    Err(_) => break,
                }
            }
        })?;
    Ok(Handle { tx, stopped })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::file_watcher::FileChange;
    use std::sync::atomic::AtomicUsize;

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Tree(PathBuf);
    impl Tree {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "loom-watch-{}-{}",
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn watches(&self) -> Vec<Watch> {
            Watch::parse(&json!({"watchers":[{"globPattern":"**/*.wat"}]}), &self.0).unwrap()
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn literal_globs_scan_only_relevant_ancestor_branches() {
        let tree = Tree::new();
        std::fs::create_dir_all(tree.0.join("unrelated/private")).unwrap();
        std::fs::write(tree.0.join("unrelated/private/ignored.wat"), "unrelated").unwrap();
        std::fs::create_dir(tree.0.join("project")).unwrap();
        std::fs::write(tree.0.join("project/a.wat"), "target").unwrap();
        let pattern = Watch::parse(
            &json!({"watchers":[{"globPattern":"*.wat"}]}),
            &tree.0.join("project"),
        )
        .unwrap();
        let snapshot = scan_tree(
            std::slice::from_ref(&tree.0),
            &AtomicBool::new(false),
            true,
            &pattern,
        )
        .unwrap();
        assert!(snapshot.contains_key(&tree.0.join("project/a.wat")));
        assert!(!snapshot.contains_key(&tree.0.join("unrelated")));
    }

    #[test]
    fn physical_sources_are_shared_until_the_last_subscription_is_released() {
        let tree = Tree::new();
        let first = source_leases(std::slice::from_ref(&tree.0), |_| {}).unwrap();
        let second = source_leases(std::slice::from_ref(&tree.0), |_| {}).unwrap();
        assert!(Arc::ptr_eq(&first[0]._source, &second[0]._source));
        let source = Arc::downgrade(&first[0]._source);
        drop(first);
        assert!(source.upgrade().is_some());
        drop(second);
        assert!(source.upgrade().is_none());
    }

    #[test]
    fn replacing_a_lease_preserves_changes_during_setup_without_initial_creations() {
        let tree = Tree::new();
        let existing = tree.0.join("existing.wat");
        let deleted = tree.0.join("deleted.wat");
        std::fs::write(&existing, "before").unwrap();
        std::fs::write(&deleted, "delete").unwrap();
        let (ready, receive) = mpsc::channel();
        let (events, changes) = mpsc::channel();
        let old = prepare(
            tree.watches(),
            None,
            move |outcome| {
                let _ = ready.send(outcome);
            },
            move |batch| {
                let _ = events.send(batch);
            },
            |error| panic!("{error}"),
        )
        .unwrap();
        receive
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        old.activate();
        old.checkpoint()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        std::fs::write(&existing, "modified and longer").unwrap();
        std::fs::remove_file(&deleted).unwrap();
        let created = tree.0.join("created.wat");
        std::fs::write(&created, "new").unwrap();
        let newly_watched = tree.0.join("already-present.new");
        std::fs::write(&newly_watched, "no initial notification").unwrap();
        let mut watches = tree.watches();
        watches.extend(
            Watch::parse(&json!({"watchers":[{"globPattern":"**/*.new"}]}), &tree.0).unwrap(),
        );
        let (ready, receive) = mpsc::channel();
        let (events, new_changes) = mpsc::channel();
        let new = prepare(
            watches,
            Some(&old),
            move |outcome| {
                let _ = ready.send(outcome);
            },
            move |batch| {
                let _ = events.send(batch);
            },
            |error| panic!("{error}"),
        )
        .unwrap();
        receive
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        drop(old);
        new.activate();
        let batch = new_changes.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(batch.len(), 3, "{batch:?}");
        for (file, kind) in [(existing, 2), (deleted, 3), (created, 1)] {
            assert!(
                batch.contains(&json!({"uri":uri::file_uri(&file).unwrap(),"type":kind})),
                "{batch:?}"
            );
        }
        assert!(changes.try_recv().is_err(), "the old stream stayed paused");
    }

    #[test]
    fn rescan_and_directory_rename_converge_without_a_rescan_wire_event() {
        let tree = Tree::new();
        let stop = AtomicBool::new(false);
        std::fs::create_dir(tree.0.join("old")).unwrap();
        std::fs::write(tree.0.join("old/a.wat"), "before").unwrap();
        std::fs::write(tree.0.join("modified.wat"), "a").unwrap();
        let before = scan(std::slice::from_ref(&tree.0), &stop).unwrap();
        std::fs::rename(tree.0.join("old"), tree.0.join("new")).unwrap();
        std::fs::write(tree.0.join("modified.wat"), "longer").unwrap();
        let rescan = FileChangeBatch {
            changes: Vec::new(),
            requires_full_rescan: true,
        };
        let after = reconcile(&before, std::slice::from_ref(&tree.0), &rescan, &stop).unwrap();
        let events = changes(&before, &after, &rescan, &tree.watches());
        assert_eq!(events.len(), 3);
        for (file, kind) in [("old/a.wat", 3), ("new/a.wat", 1), ("modified.wat", 2)] {
            assert!(
                events.contains(
                    &json!({"uri":uri::file_uri(&tree.0.join(file)).unwrap(),"type":kind})
                ),
                "{events:?}"
            );
        }
        let rename = FileChangeBatch {
            changes: ["old", "new", "modified.wat"]
                .iter()
                .map(|name| FileChange {
                    path: tree.0.join(name),
                    kind: FileChangeKind::Create,
                })
                .collect(),
            requires_full_rescan: false,
        };
        let incremental =
            reconcile(&before, std::slice::from_ref(&tree.0), &rename, &stop).unwrap();
        assert_eq!(
            changes(&before, &incremental, &rename, &tree.watches()),
            events
        );
    }

    #[test]
    fn delete_recreate_and_overlapping_masks_follow_final_disk_state() {
        let tree = Tree::new();
        let stop = AtomicBool::new(false);
        let file = tree.0.join("a.wat");
        std::fs::write(&file, "a").unwrap();
        let before = scan(std::slice::from_ref(&tree.0), &stop).unwrap();
        let hint = FileChangeBatch {
            changes: vec![FileChange {
                path: file.clone(),
                kind: FileChangeKind::Remove,
            }],
            requires_full_rescan: false,
        };
        let after = reconcile(&before, std::slice::from_ref(&tree.0), &hint, &stop).unwrap();
        let mut watches = tree.watches();
        watches.extend(tree.watches());
        assert_eq!(
            changes(&before, &after, &hint, &watches),
            [json!({"uri":uri::file_uri(&file).unwrap(),"type":2})]
        );
        std::fs::remove_file(&file).unwrap();
        let after = reconcile(&before, std::slice::from_ref(&tree.0), &hint, &stop).unwrap();
        assert_eq!(changes(&before, &after, &hint, &watches)[0]["type"], 3);
        assert!(changes(&Snapshot::new(), &after, &hint, &watches).is_empty());
    }

    #[test]
    fn missing_roots_and_root_budget_fail_explicitly() {
        let tree = Tree::new();
        let watch=Watch::parse(&json!({"watchers":[{"globPattern":{"baseUri":uri::file_uri(&tree.0.join("missing")).unwrap(),"pattern":"**"}}]}),&tree.0).unwrap();
        assert!(roots(&watch).is_err());
        let mut watches = Vec::new();
        for index in 0..17 {
            let path = tree.0.join(index.to_string());
            std::fs::create_dir(&path).unwrap();
            watches.extend(Watch::parse(&json!({"watchers":[{"globPattern":{"baseUri":uri::file_uri(&path).unwrap(),"pattern":"**"}}]}),&tree.0).unwrap());
        }
        assert!(roots(&watches).unwrap_err().contains("budget"));
        assert!(scan(std::slice::from_ref(&tree.0), &AtomicBool::new(true)).is_err());
    }
}
