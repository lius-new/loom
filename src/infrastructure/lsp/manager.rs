//! Runs the language servers for the open documents.
//!
//! One coordinator thread owns every server instance. The application hands it
//! snapshots of the open documents (cheap: text is shared) and gets back a
//! status list; nothing here runs on the UI thread, and a server that is slow,
//! missing or crashing never delays an edit.
//!
//! The coordinator
//! - decides which instance each document belongs to (see `plan`), reading the
//!   disk for project markers only when a document is new or the settings changed;
//! - starts instances, keeps their documents in step, and stops them when idle;
//! - restarts a crashed server after 1, 2 and 4 seconds, then reports it failed
//!   until the user restarts it;
//! - reports a program that cannot be started as unavailable, once, quietly.
use super::client::{Client, Launch, Timeouts};
use crate::domain::language::{Registry, ResolvedLanguage};
use crate::features::language_service::{
    capabilities::ServerCapabilities,
    diagnostics::{self, Diagnostic},
    plan::{self, Assignment, InstanceKey},
    semantic,
    session::{Event, SessionOptions, State},
    sync::{DesiredDocument, DocumentSync, Notification, Stamp},
    uri,
};
use crate::features::settings::servers::LanguageServerSetting;
use crate::features::syntax::TokenSpan;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Largest message accepted from a server.
const MAX_MESSAGE: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct Options {
    pub timeouts: Timeouts,
    /// How long an instance with no documents keeps running.
    pub idle_stop: Duration,
    /// Waits before each automatic restart; once used up the instance is failed.
    pub backoff: Vec<Duration>,
    /// A server that stayed ready this long starts counting failures afresh.
    pub stable_after: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            timeouts: Timeouts::default(),
            idle_stop: Duration::from_secs(60),
            backoff: vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
            ],
            stable_after: Duration::from_secs(60),
        }
    }
}

/// An open document, as the application hands it over.
#[derive(Clone, Debug)]
pub(crate) struct SnapshotDocument {
    pub path: PathBuf,
    pub language: ResolvedLanguage,
    pub stamp: Stamp,
    pub text: Arc<String>,
}

/// Everything the coordinator needs to know about the application. Each snapshot
/// replaces the previous one.
#[derive(Clone, Debug, Default)]
pub(crate) struct Snapshot {
    pub documents: Vec<SnapshotDocument>,
    pub workspace_roots: Vec<PathBuf>,
    pub servers: Vec<LanguageServerSetting>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    /// The process is starting or answering `initialize`.
    Starting,
    Ready,
    /// It stopped unexpectedly and will be started again.
    Restarting {
        attempt: usize,
        reason: String,
    },
    /// It stopped repeatedly; it stays down until restarted by the user.
    Failed(String),
    /// The program could not be started at all.
    Unavailable(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InstanceInfo {
    pub server: String,
    pub root: PathBuf,
    pub status: Status,
    pub documents: usize,
}

/// How long after the last edit a document's semantic tokens are asked for.
const SEMANTIC_DEBOUNCE: Duration = Duration::from_millis(75);

/// What the servers said about one document, each part with the editor state it
/// describes. A part is only meant to be shown for exactly that state.
#[derive(Default)]
struct Entry {
    diagnostics: Option<(Stamp, Arc<[Diagnostic]>)>,
    semantic: Option<(Stamp, Arc<[TokenSpan]>)>,
}

type Store = Arc<Mutex<HashMap<String, Entry>>>;

/// The server results that apply to a document in a given state.
#[derive(Clone, Debug, Default)]
pub(crate) struct DocumentResults {
    pub diagnostics: Option<Arc<[Diagnostic]>>,
    pub semantic: Option<Arc<[TokenSpan]>>,
}

enum Msg {
    Snapshot(Box<Snapshot>),
    Saved {
        path: PathBuf,
        text: Arc<String>,
    },
    Restart,
    Event {
        instance: u64,
        run: u64,
        event: Event,
    },
    Stop,
}

pub(crate) struct Manager {
    tx: Sender<Msg>,
    infos: Arc<Mutex<Vec<InstanceInfo>>>,
    results: Store,
    thread: Option<JoinHandle<()>>,
}

impl Manager {
    /// `on_change` runs on the coordinator thread whenever the status list
    /// changes; it should only request a repaint.
    pub fn start(
        registry: &'static Registry,
        options: Options,
        on_change: impl Fn() + Send + 'static,
    ) -> Self {
        let (tx, inbox) = mpsc::channel();
        let infos = Arc::new(Mutex::new(Vec::new()));
        let results: Store = Arc::default();
        let coordinator = Coordinator {
            registry,
            options,
            tx: tx.clone(),
            inbox,
            infos: infos.clone(),
            results: results.clone(),
            results_changed: false,
            on_change: Box::new(on_change),
            instances: Vec::new(),
            next_id: 1,
            snapshot: Snapshot::default(),
            assignments: HashMap::new(),
            published: Vec::new(),
        };
        let thread = thread::Builder::new()
            .name("loom-lsp-manager".into())
            .spawn(move || coordinator.run())
            .expect("the coordinator thread starts");
        Self {
            tx,
            infos,
            results,
            thread: Some(thread),
        }
    }

    pub fn update(&self, snapshot: Snapshot) {
        let _ = self.tx.send(Msg::Snapshot(Box::new(snapshot)));
    }

    /// A document was written to disk.
    pub fn saved(&self, path: PathBuf, text: Arc<String>) {
        let _ = self.tx.send(Msg::Saved { path, text });
    }

    /// Starts again every server that is down, and restarts the running ones.
    pub fn restart(&self) {
        let _ = self.tx.send(Msg::Restart);
    }

    pub fn infos(&self) -> Vec<InstanceInfo> {
        self.infos.lock().unwrap().clone()
    }

    /// What the servers know about `path` in the editor state `stamp`. Results
    /// computed for any other state are not returned.
    pub fn results(&self, path: &Path, stamp: Stamp) -> DocumentResults {
        let Some(uri) = uri::file_uri(path) else {
            return DocumentResults::default();
        };
        let store = self.results.lock().unwrap();
        let Some(entry) = store.get(&uri::key(&uri)) else {
            return DocumentResults::default();
        };
        DocumentResults {
            diagnostics: entry
                .diagnostics
                .as_ref()
                .filter(|(at, _)| *at == stamp)
                .map(|(_, v)| v.clone()),
            semantic: entry
                .semantic
                .as_ref()
                .filter(|(at, _)| *at == stamp)
                .map(|(_, v)| v.clone()),
        }
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Running {
    client: Client,
    /// Present once `initialize` has been answered.
    ready: Option<Box<Ready>>,
}

struct Ready {
    sync: DocumentSync,
    since: Instant,
    capabilities: ServerCapabilities,
    semantic: SemanticState,
}

/// When to ask for a document's semantic tokens and which answers still count.
#[derive(Default)]
struct SemanticState {
    /// Raised when the server says its tokens changed; answers asked for before
    /// that no longer count.
    epoch: u64,
    /// The state of each document (by URI key) last noticed.
    noted: HashMap<String, Stamp>,
    /// Documents to ask about, and when.
    due: HashMap<String, Instant>,
    /// Requests sent: tag to (document key, state asked about, epoch).
    in_flight: HashMap<u64, (String, Stamp, u64)>,
    next_tag: u64,
}

impl Ready {
    fn wants_semantic_tokens(&self) -> bool {
        self.capabilities
            .semantic_tokens
            .as_ref()
            .is_some_and(|tokens| tokens.full)
    }
}

enum Phase {
    Running(Running),
    Waiting { retry_at: Instant, reason: String },
    Failed(String),
    Unavailable(String),
}

struct Instance {
    id: u64,
    run: u64,
    key: InstanceKey,
    phase: Phase,
    failures: usize,
    documents: Vec<DesiredDocument>,
    idle_since: Option<Instant>,
}

struct Coordinator {
    registry: &'static Registry,
    options: Options,
    tx: Sender<Msg>,
    inbox: Receiver<Msg>,
    infos: Arc<Mutex<Vec<InstanceInfo>>>,
    results: Store,
    /// Results were stored since the last announcement.
    results_changed: bool,
    on_change: Box<dyn Fn() + Send>,
    instances: Vec<Instance>,
    next_id: u64,
    snapshot: Snapshot,
    /// Where each document went last time. Text-only edits reuse it, so typing
    /// never touches the disk to look for project markers.
    assignments: HashMap<PathBuf, (ResolvedLanguage, Option<Assignment>)>,
    published: Vec<InstanceInfo>,
}

fn has_entry(dir: &Path, name: &str) -> bool {
    dir.join(name).exists()
}

impl Coordinator {
    fn run(mut self) {
        loop {
            let message = match self.next_deadline() {
                Some(deadline) => match self
                    .inbox
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                {
                    Ok(message) => Some(message),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                },
                None => match self.inbox.recv() {
                    Ok(message) => Some(message),
                    Err(_) => break,
                },
            };
            if let Some(message) = message
                && !self.handle(message)
            {
                break;
            }
            self.timers();
            self.publish();
        }
        self.shutdown();
    }

    /// Returns false to stop.
    fn handle(&mut self, message: Msg) -> bool {
        match message {
            Msg::Stop => return false,
            Msg::Snapshot(snapshot) => {
                // Only the newest snapshot matters.
                let mut latest = *snapshot;
                let mut pending = Vec::new();
                while let Ok(next) = self.inbox.try_recv() {
                    match next {
                        Msg::Snapshot(newer) => latest = *newer,
                        Msg::Stop => return false,
                        other => pending.push(other),
                    }
                }
                self.apply_snapshot(latest);
                for message in pending {
                    if !self.handle(message) {
                        return false;
                    }
                }
            }
            Msg::Saved { path, text } => self.saved(&path, &text),
            Msg::Restart => self.restart(),
            Msg::Event {
                instance,
                run,
                event,
            } => self.event(instance, run, event),
        }
        true
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.instances
            .iter()
            .filter_map(|instance| {
                let idle = instance
                    .idle_since
                    .map(|since| since + self.options.idle_stop);
                let retry = match &instance.phase {
                    Phase::Waiting { retry_at, .. } => Some(*retry_at),
                    _ => None,
                };
                let semantic = match &instance.phase {
                    Phase::Running(Running {
                        ready: Some(ready), ..
                    }) => ready.semantic.due.values().min().copied(),
                    _ => None,
                };
                [idle, retry, semantic].into_iter().flatten().min()
            })
            .min()
    }

    fn timers(&mut self) {
        let now = Instant::now();
        self.request_semantic_tokens(now);
        let idle_stop = self.options.idle_stop;
        let mut stopping = Vec::new();
        self.instances.retain_mut(|instance| {
            if instance.idle_since.is_some_and(|s| now >= s + idle_stop) {
                stopping.push(std::mem::replace(
                    &mut instance.phase,
                    Phase::Failed(String::new()),
                ));
                return false;
            }
            true
        });
        for phase in stopping {
            stop_in_background(phase);
        }
        let due: Vec<usize> = self
            .instances
            .iter()
            .enumerate()
            .filter(
                |(_, i)| matches!(&i.phase, Phase::Waiting { retry_at, .. } if now >= *retry_at),
            )
            .map(|(at, _)| at)
            .collect();
        for at in due {
            self.start(at);
        }
    }

    fn assign(&mut self, document: &SnapshotDocument) -> Option<Assignment> {
        if let Some((language, assignment)) = self.assignments.get(&document.path)
            && *language == document.language
        {
            return assignment.clone();
        }
        let assignment = plan::assign(
            self.registry,
            document.language,
            &document.path,
            &self.snapshot.workspace_roots,
            &self.snapshot.servers,
            has_entry,
        );
        self.assignments.insert(
            document.path.clone(),
            (document.language, assignment.clone()),
        );
        assignment
    }

    fn apply_snapshot(&mut self, snapshot: Snapshot) {
        // New roots or settings may move every document, so look again.
        if snapshot.workspace_roots != self.snapshot.workspace_roots
            || snapshot.servers != self.snapshot.servers
        {
            self.assignments.clear();
        }
        let open: std::collections::HashSet<&PathBuf> =
            snapshot.documents.iter().map(|d| &d.path).collect();
        self.assignments.retain(|path, _| open.contains(path));
        let documents = snapshot.documents.clone();
        self.snapshot = snapshot;

        let mut wanted: Vec<(InstanceKey, Vec<DesiredDocument>)> = Vec::new();
        for document in &documents {
            let Some(assignment) = self.assign(document) else {
                continue;
            };
            let Some(uri) = uri::file_uri(&document.path) else {
                continue;
            };
            let desired = DesiredDocument {
                uri,
                language_id: assignment.language_id,
                stamp: document.stamp,
                text: document.text.clone(),
            };
            match wanted.iter_mut().find(|(key, _)| *key == assignment.key) {
                Some((_, list)) => list.push(desired),
                None => wanted.push((assignment.key, vec![desired])),
            }
        }

        let now = Instant::now();
        for instance in &mut self.instances {
            match wanted.iter().position(|(key, _)| *key == instance.key) {
                Some(at) => {
                    instance.documents = std::mem::take(&mut wanted[at].1);
                    instance.idle_since = None;
                }
                None => {
                    instance.documents.clear();
                    instance.idle_since.get_or_insert(now);
                }
            }
        }
        let mut created = Vec::new();
        for (key, documents) in wanted {
            if self.instances.iter().any(|i| i.key == key) {
                continue;
            }
            let id = self.next_id;
            self.next_id += 1;
            self.instances.push(Instance {
                id,
                run: 0,
                key,
                phase: Phase::Failed(String::new()),
                failures: 0,
                documents,
                idle_since: None,
            });
            created.push(self.instances.len() - 1);
        }
        for at in created {
            self.start(at);
        }
        for at in 0..self.instances.len() {
            self.sync(at);
        }
    }

    fn start(&mut self, at: usize) {
        let registry = self.registry;
        let timeouts = self.options.timeouts.clone();
        let tx = self.tx.clone();
        let instance = &mut self.instances[at];
        instance.run += 1;
        let (id, run) = (instance.id, instance.run);
        let key = &instance.key;
        let Some(root_uri) = uri::file_uri(&key.root) else {
            instance.phase = Phase::Unavailable("the project folder has no usable path".into());
            return;
        };
        let options = key
            .launch
            .initialization_options
            .as_deref()
            .and_then(|text| serde_json::from_str::<Value>(text).ok());
        let launch = Launch {
            command: key.launch.command.clone(),
            args: key.launch.args.clone(),
            cwd: key.root.clone(),
            session: SessionOptions {
                root_uri,
                root_name: key
                    .root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                process_id: Some(std::process::id()),
                client_version: env!("CARGO_PKG_VERSION").to_owned(),
                initialization_options: options,
                configuration: BTreeMap::new(),
            },
            timeouts,
            max_message: MAX_MESSAGE,
        };
        let name = registry.server(key.definition).display_name.to_string();
        let command = key.launch.command.clone();
        instance.phase = match Client::spawn(launch, move |event| {
            let _ = tx.send(Msg::Event {
                instance: id,
                run,
                event,
            });
        }) {
            Ok(client) => Phase::Running(Running {
                client,
                ready: None,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Phase::Unavailable(format!("{name}: `{command}` was not found"))
            }
            Err(error) => Phase::Unavailable(format!("{name}: cannot start `{command}`: {error}")),
        };
    }

    fn event(&mut self, instance_id: u64, run: u64, event: Event) {
        let Some(at) = self
            .instances
            .iter()
            .position(|i| i.id == instance_id && i.run == run)
        else {
            return; // from a run that has been replaced
        };
        match event {
            Event::Ready(capabilities) => {
                if let Phase::Running(running) = &mut self.instances[at].phase {
                    running.ready = Some(Box::new(Ready {
                        sync: DocumentSync::new(capabilities.text_sync),
                        since: Instant::now(),
                        capabilities,
                        semantic: SemanticState::default(),
                    }));
                }
                self.sync(at);
            }
            Event::Notification { method, params }
                if method == "textDocument/publishDiagnostics" =>
            {
                self.diagnostics(at, &params);
            }
            Event::SemanticTokensRefresh => {
                let instance = &mut self.instances[at];
                if let Phase::Running(Running {
                    ready: Some(ready), ..
                }) = &mut instance.phase
                {
                    ready.semantic.epoch += 1;
                    let now = Instant::now();
                    for document in &instance.documents {
                        ready.semantic.due.insert(uri::key(&document.uri), now);
                    }
                }
            }
            Event::Response { tag, outcome, .. } => self.semantic_response(at, tag, outcome),
            Event::StateChanged(State::Failed(reason)) => self.failed(at, reason),
            Event::StateChanged(State::Stopped) => {
                // Nobody asked it to stop while it was running here.
                self.failed(at, "the server exited".to_owned());
            }
            _ => {}
        }
    }

    fn failed(&mut self, at: usize, reason: String) {
        let stable_after = self.options.stable_after;
        let backoff = self.options.backoff.clone();
        let instance = &mut self.instances[at];
        {
            let mut store = self.results.lock().unwrap();
            for document in &instance.documents {
                store.remove(&uri::key(&document.uri));
            }
            self.results_changed = true;
        }
        let stable = matches!(
            &instance.phase,
            Phase::Running(Running { ready: Some(ready), .. }) if ready.since.elapsed() >= stable_after
        );
        // Dropping the client reaps the process; it has already ended.
        instance.phase = Phase::Failed(String::new());
        if stable {
            instance.failures = 0;
        }
        instance.failures += 1;
        instance.phase = match backoff.get(instance.failures - 1) {
            Some(wait) => Phase::Waiting {
                retry_at: Instant::now() + *wait,
                reason,
            },
            None => Phase::Failed(reason),
        };
    }

    /// Sends the server whatever it is missing about the documents.
    fn sync(&mut self, at: usize) {
        let instance = &mut self.instances[at];
        let Phase::Running(Running {
            client,
            ready: Some(ready),
        }) = &mut instance.phase
        else {
            return;
        };
        for Notification { method, params } in ready.sync.reconcile(&instance.documents) {
            client.notify(method, params);
        }
        if ready.wants_semantic_tokens() {
            let due = Instant::now() + SEMANTIC_DEBOUNCE;
            for document in &instance.documents {
                let key = uri::key(&document.uri);
                if ready.semantic.noted.get(&key) != Some(&document.stamp) {
                    ready.semantic.noted.insert(key.clone(), document.stamp);
                    ready.semantic.due.insert(key, due);
                }
            }
        }
    }

    /// Stores a server's diagnostics when they describe the document as it is now.
    fn diagnostics(&mut self, at: usize, params: &Value) {
        let Some(published) = diagnostics::parse(params) else {
            return;
        };
        let instance = &self.instances[at];
        let Phase::Running(Running {
            ready: Some(ready), ..
        }) = &instance.phase
        else {
            return;
        };
        let key = uri::key(&published.uri);
        let Some(document) = instance.documents.iter().find(|d| uri::key(&d.uri) == key) else {
            return;
        };
        // With a version the server says which text it looked at; without one we
        // can only assume it is the latest, which the next edit invalidates.
        let describes_now = match published.version {
            Some(version) => {
                i32::try_from(version)
                    .ok()
                    .and_then(|version| ready.sync.stamp_of(&document.uri, version))
                    == Some(document.stamp)
            }
            None => true,
        };
        if !describes_now {
            return;
        }
        let resolved = published.resolve(&document.text, ready.capabilities.position_encoding);
        let stamp = document.stamp;
        let mut store = self.results.lock().unwrap();
        store.entry(key).or_default().diagnostics = Some((stamp, resolved.into()));
        self.results_changed = true;
    }

    fn semantic_response(
        &mut self,
        at: usize,
        tag: u64,
        outcome: Result<Value, crate::features::language_service::message::ResponseError>,
    ) {
        let instance = &mut self.instances[at];
        let Phase::Running(Running {
            ready: Some(ready), ..
        }) = &mut instance.phase
        else {
            return;
        };
        let Some((key, stamp, epoch)) = ready.semantic.in_flight.remove(&tag) else {
            return;
        };
        let document = instance.documents.iter().find(|d| uri::key(&d.uri) == key);
        let current = document.is_some_and(|d| d.stamp == stamp) && epoch == ready.semantic.epoch;
        let now = Instant::now();
        match outcome {
            Ok(_) if !current => {
                if document.is_some() {
                    ready.semantic.due.insert(key, now);
                }
            }
            Ok(value) => {
                let Some(document) = document else { return };
                let legend = ready
                    .capabilities
                    .semantic_tokens
                    .as_ref()
                    .map(|tokens| tokens.legend.clone())
                    .unwrap_or_default();
                // A null answer clears the layer, like an empty one.
                let decoded = if value.is_null() {
                    Ok(Vec::new())
                } else {
                    semantic::decode(
                        value.get("data").unwrap_or(&Value::Null),
                        &legend,
                        &document.text,
                        ready.capabilities.position_encoding,
                    )
                };
                if let Ok(spans) = decoded {
                    let mut store = self.results.lock().unwrap();
                    store.entry(key).or_default().semantic = Some((stamp, spans.into()));
                    self.results_changed = true;
                }
            }
            Err(error) if crate::features::language_service::session::is_stale(&error) => {
                if document.is_some() {
                    ready.semantic.due.insert(key, now + SEMANTIC_DEBOUNCE);
                }
            }
            Err(_) => {}
        }
    }

    /// Asks for the semantic tokens of documents whose wait is over.
    fn request_semantic_tokens(&mut self, now: Instant) {
        for instance in &mut self.instances {
            let Phase::Running(Running {
                client,
                ready: Some(ready),
            }) = &mut instance.phase
            else {
                continue;
            };
            let due: Vec<String> = ready
                .semantic
                .due
                .iter()
                .filter(|(_, when)| **when <= now)
                .map(|(key, _)| key.clone())
                .collect();
            for key in due {
                // One request per document at a time.
                if ready.semantic.in_flight.values().any(|(k, _, _)| *k == key) {
                    continue;
                }
                ready.semantic.due.remove(&key);
                let Some(document) = instance.documents.iter().find(|d| uri::key(&d.uri) == key)
                else {
                    continue;
                };
                ready.semantic.next_tag += 1;
                let tag = ready.semantic.next_tag;
                let params = json!({ "textDocument": { "uri": document.uri } });
                if client.request("textDocument/semanticTokens/full", params, tag) {
                    ready
                        .semantic
                        .in_flight
                        .insert(tag, (key, document.stamp, ready.semantic.epoch));
                }
            }
        }
    }

    fn saved(&mut self, path: &Path, text: &Arc<String>) {
        let Some(uri) = uri::file_uri(path) else {
            return;
        };
        for instance in &mut self.instances {
            if let Phase::Running(Running {
                client,
                ready: Some(ready),
            }) = &mut instance.phase
                && let Some(Notification { method, params }) = ready.sync.saved(&uri, text)
            {
                client.notify(method, params);
            }
        }
    }

    fn restart(&mut self) {
        // An instance with no documents has nothing to restart for; it is simply
        // dropped and comes back when a document needs it.
        let mut stopping = Vec::new();
        self.instances.retain_mut(|instance| {
            stopping.push(std::mem::replace(
                &mut instance.phase,
                Phase::Failed(String::new()),
            ));
            instance.failures = 0;
            !instance.documents.is_empty()
        });
        for phase in stopping {
            stop_in_background(phase);
        }
        for at in 0..self.instances.len() {
            self.start(at);
            self.sync(at);
        }
    }

    fn publish(&mut self) {
        let infos: Vec<InstanceInfo> = self
            .instances
            .iter()
            .map(|instance| InstanceInfo {
                server: self
                    .registry
                    .server(instance.key.definition)
                    .display_name
                    .to_string(),
                root: instance.key.root.clone(),
                documents: instance.documents.len(),
                status: match &instance.phase {
                    Phase::Running(Running { ready: Some(_), .. }) => Status::Ready,
                    Phase::Running(_) => Status::Starting,
                    Phase::Waiting { reason, .. } if reason.is_empty() => Status::Starting,
                    Phase::Waiting { reason, .. } => Status::Restarting {
                        attempt: instance.failures,
                        reason: reason.clone(),
                    },
                    Phase::Failed(reason) => Status::Failed(reason.clone()),
                    Phase::Unavailable(reason) => Status::Unavailable(reason.clone()),
                },
            })
            .collect();
        {
            // Forget results of documents no server holds any more.
            let held: std::collections::HashSet<String> = self
                .instances
                .iter()
                .flat_map(|i| i.documents.iter().map(|d| uri::key(&d.uri)))
                .collect();
            let mut store = self.results.lock().unwrap();
            let before = store.len();
            store.retain(|key, _| held.contains(key));
            self.results_changed |= store.len() != before;
        }
        if std::mem::take(&mut self.results_changed) {
            (self.on_change)();
        }
        if infos != self.published {
            *self.infos.lock().unwrap() = infos.clone();
            self.published = infos;
            (self.on_change)();
        }
    }

    fn shutdown(&mut self) {
        let handles: Vec<JoinHandle<()>> = self
            .instances
            .drain(..)
            .filter_map(|instance| stop_in_background(instance.phase))
            .collect();
        for handle in handles {
            let _ = handle.join();
        }
        self.published.clear();
        self.infos.lock().unwrap().clear();
    }
}

/// Stops a running server without making the coordinator wait for it.
fn stop_in_background(phase: Phase) -> Option<JoinHandle<()>> {
    let Phase::Running(mut running) = phase else {
        return None;
    };
    thread::Builder::new()
        .name("loom-lsp-stop".into())
        .spawn(move || {
            if let Some(ready) = running.ready.as_mut() {
                for Notification { method, params } in ready.sync.close_all() {
                    running.client.notify(method, params);
                }
            }
            running.client.stop();
        })
        .ok()
}
