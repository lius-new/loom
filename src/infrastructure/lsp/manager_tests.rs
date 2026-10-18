//! The coordinator against real fake-server processes (`examples/fake_lsp.rs`).
use super::manager::{Manager, Options, Snapshot, SnapshotDocument, Status};
use crate::domain::language::Registry;
use crate::features::language_service::sync::Stamp;
use crate::features::settings::servers::LanguageServerSetting;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let plain = r#"{ "name": "plaintext", "display_name": "Plain", "badge": "T",
            "variants": [{ "id": "plaintext" }] }"#;
        let fake = r#"{ "name": "fakelang", "display_name": "Fake", "badge": "F",
            "variants": [{ "id": "fakelang",
              "server": { "definition": "fake-server", "language_id": "fake" } }],
            "files": [{ "extensions": ["fk"], "variant": "fakelang" }] }"#;
        let server = r#"{ "name": "fake-server", "display_name": "Fake server", "command": "unused",
            "root": { "markers": ["fake.toml"], "strategy": "nearest", "fallback": "file_directory" } }"#;
        Registry::from_parts(
            &[("plaintext", plain), ("fakelang", fake)],
            &[],
            &[("fake-server", server)],
        )
        .expect("test registry")
    })
}

fn fake_server() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().and_then(|deps| deps.parent()).unwrap();
    let name = if cfg!(windows) {
        "fake_lsp.exe"
    } else {
        "fake_lsp"
    };
    dir.join("examples").join(name)
}

static RUNS: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    manager: Option<Manager>,
    dir: PathBuf,
    log: PathBuf,
    settings: Vec<LanguageServerSetting>,
    changes: Arc<AtomicUsize>,
}

fn options() -> Options {
    Options {
        timeouts: crate::infrastructure::lsp::client::Timeouts {
            initialize: Duration::from_secs(10),
            request: Duration::from_secs(5),
            exit_grace: Duration::from_millis(500),
        },
        idle_stop: Duration::from_millis(600),
        backoff: vec![Duration::from_millis(50), Duration::from_millis(50)],
        stable_after: Duration::from_secs(60),
    }
}

impl Fixture {
    fn mgr(&self) -> &Manager {
        self.manager.as_ref().unwrap()
    }

    fn new(extra_args: &[&str]) -> Self {
        let n = RUNS.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("loom-mgr-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("server.log");
        let mut args: Vec<String> = if extra_args.contains(&"--sync") {
            Vec::new()
        } else {
            ["--sync", "full"].map(String::from).into()
        };
        args.extend(extra_args.iter().map(|a| a.to_string()));
        args.extend(["--log".into(), log.display().to_string()]);
        let settings = vec![LanguageServerSetting {
            server: "fake-server".into(),
            enabled: true,
            command: Some(fake_server().display().to_string()),
            args: Some(args),
            initialization_options: None,
        }];
        let changes = Arc::new(AtomicUsize::new(0));
        let counter = changes.clone();
        let manager = Manager::start(registry(), options(), move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        Self {
            manager: Some(manager),
            dir,
            log,
            settings,
            changes,
        }
    }

    fn doc(&self, name: &str, version: u64, text: &str) -> SnapshotDocument {
        let path = self.dir.join(name);
        SnapshotDocument {
            language: registry().resolve_path(&path),
            path,
            stamp: Stamp {
                incarnation: 1,
                version,
            },
            text: Arc::new(text.to_owned()),
        }
    }

    fn show(&self, documents: Vec<SnapshotDocument>) {
        self.mgr().update(Snapshot {
            documents,
            workspace_roots: vec![self.dir.clone()],
            servers: self.settings.clone(),
        });
    }

    fn wait_for<T>(&self, what: &str, mut check: impl FnMut() -> Option<T>) -> T {
        let limit = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(found) = check() {
                return found;
            }
            assert!(
                Instant::now() < limit,
                "timed out waiting for {what}; infos {:?}",
                self.mgr().infos()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_status(&self, what: &str, want: impl Fn(&Status) -> bool) {
        self.wait_for(what, || {
            self.mgr()
                .infos()
                .iter()
                .any(|i| want(&i.status))
                .then_some(())
        });
    }

    fn log(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn methods(&self) -> Vec<String> {
        self.log()
            .iter()
            .filter_map(|m| m["method"].as_str().map(str::to_owned))
            .collect()
    }

    fn wait_method_count(&self, method: &str, count: usize) {
        self.wait_for(&format!("{count} x {method}"), || {
            (self.methods().iter().filter(|m| *m == method).count() >= count).then_some(())
        });
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_document_starts_its_server_and_is_opened_once_ready() {
    let f = Fixture::new(&[]);
    f.show(vec![f.doc("a.fk", 0, "hello")]);
    f.wait_status("ready", |s| *s == Status::Ready);
    f.wait_method_count("textDocument/didOpen", 1);
    let open = f
        .log()
        .into_iter()
        .find(|m| m["method"] == "textDocument/didOpen")
        .unwrap();
    assert_eq!(open["params"]["textDocument"]["languageId"], "fake");
    assert_eq!(open["params"]["textDocument"]["text"], "hello");
    assert!(
        f.changes.load(Ordering::Relaxed) >= 1,
        "status changes are announced"
    );
    let info = &f.mgr().infos()[0];
    assert_eq!(info.documents, 1);
    assert_eq!(info.root, f.dir, "no marker: the file's own folder");
}

#[test]
fn edits_and_closes_follow_the_snapshots() {
    let f = Fixture::new(&[]);
    f.show(vec![f.doc("a.fk", 0, "one"), f.doc("b.fk", 0, "bee")]);
    f.wait_method_count("textDocument/didOpen", 2);
    f.show(vec![f.doc("a.fk", 4, "one two"), f.doc("b.fk", 0, "bee")]);
    f.wait_method_count("textDocument/didChange", 1);
    let change = f
        .log()
        .into_iter()
        .find(|m| m["method"] == "textDocument/didChange")
        .unwrap();
    assert_eq!(
        change["params"]["contentChanges"],
        json!([{ "text": "one two" }])
    );
    assert_eq!(change["params"]["textDocument"]["version"], 2);
    // Same snapshot again: nothing new is sent.
    f.show(vec![f.doc("a.fk", 4, "one two"), f.doc("b.fk", 0, "bee")]);
    f.show(vec![f.doc("a.fk", 4, "one two")]);
    f.wait_method_count("textDocument/didClose", 1);
    assert_eq!(
        f.methods()
            .iter()
            .filter(|m| *m == "textDocument/didChange")
            .count(),
        1
    );
}

#[test]
fn files_the_server_does_not_serve_start_nothing() {
    let f = Fixture::new(&[]);
    f.show(vec![f.doc("notes.txt", 0, "plain")]);
    std::thread::sleep(Duration::from_millis(300));
    assert!(f.mgr().infos().is_empty());
    assert!(f.methods().is_empty());
}

#[test]
fn a_marker_file_chooses_the_project_root() {
    let f = Fixture::new(&[]);
    let project = f.dir.join("proj");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("fake.toml"), "").unwrap();
    f.show(vec![f.doc("proj/src/a.fk", 0, "x")]);
    f.wait_status("ready", |s| *s == Status::Ready);
    assert_eq!(f.mgr().infos()[0].root, project);
}

#[test]
fn a_missing_program_is_unavailable_and_not_retried() {
    let mut f = Fixture::new(&[]);
    f.settings[0].command = Some(f.dir.join("no-such-server").display().to_string());
    f.show(vec![f.doc("a.fk", 0, "x")]);
    f.wait_status(
        "unavailable",
        |s| matches!(s, Status::Unavailable(m) if m.contains("not found")),
    );
    std::thread::sleep(Duration::from_millis(300));
    assert!(matches!(f.mgr().infos()[0].status, Status::Unavailable(_)));
}

#[test]
fn a_crashing_server_is_restarted_then_given_up_on() {
    let f = Fixture::new(&["--crash-after-init"]);
    f.show(vec![f.doc("a.fk", 0, "x")]);
    // Initial run plus two restarts: three initialize requests in all.
    f.wait_status("failed", |s| matches!(s, Status::Failed(_)));
    let initializes = f.methods().iter().filter(|m| *m == "initialize").count();
    assert_eq!(initializes, 3, "{:?}", f.methods());
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        f.methods().iter().filter(|m| *m == "initialize").count(),
        3,
        "no further tries"
    );
}

#[test]
fn restart_revives_a_failed_server_and_reopens_documents() {
    let f = Fixture::new(&["--crash-after-init"]);
    f.show(vec![f.doc("a.fk", 0, "x")]);
    f.wait_status("failed", |s| matches!(s, Status::Failed(_)));
    f.mgr().restart();
    f.wait_for("a fourth initialize", || {
        (f.methods().iter().filter(|m| *m == "initialize").count() >= 4).then_some(())
    });
}

#[test]
fn an_idle_server_is_stopped_and_exits_cleanly() {
    let f = Fixture::new(&[]);
    f.show(vec![f.doc("a.fk", 0, "x")]);
    f.wait_status("ready", |s| *s == Status::Ready);
    f.show(vec![]);
    f.wait_for("the instance to go", || {
        f.mgr().infos().is_empty().then_some(())
    });
    f.wait_method_count("exit", 1);
    assert!(f.methods().contains(&"shutdown".to_owned()));
}

#[test]
fn changing_the_launch_replaces_the_instance() {
    let mut f = Fixture::new(&[]);
    f.show(vec![f.doc("a.fk", 0, "x")]);
    f.wait_status("ready", |s| *s == Status::Ready);
    f.settings[0]
        .args
        .as_mut()
        .unwrap()
        .push("--stderr-noise".into());
    f.show(vec![f.doc("a.fk", 0, "x")]);
    f.wait_method_count("initialize", 2);
    f.wait_method_count("textDocument/didOpen", 2);
}

#[test]
fn turning_the_server_off_stops_it() {
    let mut f = Fixture::new(&[]);
    f.show(vec![f.doc("a.fk", 0, "x")]);
    f.wait_status("ready", |s| *s == Status::Ready);
    f.settings[0].enabled = false;
    f.show(vec![f.doc("a.fk", 0, "x")]);
    f.wait_for("the instance to go", || {
        f.mgr().infos().is_empty().then_some(())
    });
}

#[test]
fn saving_sends_did_save_to_servers_that_want_it() {
    let f = Fixture::new(&["--sync", "object"]);
    f.show(vec![f.doc("a.fk", 0, "x")]);
    f.wait_method_count("textDocument/didOpen", 1);
    f.mgr().saved(f.dir.join("a.fk"), Arc::new("x".into()));
    f.wait_method_count("textDocument/didSave", 1);
    let save = f
        .log()
        .into_iter()
        .find(|m| m["method"] == "textDocument/didSave")
        .unwrap();
    assert_eq!(save["params"]["text"], "x", "the fake asks for the text");
}

#[test]
fn dropping_the_manager_shuts_servers_down() {
    let f = Fixture::new(&[]);
    f.show(vec![f.doc("a.fk", 0, "x")]);
    f.wait_status("ready", |s| *s == Status::Ready);
    let mut f = f;
    drop(f.manager.take());
    let log = f.log.clone();
    let methods: Vec<String> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|m| m["method"].as_str().map(str::to_owned))
        .collect();
    assert!(
        methods.contains(&"textDocument/didClose".to_owned()),
        "{methods:?}"
    );
    assert!(methods.contains(&"shutdown".to_owned()), "{methods:?}");
    assert!(methods.contains(&"exit".to_owned()), "{methods:?}");
}
