//! Atomic capability transactions; a watch registration is acknowledged only
//! after its candidate disk lease is ready. Run + lease generations reject late work.
use super::{Coordinator, Msg, Phase, Running};
use crate::features::language_service::{
    dynamic::Registry,
    message::{Id, ResponseError},
};
use crate::infrastructure::lsp_watch;
use serde_json::Value;
use std::time::{Duration, Instant};

pub(super) struct Pending {
    id: Id,
    registry: Registry,
    watch: lsp_watch::Handle,
    generation: u64,
    pub deadline: Instant,
}

impl Coordinator {
    pub(super) fn registration_request(&mut self, at: usize, id: Id, method: &str, params: &Value) {
        let instance = &mut self.instances[at];
        let Phase::Running(running) = &mut instance.phase else {
            return;
        };
        let Some(ready) = &mut running.ready else {
            running.client.respond(
                id,
                Err(ResponseError::new(-32002, "initialize has not completed")),
            );
            return;
        };
        if ready.registration_pending.is_some() {
            if ready.registration_queue.len() < 32 {
                ready
                    .registration_queue
                    .push_back((id, method.into(), params.clone()));
            } else {
                running.client.respond(
                    id,
                    Err(ResponseError::new(-32000, "registration queue is full")),
                );
            }
            return;
        }
        let candidate = if method == "client/registerCapability" {
            ready.registrations.register(params, &instance.key.root)
        } else {
            ready.registrations.unregister(params)
        };
        let registry = match candidate {
            Ok(registry) => registry,
            Err(error) => {
                running
                    .client
                    .respond(id, Err(ResponseError::new(-32602, &error)));
                self.remember(at, 2, &error);
                return;
            }
        };
        if registry.watch_signature() == ready.registrations.watch_signature() {
            ready.registrations = registry;
            running.client.respond(id, Ok(Value::Null));
            self.registration_changed(at);
            return;
        }
        ready.next_registration += 1;
        let generation = ready.next_registration;
        let watches = registry.watches();
        if watches.is_empty() {
            ready.watch = None;
            ready.watch_generation = generation;
            ready.registrations = registry;
            running.client.respond(id, Ok(Value::Null));
            self.registration_changed(at);
            return;
        }
        let (instance, run) = (instance.id, instance.run);
        let done = self.tx.clone();
        let events = self.tx.clone();
        let errors = self.tx.clone();
        match lsp_watch::prepare(
            watches,
            ready.watch.as_ref(),
            move |outcome| {
                let _ = done.send(Msg::RegistrationReady {
                    instance,
                    run,
                    generation,
                    outcome,
                });
            },
            move |changes| {
                let _ = events.send(Msg::WatchedFiles {
                    instance,
                    run,
                    generation,
                    changes,
                });
            },
            move |error| {
                let _ = errors.send(Msg::WatchError {
                    instance,
                    run,
                    generation,
                    error,
                });
            },
        ) {
            Ok(watch) => {
                ready.registration_pending = Some(Pending {
                    id,
                    registry,
                    watch,
                    generation,
                    deadline: Instant::now() + Duration::from_secs(20),
                })
            }
            Err(error) => {
                if let Some(watch) = &ready.watch {
                    watch.activate();
                }
                running.client.respond(
                    id,
                    Err(ResponseError::new(
                        -32603,
                        format!("cannot start file watch worker: {error}"),
                    )),
                );
            }
        }
    }

    pub(super) fn registration_ready(
        &mut self,
        instance: u64,
        run: u64,
        generation: u64,
        outcome: Result<(), String>,
    ) {
        let Some(at) = self
            .instances
            .iter()
            .position(|i| i.id == instance && i.run == run)
        else {
            return;
        };
        let Phase::Running(Running {
            client,
            ready: Some(ready),
        }) = &mut self.instances[at].phase
        else {
            return;
        };
        if !ready
            .registration_pending
            .as_ref()
            .is_some_and(|p| p.generation == generation)
        {
            return;
        }
        let pending = ready.registration_pending.take().unwrap();
        match outcome {
            Ok(()) => {
                ready.registrations = pending.registry;
                ready.watch = Some(pending.watch);
                ready.watch_generation = generation;
                client.respond(pending.id, Ok(Value::Null));
                ready.watch.as_ref().unwrap().activate();
                self.registration_changed(at);
            }
            Err(error) => {
                if let Some(watch) = &ready.watch {
                    watch.activate();
                }
                client.respond(pending.id, Err(ResponseError::new(-32603, &error)));
                self.remember(at, 2, &format!("registration rejected: {error}"));
            }
        }
        self.drain_registrations(at);
    }

    fn drain_registrations(&mut self, at: usize) {
        loop {
            let Phase::Running(Running {
                ready: Some(ready), ..
            }) = &mut self.instances[at].phase
            else {
                return;
            };
            if ready.registration_pending.is_some() {
                return;
            }
            let Some((id, method, params)) = ready.registration_queue.pop_front() else {
                return;
            };
            self.registration_request(at, id, &method, &params);
        }
    }

    pub(super) fn expire_registrations(&mut self, now: Instant) {
        let expired = self
            .instances
            .iter()
            .filter_map(|instance| match &instance.phase {
                Phase::Running(Running {
                    ready: Some(ready), ..
                }) => ready
                    .registration_pending
                    .as_ref()
                    .filter(|p| p.deadline <= now)
                    .map(|p| (instance.id, instance.run, p.generation)),
                _ => None,
            })
            .collect::<Vec<_>>();
        for (instance, run, generation) in expired {
            self.registration_ready(
                instance,
                run,
                generation,
                Err("file watch registration timed out".into()),
            );
        }
    }

    pub(super) fn watch_instance(&self, instance: u64, run: u64, generation: u64) -> Option<usize> {
        self.instances.iter().position(|i| i.id == instance && i.run == run && matches!(&i.phase, Phase::Running(Running{ready:Some(ready),..}) if ready.watch.is_some() && ready.watch_generation == generation))
    }

    pub(super) fn watched_files(
        &mut self,
        instance: u64,
        run: u64,
        generation: u64,
        changes: Vec<Value>,
    ) {
        let Some(at) = self.watch_instance(instance, run, generation) else {
            return;
        };
        if let Phase::Running(running) = &self.instances[at].phase {
            // Keep every wire message bounded even after a large directory move.
            for changes in changes.chunks(4096) {
                running.client.notify(
                    "workspace/didChangeWatchedFiles",
                    serde_json::json!({"changes":changes}),
                );
            }
        }
    }
}
