//! Requests the application makes of the servers while the user works: hover,
//! go to definition, completion and the rest.
//!
//! The application asks with a [`RequestSpec`] and gets a [`RequestHandle`] it
//! can poll for the answers of every server that can reply. Only the document's
//! text at the stamp the request names is ever asked about; if the coordinator
//! no longer holds that state, the request is stale and nothing is sent. A newer
//! request of the same kind for the same document replaces an older one, and
//! dropping the handle cancels the request.
use super::{Coordinator, Msg, Phase, Running};
use crate::features::language_service::{
    capabilities::PositionEncoding,
    message::{CONTENT_MODIFIED, REQUEST_CANCELLED, ResponseError},
    position::LineIndex,
    request::{self, Feature, Merge},
    sync::Stamp,
    uri,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc::Sender},
    time::{Duration, Instant},
};

/// Tags at or above this belong to requests made through this channel; lower
/// ones are the coordinator's own (semantic tokens).
pub(super) const TAG_BASE: u64 = 1 << 40;
/// Requests in flight at once, over all documents.
const MAX_LIVE: usize = 32;
/// How long past the servers' own limit a request may linger before it is
/// finished by force.
const GRACE: Duration = Duration::from_millis(500);

#[derive(Clone, Debug)]
pub(crate) struct RequestSpec {
    pub feature: Feature,
    /// The document the request is about; ignored by requests about the workspace.
    pub path: PathBuf,
    /// The state of the document the offsets below are in.
    pub stamp: Stamp,
    /// Byte offsets into the document at `stamp`.
    pub start: usize,
    pub end: usize,
    /// See [`request::params`].
    pub extra: Value,
    /// For a request about an earlier answer: the instance that gave it.
    pub target: Option<u64>,
}

impl RequestSpec {
    pub fn new(feature: Feature, path: PathBuf, stamp: Stamp, offset: usize) -> Self {
        Self {
            feature,
            path,
            stamp,
            start: offset,
            end: offset,
            extra: Value::Null,
            target: None,
        }
    }

    pub fn range(mut self, end: usize) -> Self {
        self.end = end;
        self
    }

    pub fn extra(mut self, extra: Value) -> Self {
        self.extra = extra;
        self
    }

    pub fn target(mut self, instance: u64) -> Self {
        self.target = Some(instance);
        self
    }
}

/// One server's answer.
#[derive(Clone, Debug)]
pub(crate) struct Part {
    pub instance: u64,
    pub server: Arc<str>,
    /// The server's place in the listing; lower is preferred.
    pub rank: usize,
    pub encoding: PositionEncoding,
    pub value: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The document is no longer in the state the request asked about.
    Stale,
    /// No running server can answer.
    Unsupported,
    /// Too many requests are waiting.
    Busy,
    Failed(String),
}

/// What has come back so far.
#[derive(Clone, Debug, Default)]
pub(crate) struct Progress {
    /// In the servers' listing order.
    pub parts: Vec<Arc<Part>>,
    /// Every server has answered, failed or been given up on.
    pub finished: bool,
    /// Why there is nothing, when the request finished without any part.
    pub failure: Option<Failure>,
}

impl Progress {
    /// The answers to use for `feature`: all of them, or the first non-empty one.
    pub fn usable(&self, feature: Feature) -> Vec<Arc<Part>> {
        match feature.merge() {
            Merge::First => self
                .parts
                .iter()
                .find(|part| !request::is_empty(&part.value))
                .cloned()
                .into_iter()
                .collect(),
            _ => self.parts.clone(),
        }
    }
}

pub(crate) struct RequestHandle {
    id: u64,
    shared: Arc<Mutex<Progress>>,
    tx: Sender<Msg>,
}

impl RequestHandle {
    pub fn poll(&self) -> Progress {
        self.shared.lock().unwrap().clone()
    }

    pub fn is_finished(&self) -> bool {
        self.shared.lock().unwrap().finished
    }

    pub fn cancel(&self) {
        let _ = self.tx.send(Msg::Cancel(self.id));
    }
}

impl Drop for RequestHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl super::Manager {
    /// Starts a request. `on_change` runs when the first and each later answer
    /// arrives.
    pub fn request(&self, spec: RequestSpec) -> RequestHandle {
        let id = self
            .next_request
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let shared = Arc::new(Mutex::new(Progress::default()));
        let _ = self.tx.send(Msg::Request {
            id,
            spec: Box::new(spec),
            shared: shared.clone(),
        });
        RequestHandle {
            id,
            shared,
            tx: self.tx.clone(),
        }
    }
}

struct Live {
    shared: Arc<Mutex<Progress>>,
    /// Requests of the same kind for the same document replace each other.
    replaces: Option<(Feature, PathBuf)>,
    /// Tag to the instance it was sent to.
    waiting: HashMap<u64, u64>,
    parts: Vec<Arc<Part>>,
    stale: bool,
    errors: Vec<String>,
    deadline: Instant,
}

#[derive(Default)]
pub(super) struct Book {
    live: HashMap<u64, Live>,
    /// Tag to the request it belongs to.
    tags: HashMap<u64, u64>,
    next_tag: u64,
}

impl Book {
    pub fn deadline(&self) -> Option<Instant> {
        self.live.values().map(|live| live.deadline).min()
    }
}

fn is_stale(error: &ResponseError) -> bool {
    // -32802 is ServerCancelled: the server dropped the work because the text moved.
    error.code == CONTENT_MODIFIED || error.code == -32802
}

impl Coordinator {
    pub(super) fn request(&mut self, id: u64, spec: RequestSpec, shared: Arc<Mutex<Progress>>) {
        let key = spec
            .feature
            .replaces_older()
            .then(|| (spec.feature, spec.path.clone()));
        if let Some(key) = &key {
            let older: Vec<u64> = self
                .book
                .live
                .iter()
                .filter(|(_, live)| live.replaces.as_ref() == Some(key))
                .map(|(id, _)| *id)
                .collect();
            for old in older {
                self.cancel_request(old);
            }
        }
        if self.book.live.len() >= MAX_LIVE {
            finish(&shared, Vec::new(), Some(Failure::Busy));
            return;
        }
        let document_uri = if spec.feature.about_document() {
            let Some(document_uri) = uri::file_uri(&spec.path) else {
                finish(&shared, Vec::new(), Some(Failure::Unsupported));
                return;
            };
            Some(document_uri)
        } else {
            None
        };
        let document_key = document_uri.as_deref().map(uri::key);

        struct Target {
            at: usize,
            rank: usize,
            start: crate::features::language_service::position::Position,
            end: crate::features::language_service::position::Position,
        }
        let mut targets: Vec<Target> = Vec::new();
        let mut stale = false;
        for (at, instance) in self.instances.iter().enumerate() {
            let Phase::Running(Running {
                ready: Some(ready), ..
            }) = &instance.phase
            else {
                continue;
            };
            if !spec.feature.supported(&ready.capabilities.providers) {
                continue;
            }
            if spec.target.is_some_and(|target| target != instance.id) {
                continue;
            }
            if spec.feature.merge() == Merge::Target && spec.target.is_none() {
                continue;
            }
            let encoding = ready.capabilities.position_encoding;
            let (start, end) = match &document_key {
                Some(key) => {
                    let Some(document) =
                        instance.documents.iter().find(|d| uri::key(&d.uri) == *key)
                    else {
                        continue;
                    };
                    if document.stamp != spec.stamp {
                        stale = true;
                        continue;
                    }
                    let index = LineIndex::new(&document.text);
                    (
                        index.position(spec.start.min(document.text.len()), encoding),
                        index.position(spec.end.min(document.text.len()), encoding),
                    )
                }
                None => Default::default(),
            };
            targets.push(Target {
                at,
                rank: self
                    .servers
                    .rank(&instance.key.server)
                    .unwrap_or(usize::MAX),
                start,
                end,
            });
        }
        targets.sort_by_key(|target| (target.rank, target.at));
        if spec.feature.merge() == Merge::Primary {
            targets.truncate(1);
        }
        if targets.is_empty() {
            let failure = if stale {
                Failure::Stale
            } else {
                Failure::Unsupported
            };
            finish(&shared, Vec::new(), Some(failure));
            return;
        }

        let timeout = self.options.timeouts.request;
        let mut waiting = HashMap::new();
        let mut errors = Vec::new();
        for target in targets {
            let instance = &self.instances[target.at];
            let Phase::Running(Running { client, .. }) = &instance.phase else {
                continue;
            };
            let params = request::params(
                spec.feature,
                document_uri.as_deref().unwrap_or_default(),
                target.start,
                target.end,
                &spec.extra,
            );
            self.book.next_tag += 1;
            let tag = TAG_BASE + self.book.next_tag;
            if client.request_within(spec.feature.method(), params, tag, timeout) {
                waiting.insert(tag, instance.id);
                self.book.tags.insert(tag, id);
            } else {
                errors.push(format!("{}: could not send", instance.key.server));
            }
        }
        if waiting.is_empty() {
            finish(
                &shared,
                Vec::new(),
                Some(Failure::Failed(errors.join("; "))),
            );
            return;
        }
        self.book.live.insert(
            id,
            Live {
                shared,
                replaces: key,
                waiting,
                parts: Vec::new(),
                stale: false,
                errors,
                deadline: Instant::now() + timeout + GRACE,
            },
        );
    }

    /// Gives up on a request: tells the servers and drops what was waiting.
    pub(super) fn cancel_request(&mut self, id: u64) {
        let Some(live) = self.book.live.remove(&id) else {
            return;
        };
        for (tag, instance_id) in &live.waiting {
            self.book.tags.remove(tag);
            if let Some(Phase::Running(Running { client, .. })) = self
                .instances
                .iter()
                .find(|i| i.id == *instance_id)
                .map(|i| &i.phase)
            {
                client.cancel(*tag);
            }
        }
        let mut progress = live.shared.lock().unwrap();
        progress.finished = true;
        progress.parts = live.parts;
    }

    /// An answer to a request made through the channel.
    pub(super) fn request_response(
        &mut self,
        at: usize,
        tag: u64,
        outcome: Result<Value, ResponseError>,
    ) {
        let Some(request) = self.book.tags.remove(&tag) else {
            return; // cancelled or finished already
        };
        let instance = &self.instances[at];
        let rank = self
            .servers
            .rank(&instance.key.server)
            .unwrap_or(usize::MAX);
        let encoding = match &instance.phase {
            Phase::Running(Running {
                ready: Some(ready), ..
            }) => ready.capabilities.position_encoding,
            _ => PositionEncoding::Utf16,
        };
        let part = Part {
            instance: instance.id,
            server: instance.key.server.clone(),
            rank,
            encoding,
            value: Value::Null,
        };
        let name = instance.key.server.to_string();
        let Some(live) = self.book.live.get_mut(&request) else {
            return;
        };
        live.waiting.remove(&tag);
        match outcome {
            Ok(value) => {
                if !value.is_null() {
                    live.parts.push(Arc::new(Part { value, ..part }));
                    live.parts.sort_by_key(|p| (p.rank, p.instance));
                }
            }
            Err(error) if error.code == REQUEST_CANCELLED => {}
            Err(error) if is_stale(&error) => live.stale = true,
            Err(error) => live.errors.push(format!("{name}: {}", error.message)),
        }
        self.settle(request);
    }

    /// Publishes progress; finishes the request once nothing is waited for.
    fn settle(&mut self, request: u64) {
        let Some(live) = self.book.live.get(&request) else {
            return;
        };
        let done = live.waiting.is_empty();
        let failure = (done && live.parts.is_empty()).then(|| {
            if live.stale {
                Some(Failure::Stale)
            } else if live.errors.is_empty() {
                None
            } else {
                Some(Failure::Failed(live.errors.join("; ")))
            }
        });
        let mut progress = live.shared.lock().unwrap();
        progress.parts = live.parts.clone();
        progress.finished = done;
        progress.failure = failure.flatten();
        drop(progress);
        if done {
            self.book.live.remove(&request);
        }
        self.results_changed = true;
    }

    /// The instance went away: whatever it still owed is a failure.
    pub(super) fn requests_lost(&mut self, instance_id: u64) {
        let affected: Vec<u64> = self
            .book
            .live
            .iter()
            .filter(|(_, live)| live.waiting.values().any(|i| *i == instance_id))
            .map(|(id, _)| *id)
            .collect();
        for request in affected {
            let lost: Vec<u64> = self.book.live[&request]
                .waiting
                .iter()
                .filter(|(_, i)| **i == instance_id)
                .map(|(tag, _)| *tag)
                .collect();
            if let Some(live) = self.book.live.get_mut(&request) {
                for tag in lost {
                    live.waiting.remove(&tag);
                    self.book.tags.remove(&tag);
                }
                live.errors.push("the server stopped".to_owned());
            }
            self.settle(request);
        }
    }

    /// Finishes requests whose time is up, with whatever has arrived.
    pub(super) fn expire_requests(&mut self, now: Instant) {
        let late: Vec<u64> = self
            .book
            .live
            .iter()
            .filter(|(_, live)| live.deadline <= now)
            .map(|(id, _)| *id)
            .collect();
        for request in late {
            if let Some(live) = self.book.live.get_mut(&request) {
                let tags: Vec<(u64, u64)> = live.waiting.drain().collect();
                live.errors.push("timed out".to_owned());
                for (tag, instance_id) in tags {
                    self.book.tags.remove(&tag);
                    if let Some(Phase::Running(Running { client, .. })) = self
                        .instances
                        .iter()
                        .find(|i| i.id == instance_id)
                        .map(|i| &i.phase)
                    {
                        client.cancel(tag);
                    }
                }
            }
            self.settle(request);
        }
    }
}

fn finish(shared: &Arc<Mutex<Progress>>, parts: Vec<Arc<Part>>, failure: Option<Failure>) {
    let mut progress = shared.lock().unwrap();
    progress.parts = parts;
    progress.finished = true;
    progress.failure = failure;
}
