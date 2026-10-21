//! These tests run the real client against `examples/fake_lsp.rs`, a scriptable
//! server in its own process, so pipes, threads, timeouts and process control are
//! exercised for real. `cargo test` builds the example first.
use super::client::{Client, Launch, Timeouts};
use crate::features::language_service::{
    capabilities::{PositionEncoding, SyncKind},
    message::REQUEST_CANCELLED,
    session::{Event, SessionOptions, State},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

pub(super) fn fake_server() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().and_then(|deps| deps.parent()).unwrap();
    let name = if cfg!(windows) {
        "fake_lsp.exe"
    } else {
        "fake_lsp"
    };
    let path = dir.join("examples").join(name);
    assert!(
        path.exists(),
        "the fake server is missing: build it with `cargo build --example fake_lsp`"
    );
    path
}

fn quick() -> Timeouts {
    Timeouts {
        initialize: Duration::from_secs(10),
        request: Duration::from_secs(5),
        exit_grace: Duration::from_millis(500),
    }
}

struct Harness {
    client: Client,
    events: Receiver<Event>,
    history: Vec<Event>,
    log: PathBuf,
    _cleanup: Cleanup,
}

/// Removes the server's log file when the test ends.
struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

static RUNS: AtomicUsize = AtomicUsize::new(0);

pub(super) fn options() -> SessionOptions {
    SessionOptions {
        root_uri: "file:///fake/root".into(),
        root_name: "root".into(),
        process_id: Some(std::process::id()),
        client_version: "test".into(),
        initialization_options: Some(json!({ "answer": 42 })),
        configuration: BTreeMap::from([("fake".to_owned(), json!({ "on": true }))]),
    }
}

fn launch(args: &[&str], timeouts: Timeouts) -> Harness {
    let log = std::env::temp_dir().join(format!(
        "loom-fake-lsp-{}-{}.log",
        std::process::id(),
        RUNS.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&log);
    let mut arguments: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    arguments.extend(["--log".into(), log.display().to_string()]);
    let (send, events) = mpsc::channel();
    let client = Client::spawn(
        Launch {
            env: Default::default(),
            command: fake_server().display().to_string(),
            args: arguments,
            cwd: std::env::temp_dir(),
            session: options(),
            timeouts,
            max_message: 32 * 1024 * 1024,
        },
        move |event| {
            let _ = send.send(event);
        },
    )
    .expect("the fake server starts");
    Harness {
        client,
        events,
        history: Vec::new(),
        _cleanup: Cleanup(log.clone()),
        log,
    }
}

impl Harness {
    /// The first event, now or soon, for which `matches` is true. Earlier ones are
    /// kept in `history`.
    fn wait(&mut self, what: &str, matches: impl Fn(&Event) -> bool) -> Event {
        if let Some(at) = self.history.iter().position(&matches) {
            return self.history.remove(at);
        }
        let limit = Instant::now() + Duration::from_secs(15);
        loop {
            let left = limit.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(left) {
                Ok(event) if matches(&event) => return event,
                Ok(event) => self.history.push(event),
                Err(_) => panic!("no event for {what}; saw {:?}", self.history),
            }
        }
    }

    fn ready(&mut self) -> crate::features::language_service::capabilities::ServerCapabilities {
        match self.wait("ready", |e| matches!(e, Event::Ready(_))) {
            Event::Ready(capabilities) => capabilities,
            _ => unreachable!(),
        }
    }

    fn failed(&mut self) -> String {
        match self.wait("failure", |e| {
            matches!(e, Event::StateChanged(State::Failed(_)))
        }) {
            Event::StateChanged(State::Failed(reason)) => reason,
            _ => unreachable!(),
        }
    }

    fn response(
        &mut self,
        tag: u64,
    ) -> Result<Value, crate::features::language_service::message::ResponseError> {
        match self.wait(
            "a response",
            |e| matches!(e, Event::Response { tag: t, .. } if *t == tag),
        ) {
            Event::Response { outcome, .. } => outcome,
            _ => unreachable!(),
        }
    }

    /// What the fake server received, as JSON objects.
    fn log(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    /// The log once `ready` approves it, polling for a moment (the server writes
    /// it asynchronously to what the client observes).
    fn log_until(&self, what: &str, ready: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let limit = Instant::now() + Duration::from_secs(10);
        loop {
            let log = self.log();
            if ready(&log) {
                return log;
            }
            assert!(
                Instant::now() < limit,
                "the log never showed {what}: {log:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn methods(log: &[Value]) -> Vec<String> {
    log.iter()
        .filter_map(|m| m.get("method").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

#[test]
fn concurrent_fake_servers_keep_shared_log_messages_intact() {
    let log = std::env::temp_dir().join(format!(
        "loom-shared-lsp-{}-{}.log",
        std::process::id(),
        RUNS.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&log);
    let _cleanup = Cleanup(log.clone());
    let mut servers: Vec<_> = (0..4)
        .map(|_| launch(&["--log", log.to_str().unwrap()], quick()))
        .collect();
    for server in &mut servers {
        server.ready();
    }
    let barrier = std::sync::Barrier::new(servers.len());
    std::thread::scope(|scope| {
        for (writer, mut server) in servers.into_iter().enumerate() {
            let barrier = &barrier;
            scope.spawn(move || {
                barrier.wait();
                for sequence in 0..16 {
                    let params = json!({
                        "writer": writer, "sequence": sequence,
                        "items": (0..32).map(|n| json!({"n": n, "text": "日志内容"})).collect::<Vec<_>>()
                    });
                    assert!(server.client.request("custom/shared-log", params.clone(), sequence));
                    assert_eq!(server.response(sequence).unwrap()["params"], params);
                }
                server.client.stop();
            });
        }
    });
    // Restarted instances overlap while the old process shuts down. Every log
    // record must survive that overlap, including initialize and didOpen.
    let records: Vec<Value> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("one complete JSON message per line"))
        .collect();
    let requests: std::collections::HashSet<_> = records
        .iter()
        .filter(|record| record["method"] == "custom/shared-log")
        .map(|record| {
            (
                record["params"]["writer"].as_u64().unwrap(),
                record["params"]["sequence"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(requests.len(), 64);
    assert_eq!(
        methods(&records)
            .iter()
            .filter(|m| *m == "initialize")
            .count(),
        4
    );
}

/// Server flags, then the encoding, sync kind and (full, delta, range) semantic modes expected.
type Case<'a> = (
    &'a [&'a str],
    PositionEncoding,
    SyncKind,
    Option<(bool, bool, bool)>,
);

#[test]
fn the_handshake_reports_each_servers_capabilities() {
    let cases: [Case; 4] = [
        (&[], PositionEncoding::Utf16, SyncKind::None, None),
        (
            &[
                "--encoding",
                "utf-8",
                "--sync",
                "incremental",
                "--semantic",
                "delta",
            ],
            PositionEncoding::Utf8,
            SyncKind::Incremental,
            Some((true, true, false)),
        ),
        (
            &[
                "--encoding",
                "utf-16",
                "--sync",
                "full",
                "--semantic",
                "range",
            ],
            PositionEncoding::Utf16,
            SyncKind::Full,
            Some((false, false, true)),
        ),
        (
            &["--sync", "object", "--semantic", "full"],
            PositionEncoding::Utf16,
            SyncKind::Incremental,
            Some((true, false, false)),
        ),
    ];
    for (args, encoding, sync, semantic) in cases {
        let mut server = launch(args, quick());
        let capabilities = server.ready();
        assert_eq!(capabilities.position_encoding, encoding, "{args:?}");
        assert_eq!(capabilities.text_sync.change, sync, "{args:?}");
        assert_eq!(
            capabilities
                .semantic_tokens
                .as_ref()
                .map(|s| (s.full, s.delta, s.range)),
            semantic,
            "{args:?}"
        );
        if let Some(tokens) = &capabilities.semantic_tokens {
            assert_eq!(tokens.legend.token_types, ["keyword", "function"]);
        }
        assert_eq!(server.client.state(), State::Ready);

        let log = server.log_until("initialized", |log| {
            methods(log).contains(&"initialized".to_owned())
        });
        assert_eq!(methods(&log)[..2], ["initialize", "initialized"]);
        let params = &log[0]["params"];
        assert_eq!(params["rootUri"], json!("file:///fake/root"));
        assert_eq!(params["initializationOptions"], json!({ "answer": 42 }));
        assert_eq!(params["clientInfo"]["name"], json!("Loom"));
        assert!(params["processId"].as_u64().is_some());
        server.client.stop();
    }
}

#[test]
fn requests_return_to_their_callers_with_non_ascii_payloads_intact() {
    let mut server = launch(&[], quick());
    server.ready();
    let payload = |n: u64| json!({ "text": format!("第{n}行 😀 é\n\"quoted\""), "n": n });
    for tag in 1..=3 {
        assert!(
            server
                .client
                .request(&format!("custom/echo{tag}"), payload(tag), tag)
        );
    }
    // The answers come back whatever their order; each is matched by its tag.
    for tag in (1..=3).rev() {
        let result = server.response(tag).expect("answered");
        assert_eq!(result["method"], json!(format!("custom/echo{tag}")));
        assert_eq!(result["params"], payload(tag));
    }
    server.client.stop();
}

#[test]
fn messages_split_into_single_bytes_or_glued_together_are_both_understood() {
    let mut server = launch(&["--chunk", "1"], quick());
    server.ready();
    server
        .client
        .request("custom/echo", json!({ "k": "中文" }), 7);
    assert_eq!(
        server.response(7).unwrap()["params"],
        json!({ "k": "中文" })
    );
    server.client.stop();

    let mut server = launch(&["--glue"], quick());
    server.ready();
    server.client.request("custom/echo", json!({}), 8);
    server.response(8).unwrap();
    server.wait(
        "the glued log message",
        |e| matches!(e, Event::Log { message, .. } if message == "glued"),
    );
    server.client.stop();
}

#[test]
fn a_server_that_refuses_to_initialize_fails_the_session_with_its_reason() {
    let mut server = launch(&["--init-error"], quick());
    assert!(server.failed().contains("no usable workspace"));
    assert!(matches!(server.client.state(), State::Failed(_)));
    server.client.stop();

    let mut server = launch(&["--encoding", "utf-32"], quick());
    assert!(server.failed().contains("utf-32"));
    server.client.stop();
}

#[test]
fn a_server_that_never_finishes_initializing_is_ended() {
    let started = Instant::now();
    let mut server = launch(
        &["--init-delay-ms", "30000"],
        Timeouts {
            initialize: Duration::from_millis(300),
            ..quick()
        },
    );
    assert!(server.failed().contains("timed out"));
    server.client.stop();
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "the stuck process was ended, not waited for"
    );
}

#[test]
fn a_request_the_server_ignores_times_out_and_the_session_carries_on() {
    let mut server = launch(
        &["--ignore-requests"],
        Timeouts {
            request: Duration::from_millis(300),
            ..quick()
        },
    );
    server.ready();
    server.client.request("custom/never", json!({}), 1);
    let error = server.response(1).unwrap_err();
    assert!(error.message.contains("timed out"), "{error}");
    assert_eq!(
        server.client.state(),
        State::Ready,
        "one slow request is not a failure"
    );

    // The server is told to stop working on it.
    let log = server.log_until("the cancellation", |log| {
        methods(log).contains(&"$/cancelRequest".to_owned())
    });
    let cancel = log
        .iter()
        .find(|m| m["method"] == json!("$/cancelRequest"))
        .unwrap();
    assert_eq!(cancel["params"]["id"], json!(1));
    assert!(server.client.notify("custom/still-works", json!({})));
    server.log_until("a later notification", |log| {
        methods(log).contains(&"custom/still-works".to_owned())
    });
    server.client.stop();
}

#[test]
fn a_crash_fails_the_session_and_every_request_still_waiting() {
    let mut server = launch(&["--crash-on", "custom/boom"], quick());
    server.ready();
    // The server dies on the first; the second is never read.
    server.client.request("custom/boom", json!({}), 1);
    server.client.request("custom/queued", json!({}), 2);
    let reason = server.failed();
    assert!(reason.contains("exited with code 4"), "{reason}");
    for tag in [1, 2] {
        let error = server.response(tag).unwrap_err();
        assert!(!error.message.is_empty());
    }
    server.client.stop();

    let mut server = launch(&["--crash-after-init"], quick());
    assert!(server.failed().contains("exited with code 3"));
    server.client.stop();
}

#[test]
fn shutdown_is_orderly_and_the_process_goes_away() {
    let mut server = launch(&[], quick());
    server.ready();
    let pid = server.client.process_id();
    assert!(server.client.shutdown());
    server.wait("shutting down", |e| {
        matches!(e, Event::StateChanged(State::ShuttingDown))
    });
    server.wait("stopped", |e| {
        matches!(e, Event::StateChanged(State::Stopped))
    });
    let log = server.log_until("exit", |log| methods(log).contains(&"exit".to_owned()));
    let order = methods(&log);
    let at = |name: &str| order.iter().position(|m| m == name).unwrap();
    assert!(at("shutdown") < at("exit"), "{order:?}");
    server.client.stop();
    assert!(!process_exists(pid), "the server process was reaped");
}

#[test]
fn a_server_that_will_not_leave_is_ended_after_the_grace_period() {
    for args in [&["--no-shutdown-reply"][..], &["--linger"][..]] {
        let started = Instant::now();
        let mut server = launch(args, quick());
        server.ready();
        let pid = server.client.process_id();
        server.client.shutdown();
        server.wait("stopped", |e| {
            matches!(e, Event::StateChanged(State::Stopped))
        });
        server.client.stop();
        assert!(started.elapsed() < Duration::from_secs(10), "{args:?}");
        assert!(!process_exists(pid), "{args:?}: the process was ended");
    }
}

#[test]
fn dropping_the_client_ends_the_process() {
    let mut server = launch(&[], quick());
    server.ready();
    let pid = server.client.process_id();
    assert!(process_exists(pid));
    let Harness { client, .. } = server;
    drop(client);
    assert!(!process_exists(pid));
}

#[test]
fn a_program_that_does_not_exist_is_a_start_error() {
    let result = Client::spawn(
        Launch {
            env: Default::default(),
            command: "loom-no-such-language-server".into(),
            args: Vec::new(),
            cwd: std::env::temp_dir(),
            session: options(),
            timeouts: quick(),
            max_message: 1024,
        },
        |_| {},
    );
    let Err(error) = result else {
        panic!("expected an error")
    };
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn the_tail_of_standard_error_is_kept() {
    let mut server = launch(&["--stderr-noise"], quick());
    server.ready();
    let limit = Instant::now() + Duration::from_secs(10);
    while !server
        .client
        .stderr_tail()
        .iter()
        .any(|l| l.contains("ready"))
    {
        assert!(Instant::now() < limit, "{:?}", server.client.stderr_tail());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(server.client.stderr_tail()[0], "fake server: starting up");
    server.client.stop();
}

#[test]
fn what_the_server_asks_of_the_client_is_answered_and_the_answers_arrive() {
    let mut server = launch(&["--ask-config", "--ask-unknown"], quick());
    server.ready();
    let log = server.log_until("both answers", |log| {
        log.iter()
            .any(|m| m["id"] == json!(9001) && m.get("method").is_none())
            && log
                .iter()
                .any(|m| m["id"] == json!(9002) && m.get("method").is_none())
    });
    let configuration = log
        .iter()
        .find(|m| m["id"] == json!(9001) && m.get("method").is_none())
        .unwrap();
    assert_eq!(configuration["result"], json!([{ "on": true }, null]));
    let refused = log
        .iter()
        .find(|m| m["id"] == json!(9002) && m.get("method").is_none())
        .unwrap();
    assert_eq!(refused["error"]["code"], json!(-32601));
    server.wait(
        "the refusal to be noted",
        |e| matches!(e, Event::Refused { method } if method == "client/unheardOf"),
    );
    server.client.stop();
}

#[test]
fn log_lines_and_a_garbled_message_do_not_end_the_session() {
    let mut server = launch(&["--log-message", "--garbage-after-init"], quick());
    server.ready();
    server.wait(
        "the log line",
        |e| matches!(e, Event::Log { message, .. } if message == "hello from the fake server"),
    );
    // The garbage (framed but not JSON) is skipped, and the next request still works.
    server
        .client
        .request("custom/echo", json!({ "after": true }), 5);
    assert_eq!(
        server.response(5).unwrap()["params"],
        json!({ "after": true })
    );
    assert_eq!(server.client.state(), State::Ready);
    server.client.stop();
}

#[test]
fn a_stream_that_cannot_be_followed_fails_the_session() {
    let mut server = launch(&["--bad-frame"], quick());
    assert!(server.failed().contains("protocol error"));
    server.client.stop();

    // A message larger than the limit is refused before its body is read.
    let mut server = launch(&["--oversize"], quick());
    let reason = server.failed();
    assert!(reason.contains("exceeds the limit"), "{reason}");
    server.client.stop();
}

#[test]
fn nothing_can_be_sent_once_the_session_has_ended() {
    let mut server = launch(&["--crash-after-init"], quick());
    server.failed();
    // Wait for the driver to finish; commands then have nowhere to go.
    let limit = Instant::now() + Duration::from_secs(10);
    while server.client.notify("custom/late", json!({})) {
        assert!(Instant::now() < limit, "the driver never finished");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!server.client.request("custom/late", json!({}), 1));
    server.client.stop();
    let _ = REQUEST_CANCELLED;
}

/// Whether the operating system still knows a process.
fn process_exists(pid: u32) -> bool {
    use std::process::Command;
    if cfg!(windows) {
        let output = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .expect("tasklist runs");
        String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
    } else {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[test]
fn a_server_defined_only_as_data_runs_without_any_language_specific_code() {
    use crate::domain::language::Registry;
    use crate::features::language_service::server_set::ServerSet;
    let command = fake_server().display().to_string().replace('\\', "\\\\");
    let server = format!(
        r#"{{ "name": "fake-lsp", "display_name": "Fake", "command": "{command}",
             "args": ["--sync", "incremental"],
             "root": {{ "markers": ["fake.toml"], "fallback": "workspace" }},
             "languages": [{{ "language": "fakelang", "language_id": "fake-dialect" }}] }}"#
    );
    let language = r#"{ "name": "fakelang", "display_name": "Fake", "badge": "FK",
        "variants": [{ "id": "fakelang" }],
        "files": [{ "extensions": ["fake"], "variant": "fakelang" }] }"#;
    let plain = r#"{ "name": "plaintext", "display_name": "Plain", "badge": "T",
        "variants": [{ "id": "plaintext" }] }"#;
    let registry =
        Registry::from_sources(&[("fakelang", language), ("plaintext", plain)], &[]).unwrap();
    let servers = ServerSet::from_sources(&[("fake-lsp", server.as_str())]).unwrap();

    let resolved = registry.resolve_path(std::path::Path::new("/work/thing.fake"));
    let bound = servers.for_language(&registry, resolved);
    assert_eq!(bound.len(), 1, "the language has a server");
    let (definition, language_id) = bound[0];
    assert_eq!(
        language_id, "fake-dialect",
        "the server's own word for the language"
    );

    let (send, events) = mpsc::channel();
    let client = Client::spawn(
        Launch {
            env: Default::default(),
            command: definition.command.to_string(),
            args: definition.args.iter().map(|a| a.to_string()).collect(),
            cwd: std::env::temp_dir(),
            session: options(),
            timeouts: quick(),
            max_message: 1024 * 1024,
        },
        move |event| {
            let _ = send.send(event);
        },
    )
    .unwrap();
    let capabilities = loop {
        match events
            .recv_timeout(Duration::from_secs(15))
            .expect("the handshake")
        {
            Event::Ready(capabilities) => break capabilities,
            _ => continue,
        }
    };
    assert_eq!(
        capabilities.text_sync.change,
        SyncKind::Incremental,
        "the data's own arguments reached the server"
    );
    client.stop();
}
