//! Installs the packs language servers run from, as the catalog describes them.
//!
//! A pack is downloaded from its fixed release, checked against the size and
//! SHA-256 the catalog pins, unpacked into a staging directory under strict rules,
//! and only activated once every server in it has started and completed an LSP
//! `initialize`. Nothing half installed is ever visible: a pack is either under
//! `packs/<id>/<version>-<digest>` or it is not there. Requirements (a runtime such
//! as Node) are installed first. Downloads never run on the UI or parser threads.
//!
//! ```text
//! <user>/lsp-servers/.managed/
//!   archives/<sha256>.tar.gz          verified archives, reused offline
//!   packs/<id>/<version>-<digest>/    complete packs
//!   staging/                          unpacking and smoke tests, removed afterwards
//! ```
use super::lsp::client::{Client, Launch, Timeouts};
use super::lsp_catalog::{Catalog, PackArchive};
use crate::features::language_service::{
    expand::{self, Installed},
    session::{Event, SessionOptions, State},
    uri,
};
use ring::digest::{Context, SHA256};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

/// Largest unpacked pack and file, and most entries, accepted from an archive.
const MAX_UNPACKED: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FILE: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 200_000;
/// A failed pack is not asked for again for this long, unless the user retries.
const RETRY_AFTER: Duration = Duration::from_secs(30);
const WORKERS: usize = 2;

pub(crate) type Download = dyn Fn(&str) -> Result<Box<dyn Read + Send>, String> + Send + Sync;
pub(crate) type Smoke = dyn Fn(&SmokeRequest) -> Result<(), String> + Send + Sync;
type Listener = dyn Fn() + Send + Sync;

#[derive(Clone, Debug, Default)]
pub(crate) struct Cancellation(Arc<AtomicBool>);
impl PartialEq for Cancellation {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Cancellation {
    pub(crate) fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    fn check(&self) -> Result<(), String> {
        if self.cancelled() {
            Err("installation cancelled".into())
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InstallTarget {
    pub pack: String,
    pub generation: u64,
}

/// One server to start before its pack is activated.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SmokeRequest {
    pub server: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub options: Option<Value>,
    pub cwd: PathBuf,
    pub cancellation: Cancellation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PackStatus {
    NotInstalled,
    /// The catalog has no archive of the pack for this platform.
    Unavailable(String),
    Queued,
    Downloading {
        received: u64,
        total: u64,
    },
    Verifying,
    /// Starting the pack's servers once, before it is activated.
    Starting,
    Cancelling,
    Cancelled,
    Installed(PathBuf),
    Failed(String),
}

#[derive(Clone)]
enum Step {
    /// Waiting for a pack this one needs.
    Waiting,
    Downloading {
        received: u64,
        total: u64,
    },
    Verifying,
    Starting,
}

#[derive(Clone)]
struct Ticket {
    run: u64,
    cancellation: Cancellation,
}

enum Job {
    Queued(Ticket),
    Running(Ticket, Step),
    Failed { error: String, retry: Instant },
}

impl Job {
    fn ticket(&self) -> Option<&Ticket> {
        match self {
            Self::Queued(ticket) | Self::Running(ticket, _) => Some(ticket),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Work {
    states: BTreeMap<String, Job>,
    queue: VecDeque<String>,
    running: usize,
    next: u64,
    /// Explicit root requests; dependencies are retained only while needed.
    requests: BTreeMap<String, u64>,
    /// Cancelled roots and the generations still draining before cleanup ends.
    cancelled: BTreeMap<String, BTreeMap<String, u64>>,
}

pub(crate) struct Installer {
    catalog: Arc<Catalog>,
    managed: PathBuf,
    platform: String,
    work: Mutex<Work>,
    revision: AtomicU64,
    listeners: Mutex<Vec<Weak<Listener>>>,
    download: Arc<Download>,
    smoke: Arc<Smoke>,
}

impl Installer {
    pub(crate) fn new(
        catalog: Arc<Catalog>,
        managed: PathBuf,
        platform: String,
        download: Arc<Download>,
        smoke: Arc<Smoke>,
    ) -> Arc<Self> {
        Arc::new(Self {
            catalog,
            managed,
            platform,
            work: Mutex::default(),
            revision: AtomicU64::new(0),
            listeners: Mutex::default(),
            download,
            smoke,
        })
    }

    /// The installer for this machine: the user's data directory, the real network
    /// and real server processes. `None` when there is no user data directory.
    pub(crate) fn production(catalog: Arc<Catalog>) -> Option<Arc<Self>> {
        let managed =
            super::language_packs::user_resource_directory("lsp-servers")?.join(".managed");
        Some(Self::new(
            catalog,
            managed,
            super::lsp_catalog::platform(),
            Arc::new(http_download),
            Arc::new(start_and_initialize),
        ))
    }

    /// Changes whenever any pack's status does.
    pub(crate) fn generation(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    pub(crate) fn subscribe(&self, changed: Arc<Listener>) {
        self.listeners
            .lock()
            .unwrap()
            .push(Arc::downgrade(&changed));
    }

    fn notify(&self) {
        self.revision.fetch_add(1, Ordering::Release);
        let listeners: Vec<_> = {
            let mut listeners = self.listeners.lock().unwrap();
            listeners.retain(|listener| listener.strong_count() > 0);
            listeners.iter().filter_map(Weak::upgrade).collect()
        };
        for listener in listeners {
            listener();
        }
    }

    fn pack_directory(&self, archive: &PackArchive, pack: &str) -> PathBuf {
        self.managed.join("packs").join(pack).join(format!(
            "{}-{}",
            archive.version,
            &archive.sha256[..16]
        ))
    }

    /// The directory of an installed pack: complete, and the version the catalog pins.
    pub(crate) fn installed_dir(&self, pack: &str) -> Option<PathBuf> {
        let archive = self.catalog.archive(pack, &self.platform).ok()?;
        let directory = self.pack_directory(&archive, pack);
        directory.is_dir().then_some(directory)
    }

    pub(crate) fn status(&self, pack: &str) -> PackStatus {
        let work = self.work.lock().unwrap();
        if !matches!(work.states.get(pack), Some(Job::Running(_, _))) {
            if let Some(directory) = self.installed_dir(pack) {
                return PackStatus::Installed(directory);
            }
        }
        if let Some(pending) = work.cancelled.get(pack) {
            return if pending.iter().any(|(id, run)| {
                work.states
                    .get(id)
                    .and_then(Job::ticket)
                    .is_some_and(|ticket| ticket.run == *run)
            }) {
                PackStatus::Cancelling
            } else {
                PackStatus::Cancelled
            };
        }
        if let Some(state) = work.states.get(pack) {
            return match state {
                Job::Queued(_) => PackStatus::Queued,
                Job::Running(ticket, _) if ticket.cancellation.cancelled() => {
                    PackStatus::Cancelling
                }
                Job::Running(_, Step::Waiting) => PackStatus::Queued,
                Job::Running(_, Step::Downloading { received, total }) => PackStatus::Downloading {
                    received: *received,
                    total: *total,
                },
                Job::Running(_, Step::Verifying) => PackStatus::Verifying,
                Job::Running(_, Step::Starting) => PackStatus::Starting,
                Job::Failed { error, .. } => PackStatus::Failed(error.clone()),
            };
        }
        match self.catalog.archive(pack, &self.platform) {
            Err(error) => PackStatus::Unavailable(error),
            Ok(archive) if self.pack_directory(&archive, pack).is_dir() => {
                PackStatus::Installed(self.pack_directory(&archive, pack))
            }
            Ok(_) => PackStatus::NotInstalled,
        }
    }

    /// Packs with anything to report: being installed, or failed.
    pub(crate) fn busy(&self) -> Vec<(String, PackStatus)> {
        let ids: BTreeSet<String> = {
            let work = self.work.lock().unwrap();
            work.states
                .keys()
                .chain(work.cancelled.keys())
                .cloned()
                .collect()
        };
        ids.into_iter()
            .map(|id| {
                let status = self.status(&id);
                (id, status)
            })
            .collect()
    }

    /// The pack and what it needs, requirements first.
    fn plan(&self, pack: &str) -> Result<Vec<String>, String> {
        fn visit(
            installer: &Installer,
            pack: &str,
            seen: &mut BTreeSet<String>,
            order: &mut Vec<String>,
        ) -> Result<(), String> {
            if !seen.insert(pack.to_owned()) {
                return Ok(());
            }
            installer.catalog.archive(pack, &installer.platform)?;
            for requirement in installer.catalog.requires(pack) {
                visit(installer, &requirement, seen, order)?;
            }
            order.push(pack.to_owned());
            Ok(())
        }
        let mut order = Vec::new();
        visit(self, pack, &mut BTreeSet::new(), &mut order)?;
        Ok(order)
    }

    /// Catalog-only size estimate for a prompt; hashing cached archives belongs
    /// to the installer workers, never to the coordinator's status publication.
    pub(crate) fn missing_archives(&self, pack: &str) -> Result<BTreeMap<String, u64>, String> {
        self.plan(pack)?
            .into_iter()
            .filter(|id| self.installed_dir(id).is_none())
            .map(|id| {
                self.catalog
                    .archive(&id, &self.platform)
                    .map(|archive| (id, archive.size))
            })
            .collect()
    }

    /// Asks for `pack` to be installed. Repeated requests are cheap: a pack that is
    /// installed, queued or running is left alone, and a failed one waits out its
    /// cooldown.
    pub(crate) fn request(self: &Arc<Self>, pack: &str) -> Result<(), String> {
        let order = self.plan(pack)?;
        let changed = {
            let mut work = self.work.lock().unwrap();
            if work.cancelled.contains_key(pack) || self.installed_dir(pack).is_some() {
                return Ok(());
            }
            let requested = !work.requests.contains_key(pack);
            let retrying = order.iter().any(|id| matches!(work.states.get(id), Some(Job::Failed { retry, .. }) if Instant::now() >= *retry));
            if requested || retrying {
                work.next += 1;
                let generation = work.next;
                work.requests.insert(pack.to_owned(), generation);
            }
            self.reconcile(&mut work) || requested
        };
        if changed {
            self.wake();
        }
        Ok(())
    }

    /// A manual install also resumes a cancelled request, with a fresh target.
    pub(crate) fn resume(self: &Arc<Self>, pack: &str) -> Result<(), String> {
        let order = self.plan(pack)?;
        {
            let mut work = self.work.lock().unwrap();
            work.cancelled.remove(pack);
            let retrying = order
                .iter()
                .any(|id| matches!(work.states.get(id), Some(Job::Failed { .. })));
            if retrying || !work.requests.contains_key(pack) {
                work.next += 1;
                let generation = work.next;
                work.requests.insert(pack.to_owned(), generation);
            }
            if retrying {
                if let Some(Job::Running(ticket, _)) = work.states.get(pack) {
                    ticket.cancellation.0.store(true, Ordering::Release);
                }
            }
            for id in order {
                if matches!(work.states.get(&id), Some(Job::Failed { .. })) {
                    work.states.remove(&id);
                }
            }
            self.reconcile(&mut work);
        }
        self.wake();
        Ok(())
    }

    pub(crate) fn cancel_target(&self, pack: &str) -> Option<InstallTarget> {
        let work = self.work.lock().unwrap();
        if self.installed_dir(pack).is_some() {
            return None;
        }
        work.requests.get(pack).map(|generation| InstallTarget {
            pack: pack.into(),
            generation: *generation,
        })
    }

    pub(crate) fn cancel(self: &Arc<Self>, target: &InstallTarget) {
        {
            let mut work = self.work.lock().unwrap();
            if work.requests.get(&target.pack) != Some(&target.generation)
                || self.installed_dir(&target.pack).is_some()
            {
                return;
            }
            work.requests.remove(&target.pack);
            let needed = self.needed(&work);
            let mut pending = BTreeMap::new();
            for id in self.plan(&target.pack).unwrap_or_default() {
                if needed.contains(&id) {
                    continue;
                }
                match work.states.get(&id) {
                    Some(Job::Running(ticket, _)) => {
                        ticket.cancellation.0.store(true, Ordering::Release);
                        pending.insert(id, ticket.run);
                    }
                    Some(_) => {
                        work.states.remove(&id);
                    }
                    None => {}
                }
            }
            work.queue.retain(|id| needed.contains(id));
            work.cancelled.insert(target.pack.clone(), pending);
        }
        self.wake();
    }

    fn needed(&self, work: &Work) -> BTreeSet<String> {
        work.requests
            .keys()
            .filter(|id| self.installed_dir(id).is_none())
            .flat_map(|id| self.plan(id).unwrap_or_default())
            .collect()
    }

    fn reconcile(&self, work: &mut Work) -> bool {
        let mut changed = false;
        for id in self.needed(work) {
            if self.installed_dir(&id).is_some() {
                continue;
            }
            match work.states.get(&id) {
                Some(Job::Queued(_) | Job::Running(_, _)) => continue,
                Some(Job::Failed { retry, .. }) if Instant::now() < *retry => continue,
                _ => {}
            }
            work.next += 1;
            let ticket = Ticket {
                run: work.next,
                cancellation: Cancellation::default(),
            };
            work.states.insert(id.clone(), Job::Queued(ticket));
            work.queue.push_back(id);
            changed = true;
        }
        changed
    }

    // Only dispatch dependency-ready jobs. A worker must never block behind a
    // queued dependency after another worker has been cancelled and resumed.
    fn ready(&self, work: &Work, id: &str) -> bool {
        self.catalog.requires(id).iter().all(|requirement| {
            self.installed_dir(requirement).is_some()
                || matches!(work.states.get(requirement), Some(Job::Failed { .. }))
        })
    }

    fn wake(self: &Arc<Self>) {
        let workers = {
            let mut work = self.work.lock().unwrap();
            let ready = work.queue.iter().filter(|id| self.ready(&work, id)).count();
            let workers = ready.min(WORKERS - work.running);
            work.running += workers;
            workers
        };
        for _ in 0..workers {
            let installer = self.clone();
            let spawned = std::thread::Builder::new()
                .name("loom-lsp-install".into())
                .spawn(move || installer.run());
            if spawned.is_err() {
                self.work.lock().unwrap().running -= 1;
            }
        }
        self.notify();
    }

    /// Forgets every failure and asks for those packs again.
    pub(crate) fn retry(self: &Arc<Self>) {
        let roots: Vec<String> = {
            let work = self.work.lock().unwrap();
            work.requests
                .keys()
                .filter(|root| {
                    self.plan(root)
                        .unwrap_or_default()
                        .iter()
                        .any(|id| matches!(work.states.get(id), Some(Job::Failed { .. })))
                })
                .cloned()
                .collect()
        };
        for id in roots {
            let _ = self.resume(&id);
        }
    }

    fn run(self: Arc<Self>) {
        loop {
            let (id, ticket, dependency_error) = {
                let mut work = self.work.lock().unwrap();
                let Some(index) = work.queue.iter().position(|id| self.ready(&work, id)) else {
                    work.running -= 1;
                    return;
                };
                let id = work.queue.remove(index).unwrap();
                let Some(Job::Queued(ticket)) = work.states.remove(&id) else {
                    unreachable!("queued ticket")
                };
                work.states
                    .insert(id.clone(), Job::Running(ticket.clone(), Step::Waiting));
                let dependency_error = self.catalog.requires(&id).iter().find_map(|requirement| {
                    match work.states.get(requirement) {
                        Some(Job::Failed { error, .. })
                            if self.installed_dir(requirement).is_none() =>
                        {
                            Some(format!("{id} needs {requirement}: {error}"))
                        }
                        _ => None,
                    }
                });
                (id, ticket, dependency_error)
            };
            self.notify();
            let result = match dependency_error {
                Some(error) => Err(error),
                None => self.install(&id, &ticket),
            };
            {
                let mut work = self.work.lock().unwrap();
                match result {
                    _ if ticket.cancellation.cancelled() => {
                        work.states.remove(&id);
                    }
                    Ok(()) => {
                        work.states.remove(&id);
                    }
                    Err(error) => {
                        work.states.insert(
                            id.clone(),
                            Job::Failed {
                                error,
                                retry: Instant::now() + RETRY_AFTER,
                            },
                        );
                    }
                }
                self.reconcile(&mut work);
            }
            self.wake();
        }
    }

    fn step(&self, pack: &str, ticket: &Ticket, step: Step) {
        if let Some(state @ Job::Running(_, _)) = self.work.lock().unwrap().states.get_mut(pack) {
            if state
                .ticket()
                .is_some_and(|current| current.run == ticket.run)
                && !ticket.cancellation.cancelled()
            {
                *state = Job::Running(ticket.clone(), step);
            }
        }
        self.notify();
    }

    fn install(&self, pack: &str, ticket: &Ticket) -> Result<(), String> {
        let cancellation = &ticket.cancellation;
        cancellation.check()?;
        let archive = self.catalog.archive(pack, &self.platform)?;
        let destination = self.pack_directory(&archive, pack);
        if destination.is_dir() {
            return Ok(());
        }
        let file = self.fetch(pack, &archive, ticket)?;
        let staging = Temporary::new(&self.managed.join("staging"))?;
        let unpacked = unpack(&file, staging.path(), pack, cancellation)?;
        self.step(pack, ticket, Step::Starting);
        for launch in self.catalog.launches(pack) {
            cancellation.check()?;
            let runtime = |id: &str| self.installed_dir(id);
            let installed = Installed {
                pack: &unpacked,
                runtime: &runtime,
            };
            let command = expand::expand(&launch.command, &installed)?;
            let request = SmokeRequest {
                server: launch.server.clone(),
                program: expand::program(&command, cfg!(windows)),
                args: launch
                    .args
                    .iter()
                    .map(|arg| expand::expand(arg, &installed))
                    .collect::<Result<_, _>>()?,
                options: launch
                    .options
                    .as_ref()
                    .map(|options| expand::expand_json(options, &installed))
                    .transpose()?,
                cwd: unpacked.clone(),
                cancellation: cancellation.clone(),
            };
            (self.smoke)(&request)
                .map_err(|error| format!("{} did not start: {error}", launch.server))?;
        }
        fs::create_dir_all(destination.parent().expect("a pack directory has a parent"))
            .map_err(|e| e.to_string())?;
        // Cancellation and activation share this fence. Once cancellation has
        // been acknowledged, this generation cannot expose an installed pack.
        let work = self.work.lock().unwrap();
        cancellation.check()?;
        if work
            .states
            .get(pack)
            .and_then(Job::ticket)
            .map(|current| current.run)
            != Some(ticket.run)
        {
            return Err("installation superseded".into());
        }
        match fs::rename(&unpacked, &destination) {
            Ok(()) => {}
            // Another process installed the same pack first.
            Err(_) if destination.is_dir() => {}
            Err(error) => return Err(format!("activate {pack}: {error}")),
        }
        drop(work);
        prune(
            destination.parent().expect("a pack directory has a parent"),
            &destination,
        );
        Ok(())
    }

    /// The verified archive: from the cache, or downloaded and verified.
    fn fetch(&self, pack: &str, archive: &PackArchive, ticket: &Ticket) -> Result<PathBuf, String> {
        let cancellation = &ticket.cancellation;
        cancellation.check()?;
        let cache = self.managed.join("archives");
        fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
        let file = cache.join(format!("{}.tar.gz", archive.sha256));
        if verified(&file, archive, cancellation)? {
            return Ok(file);
        }
        self.step(
            pack,
            ticket,
            Step::Downloading {
                received: 0,
                total: archive.size,
            },
        );
        cancellation.check()?;
        let mut reader = (self.download)(&archive.url)?;
        cancellation.check()?;
        let partial = Temporary::file(&cache)?;
        let mut output = fs::File::create(partial.path()).map_err(|e| e.to_string())?;
        let mut context = Context::new(&SHA256);
        let mut buffer = vec![0u8; 64 * 1024];
        let (mut received, mut reported) = (0u64, 0u64);
        loop {
            cancellation.check()?;
            let read = reader
                .read(&mut buffer)
                .map_err(|e| format!("download: {e}"))?;
            cancellation.check()?;
            if read == 0 {
                break;
            }
            received += read as u64;
            if received > archive.size {
                return Err("the download is larger than the catalog says".into());
            }
            context.update(&buffer[..read]);
            output
                .write_all(&buffer[..read])
                .map_err(|e| e.to_string())?;
            if received - reported > archive.size / 100 {
                reported = received;
                self.step(
                    pack,
                    ticket,
                    Step::Downloading {
                        received,
                        total: archive.size,
                    },
                );
            }
        }
        output.flush().map_err(|e| e.to_string())?;
        drop(output);
        self.step(pack, ticket, Step::Verifying);
        cancellation.check()?;
        if received != archive.size {
            return Err(format!(
                "the download is {received} bytes, the catalog says {}",
                archive.size
            ));
        }
        if hex(context.finish().as_ref()) != archive.sha256 {
            return Err("the download does not match its SHA-256".into());
        }
        fs::rename(partial.path(), &file).map_err(|e| format!("cache the archive: {e}"))?;
        Ok(file)
    }
}

/// Whether `file` is the archive the catalog pins.
fn verified(
    file: &Path,
    archive: &PackArchive,
    cancellation: &Cancellation,
) -> Result<bool, String> {
    cancellation.check()?;
    let Ok(mut input) = fs::File::open(file) else {
        return Ok(false);
    };
    if input.metadata().map(|m| m.len()).ok() != Some(archive.size) {
        return Ok(false);
    }
    let mut context = Context::new(&SHA256);
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        cancellation.check()?;
        match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => context.update(&buffer[..read]),
            Err(_) => return Ok(false),
        }
    }
    cancellation.check()?;
    Ok(hex(context.finish().as_ref()) == archive.sha256)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Removes older versions of a pack, keeping the one just installed and the newest other.
fn prune(versions: &Path, current: &Path) {
    let Ok(entries) = fs::read_dir(versions) else {
        return;
    };
    let mut others: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path() != current && entry.path().is_dir())
        .map(|entry| {
            let modified = entry.metadata().and_then(|m| m.modified()).ok();
            (modified, entry.path())
        })
        .collect();
    others.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (_, path) in others.into_iter().skip(1) {
        let _ = fs::remove_dir_all(path);
    }
}

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

/// A directory or file that disappears with it unless it was renamed away.
struct Temporary(PathBuf);
impl Temporary {
    fn unique(root: &Path) -> PathBuf {
        root.join(format!(
            ".install-{}-{}",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }
    fn new(root: &Path) -> Result<Self, String> {
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let path = Self::unique(root);
        fs::create_dir(&path).map_err(|e| e.to_string())?;
        Ok(Self(path))
    }
    fn file(root: &Path) -> Result<Self, String> {
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        Ok(Self(Self::unique(root)))
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        if self.0.is_dir() {
            let _ = fs::remove_dir_all(&self.0);
        } else {
            let _ = fs::remove_file(&self.0);
        }
    }
}

/// Whether a path in an archive is safe to create: relative, inside the pack,
/// with names every platform can hold.
fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains(['\\', ':', '\0'])
        && path.split('/').all(|part| {
            !part.is_empty() && part != "." && part != ".." && !part.ends_with(['.', ' '])
        })
}

/// Unpacks a pack archive into `root`, which must hold exactly the directory `id`.
/// Only regular files and directories are accepted, nothing may repeat, and size
/// limits apply; anything else rejects the whole archive.
fn unpack(
    archive: &Path,
    root: &Path,
    id: &str,
    cancellation: &Cancellation,
) -> Result<PathBuf, String> {
    cancellation.check()?;
    let input = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(input));
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    for entry in tar.entries().map_err(|e| e.to_string())? {
        cancellation.check()?;
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry
            .path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        let path = path.trim_end_matches('/').to_owned();
        if !safe_path(&path) {
            return Err(format!("the archive has an unsafe path: {path}"));
        }
        if path != id && !path.starts_with(&format!("{id}/")) {
            return Err("the archive has an unexpected root directory".into());
        }
        if !seen.insert(path.to_ascii_lowercase()) || seen.len() > MAX_ENTRIES {
            return Err("the archive repeats a path or has too many entries".into());
        }
        let output = root.join(&path);
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            fs::create_dir_all(&output).map_err(|e| e.to_string())?;
            continue;
        }
        if !kind.is_file() {
            return Err("the archive may hold only files and directories".into());
        }
        let size = entry.size();
        total = total.checked_add(size).ok_or("the archive is too large")?;
        if size > MAX_FILE || total > MAX_UNPACKED {
            return Err("the archive expands beyond the allowed size".into());
        }
        fs::create_dir_all(output.parent().expect("an entry has a parent"))
            .map_err(|e| e.to_string())?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .map_err(|e| e.to_string())?;
        let mut copied = 0;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            cancellation.check()?;
            let read = entry.read(&mut buffer).map_err(|e| e.to_string())?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read]).map_err(|e| e.to_string())?;
            copied += read as u64;
        }
        if copied != size {
            return Err("the archive is truncated".into());
        }
        file.flush().map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = entry.header().mode().unwrap_or(0o644);
            fs::set_permissions(&output, fs::Permissions::from_mode(mode & 0o777))
                .map_err(|e| e.to_string())?;
        }
    }
    let unpacked = root.join(id);
    if !unpacked.is_dir() {
        return Err("the archive has no pack directory".into());
    }
    Ok(unpacked)
}

fn http_download(url: &str) -> Result<Box<dyn Read + Send>, String> {
    let response = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(30))
        .timeout_read(Duration::from_secs(60))
        .build()
        .get(url)
        .call()
        .map_err(|e| e.to_string())?;
    Ok(Box::new(response.into_reader()))
}

/// Starts a server and completes `initialize` with it, then stops it.
fn start_and_initialize(request: &SmokeRequest) -> Result<(), String> {
    request.cancellation.check()?;
    let root_uri = uri::file_uri(&request.cwd).ok_or("the pack directory has no usable path")?;
    let launch = Launch {
        env: Default::default(),
        command: request.program.to_string_lossy().into_owned(),
        args: request.args.clone(),
        cwd: request.cwd.clone(),
        session: SessionOptions {
            root_uri,
            root_name: String::new(),
            process_id: Some(std::process::id()),
            client_version: env!("CARGO_PKG_VERSION").to_owned(),
            initialization_options: request.options.clone(),
            configuration: BTreeMap::new(),
        },
        timeouts: Timeouts::default(),
        max_message: 64 * 1024 * 1024,
    };
    let (events, inbox) = mpsc::channel();
    let client = Client::spawn(launch, move |event| {
        let _ = events.send(event);
    })
    .map_err(|e| format!("{}: {e}", request.program.display()))?;
    let deadline = Instant::now() + Duration::from_secs(40);
    let outcome = loop {
        if let Err(error) = request.cancellation.check() {
            break Err(error);
        }
        if Instant::now() >= deadline {
            break Err("it did not answer initialize in time".to_owned());
        }
        match inbox.recv_timeout(Duration::from_millis(50)) {
            Ok(Event::Ready(_)) => break Ok(()),
            Ok(Event::StateChanged(State::Failed(reason))) => break Err(reason),
            Ok(Event::StateChanged(State::Stopped)) => break Err("the server exited".to_owned()),
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break Err("the server exited".to_owned()),
        }
    };
    let tail = client.stderr_tail();
    client.stop();
    outcome.map_err(|reason| match tail.last() {
        Some(line) => format!("{reason} ({line})"),
        None => reason,
    })
}

#[cfg(test)]
#[path = "lsp_install_tests.rs"]
mod tests;
