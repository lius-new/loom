use super::*;
use serde_json::json;
use std::{
    collections::HashMap,
    io::Cursor,
    sync::{
        Condvar,
        atomic::{AtomicUsize, Ordering},
    },
};

static CASES: AtomicUsize = AtomicUsize::new(0);

/// A scratch directory, removed with the test.
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let n = CASES.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("loom-install-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Blob {
    bytes: Vec<u8>,
    sha256: String,
}
impl Blob {
    fn new(bytes: Vec<u8>) -> Self {
        let mut context = Context::new(&SHA256);
        context.update(&bytes);
        Self {
            sha256: hex(context.finish().as_ref()),
            bytes,
        }
    }
}

/// A pack archive with the given root and entries `(path, content, mode)`.
fn archive(entries: &[(&str, &[u8], u32)]) -> Blob {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, content, mode) in entries {
        let kind = if path.ends_with('/') {
            tar::EntryType::Directory
        } else {
            tar::EntryType::Regular
        };
        raw_entry(&mut builder, path, content, *mode, kind);
    }
    gzip(builder)
}

fn gzip(builder: tar::Builder<Vec<u8>>) -> Blob {
    let tar = builder.into_inner().unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&tar).unwrap();
    Blob::new(encoder.finish().unwrap())
}

/// Writes an entry without the builder's own path checks, so hostile names can be tested.
fn raw_entry(
    builder: &mut tar::Builder<Vec<u8>>,
    path: &str,
    content: &[u8],
    mode: u32,
    kind: tar::EntryType,
) {
    let mut header = tar::Header::new_gnu();
    header.as_mut_bytes()[..path.len()].copy_from_slice(path.as_bytes());
    header.set_size(content.len() as u64);
    header.set_mode(mode);
    header.set_entry_type(kind);
    if kind == tar::EntryType::Symlink {
        header.set_link_name("/etc/passwd").unwrap();
    }
    header.set_cksum();
    builder.append(&header, content).unwrap();
}

fn good_node() -> Blob {
    archive(&[
        ("node/", b"", 0o755),
        ("node/node", b"node binary", 0o755),
        ("node/LICENSE", b"MIT", 0o644),
    ])
}
fn good_demo() -> Blob {
    archive(&[
        ("demo/bin/demo", b"demo binary", 0o755),
        ("demo/data/readme.txt", b"hello", 0o644),
    ])
}

fn catalog(node: &Blob, demo: &Blob) -> Arc<Catalog> {
    Arc::new(Catalog::parse(&catalog_text(node, demo)).unwrap())
}

fn catalog_text(node: &Blob, demo: &Blob) -> String {
    format!(
        r#"
schema = 1
[distribution]
repository = "owner/loom"

[packs.node]
version = "22.1.0"
license = "MIT"
[packs.node.platforms.any]
asset = "node-any.tar.gz"
release = "lsp-servers-1"
sha256 = "{}"
size = {}

[packs.demo]
version = "1.2.3"
license = "MIT"
requires = ["node@22"]
[packs.demo.platforms.any]
asset = "demo-any.tar.gz"
release = "lsp-servers-1"
sha256 = "{}"
size = {}

[servers.demo]
name = "Demo"
pack = "demo"
command = "${{pack}}/bin/demo"
args = ["--runtime=${{runtime.node}}", "--pack=${{pack}}"]
[servers.demo.root]
markers = ["demo.toml"]
fallback = "workspace"
[[servers.demo.languages]]
language = "rust"
language_id = "rust"
"#,
        node.sha256,
        node.bytes.len(),
        demo.sha256,
        demo.bytes.len(),
    )
}

const NODE_URL: &str =
    "https://github.com/owner/loom/releases/download/lsp-servers-1/node-any.tar.gz";
const DEMO_URL: &str =
    "https://github.com/owner/loom/releases/download/lsp-servers-1/demo-any.tar.gz";

/// Everything a test needs: the installer and what it was given to talk to.
struct World {
    _scratch: Scratch,
    managed: PathBuf,
    installer: Arc<Installer>,
    served: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    downloads: Arc<AtomicUsize>,
    smoked: Arc<Mutex<Vec<SmokeRequest>>>,
    smoke_error: Arc<Mutex<Option<String>>>,
}

impl World {
    fn new(node: Blob, demo: Blob) -> Self {
        Self::on("linux-x86_64", node, demo)
    }

    fn on(platform: &str, node: Blob, demo: Blob) -> Self {
        let scratch = Scratch::new();
        let managed = scratch.0.join("managed");
        let catalog = catalog(&node, &demo);
        let served = Arc::new(Mutex::new(HashMap::from([
            (NODE_URL.to_owned(), node.bytes),
            (DEMO_URL.to_owned(), demo.bytes),
        ])));
        let downloads = Arc::new(AtomicUsize::new(0));
        let smoked = Arc::new(Mutex::new(Vec::new()));
        let smoke_error = Arc::new(Mutex::new(None));
        let (served_for, count) = (served.clone(), downloads.clone());
        let download: Arc<Download> = Arc::new(move |url: &str| {
            count.fetch_add(1, Ordering::SeqCst);
            match served_for.lock().unwrap().get(url) {
                Some(bytes) => Ok(Box::new(Cursor::new(bytes.clone())) as Box<dyn Read + Send>),
                None => Err(format!("404 {url}")),
            }
        });
        let (record, error) = (smoked.clone(), smoke_error.clone());
        let smoke: Arc<Smoke> = Arc::new(move |request: &SmokeRequest| {
            record.lock().unwrap().push(request.clone());
            match error.lock().unwrap().clone() {
                Some(message) => Err(message),
                None => Ok(()),
            }
        });
        let installer = Installer::new(
            catalog,
            managed.clone(),
            platform.to_owned(),
            download,
            smoke,
        );
        Self {
            _scratch: scratch,
            managed,
            installer,
            served,
            downloads,
            smoked,
            smoke_error,
        }
    }

    /// Waits until `pack` is neither queued nor in progress.
    fn settle(&self, pack: &str) -> PackStatus {
        let limit = Instant::now() + Duration::from_secs(20);
        loop {
            let status = self.installer.status(pack);
            if !matches!(
                status,
                PackStatus::Queued
                    | PackStatus::Downloading { .. }
                    | PackStatus::Verifying
                    | PackStatus::Starting
                    | PackStatus::Cancelling
            ) {
                return status;
            }
            assert!(Instant::now() < limit, "{pack} did not settle: {status:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn staging_is_empty(&self) -> bool {
        fs::read_dir(self.managed.join("staging"))
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true)
    }
}

fn failure(status: PackStatus) -> String {
    match status {
        PackStatus::Failed(error) => error,
        other => panic!("expected a failure, got {other:?}"),
    }
}

/// A bounded stand-in for an OS read which cannot be interrupted mid-call.
#[derive(Default)]
struct ReadGate {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}
impl ReadGate {
    fn block(&self) {
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        self.changed.notify_all();
        let (state, timeout) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(20), |s| !s.1)
            .unwrap();
        assert!(
            state.1 && !timeout.timed_out(),
            "test did not release the read"
        );
    }
    fn entered(&self) {
        let state = self.state.lock().unwrap();
        let (state, _) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(20), |s| !s.0)
            .unwrap();
        assert!(state.0, "worker did not reach the read");
    }
    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.changed.notify_all();
    }
}
struct GatedReader {
    bytes: Cursor<Vec<u8>>,
    gate: Arc<ReadGate>,
}
impl Read for GatedReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        self.gate.block();
        self.bytes.read(output)
    }
}
fn gate_downloads(world: &mut World) -> Arc<ReadGate> {
    let gate = Arc::new(ReadGate::default());
    let (reader_gate, served, downloads) =
        (gate.clone(), world.served.clone(), world.downloads.clone());
    world.installer = Installer::new(
        world.installer.catalog.clone(),
        world.managed.clone(),
        "linux-x86_64".into(),
        Arc::new(move |url| {
            downloads.fetch_add(1, Ordering::SeqCst);
            let bytes = served.lock().unwrap()[url].clone();
            Ok(Box::new(GatedReader {
                bytes: Cursor::new(bytes),
                gate: reader_gate.clone(),
            }))
        }),
        Arc::new(|_| Ok(())),
    );
    gate
}
fn archives_have_no_partial_files(world: &World) -> bool {
    fs::read_dir(world.managed.join("archives"))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .all(|entry| !entry.file_name().to_string_lossy().starts_with(".install-"))
        })
        .unwrap_or(true)
}

#[test]
fn cancellation_drains_a_blocked_read_removes_queued_dependents_and_requires_manual_resume() {
    let mut w = World::new(good_node(), good_demo());
    let gate = gate_downloads(&mut w);
    w.installer.request("demo").unwrap();
    gate.entered();
    let target = w.installer.cancel_target("demo").unwrap();
    w.installer.cancel(&target);
    assert_eq!(w.installer.status("demo"), PackStatus::Cancelling);
    assert!(w.installer.cancel_target("demo").is_none());
    w.installer.request("demo").unwrap();
    w.installer.retry();
    assert_eq!(w.installer.status("demo"), PackStatus::Cancelling);
    gate.release();
    assert_eq!(w.settle("demo"), PackStatus::Cancelled);
    assert_eq!(
        w.downloads.load(Ordering::SeqCst),
        1,
        "queued server never downloaded"
    );
    assert!(w.installer.installed_dir("node").is_none());
    assert!(w.staging_is_empty() && archives_have_no_partial_files(&w));
    w.installer.resume("demo").unwrap();
    w.installer.cancel(&target); // An old panel must not cancel a resumed job.
    assert!(matches!(w.settle("demo"), PackStatus::Installed(_)));
    assert_eq!(w.downloads.load(Ordering::SeqCst), 3);
}

#[test]
fn cancelling_one_request_retains_a_shared_runtime_until_the_last_request_cancels() {
    for cancel_last in [false, true] {
        let mut w = World::new(good_node(), good_demo());
        let gate = gate_downloads(&mut w);
        w.installer.request("demo").unwrap();
        gate.entered();
        w.installer.request("node").unwrap();
        w.installer
            .cancel(&w.installer.cancel_target("demo").unwrap());
        assert_eq!(w.installer.status("demo"), PackStatus::Cancelled);
        assert!(matches!(
            w.installer.status("node"),
            PackStatus::Downloading { .. }
        ));
        if cancel_last {
            w.installer
                .cancel(&w.installer.cancel_target("node").unwrap());
            assert_eq!(w.installer.status("node"), PackStatus::Cancelling);
        }
        gate.release();
        if cancel_last {
            assert_eq!(w.settle("node"), PackStatus::Cancelled);
            assert!(w.installer.installed_dir("node").is_none());
        } else {
            assert!(matches!(w.settle("node"), PackStatus::Installed(_)));
            w.installer.resume("demo").unwrap();
            assert!(matches!(w.settle("demo"), PackStatus::Installed(_)));
            assert_eq!(
                w.downloads.load(Ordering::SeqCst),
                2,
                "shared runtime downloaded once"
            );
        }
        assert!(w.staging_is_empty() && archives_have_no_partial_files(&w));
    }
}

#[test]
fn resume_while_smoke_is_draining_uses_a_fresh_generation_and_verified_cache() {
    let mut w = World::new(good_node(), good_demo());
    let gate = Arc::new(ReadGate::default());
    let smoke_gate = gate.clone();
    let (served, downloads, smoked) = (w.served.clone(), w.downloads.clone(), w.smoked.clone());
    w.installer = Installer::new(
        w.installer.catalog.clone(),
        w.managed.clone(),
        "linux-x86_64".into(),
        Arc::new(move |url| {
            downloads.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Cursor::new(served.lock().unwrap()[url].clone())))
        }),
        Arc::new(move |request| {
            smoked.lock().unwrap().push(request.clone());
            smoke_gate.block();
            if request.cancellation.cancelled() {
                Err("late smoke failure".into())
            } else {
                Ok(())
            }
        }),
    );
    w.installer.request("demo").unwrap();
    gate.entered();
    let old = w.installer.cancel_target("demo").unwrap();
    w.installer.cancel(&old);
    assert_eq!(w.installer.status("demo"), PackStatus::Cancelling);
    assert!(w.installer.installed_dir("node").is_some());
    w.installer.resume("demo").unwrap();
    let resumed = w.installer.cancel_target("demo").unwrap();
    assert_ne!(old.generation, resumed.generation);
    w.installer.cancel(&old);
    gate.release();
    assert!(matches!(w.settle("demo"), PackStatus::Installed(_)));
    assert_eq!(
        w.downloads.load(Ordering::SeqCst),
        2,
        "verified archives reused"
    );
    assert_eq!(w.smoked.lock().unwrap().len(), 2);
    assert!(w.staging_is_empty() && archives_have_no_partial_files(&w));
    w.installer.cancel(&resumed);
    assert!(
        matches!(w.installer.status("demo"), PackStatus::Installed(_)),
        "activation won the race"
    );
}

#[test]
fn cancellation_during_verification_or_before_smoke_cannot_activate_or_leave_staging() {
    for before_smoke in [false, true] {
        let w = World::new(good_node(), good_demo());
        let weak = Arc::downgrade(&w.installer);
        let listener: Arc<Listener> = Arc::new(move || {
            let installer = weak.upgrade().unwrap();
            let status = installer.status("demo");
            if (before_smoke && status == PackStatus::Starting)
                || (!before_smoke && status == PackStatus::Verifying)
            {
                if let Some(target) = installer.cancel_target("demo") {
                    installer.cancel(&target);
                }
            }
        });
        w.installer.subscribe(listener.clone());
        w.installer.request("demo").unwrap();
        assert_eq!(w.settle("demo"), PackStatus::Cancelled);
        assert!(w.installer.installed_dir("demo").is_none());
        assert!(w.smoked.lock().unwrap().is_empty());
        assert!(w.staging_is_empty() && archives_have_no_partial_files(&w));
        drop(listener);
        w.installer.resume("demo").unwrap();
        assert!(matches!(w.settle("demo"), PackStatus::Installed(_)));
    }
}

#[test]
fn two_servers_share_a_runtime_and_resume_without_workers_waiting_on_queued_dependencies() {
    let node = good_node();
    let demo = good_demo();
    let other = archive(&[("other/server", b"other", 0o755)]);
    let text = format!(
        r#"{}
[packs.other]
version="1.0.0"
license="MIT"
requires=["node@22"]
[packs.other.platforms.any]
asset="other-any.tar.gz"
release="lsp-servers-1"
sha256="{}"
size={}
[servers.other]
name="Other"
pack="other"
command="${{pack}}/server"
[servers.other.root]
markers=["other.toml"]
fallback="workspace"
[[servers.other.languages]]
language="rust"
language_id="rust"
"#,
        catalog_text(&node, &demo),
        other.sha256,
        other.bytes.len()
    );
    let catalog = Arc::new(Catalog::parse(&text).unwrap());
    let mut w = World::new(node, demo);
    w.served.lock().unwrap().insert(
        catalog.archive("other", "linux-x86_64").unwrap().url,
        other.bytes,
    );
    let gate = gate_downloads(&mut w);
    // Replace only metadata before any workers exist; retain the controlled reader.
    w.installer = Installer::new(
        catalog,
        w.managed.clone(),
        "linux-x86_64".into(),
        w.installer.download.clone(),
        Arc::new(|_| Ok(())),
    );
    w.installer.request("demo").unwrap();
    gate.entered();
    w.installer.request("other").unwrap();
    let old = w.installer.cancel_target("demo").unwrap();
    w.installer.cancel(&old);
    assert_eq!(w.installer.status("demo"), PackStatus::Cancelled);
    assert_eq!(w.installer.status("other"), PackStatus::Queued);
    w.installer.resume("demo").unwrap();
    w.installer.cancel(&old);
    assert_eq!(
        w.installer.work.lock().unwrap().running,
        1,
        "dependent jobs do not occupy workers"
    );
    gate.release();
    assert!(matches!(w.settle("demo"), PackStatus::Installed(_)));
    assert!(matches!(w.settle("other"), PackStatus::Installed(_)));
    assert_eq!(
        w.downloads.load(Ordering::SeqCst),
        3,
        "two servers and exactly one runtime"
    );
    assert!(w.staging_is_empty() && archives_have_no_partial_files(&w));
}

#[test]
fn installation_prompt_size_includes_only_missing_archives_and_shared_runtime_ids() {
    let w = World::new(good_node(), good_demo());
    let estimate = w.installer.missing_archives("demo").unwrap();
    assert_eq!(estimate.len(), 2);
    assert!(estimate["node"] > 0 && estimate["demo"] > 0);
    assert_eq!(
        w.downloads.load(Ordering::SeqCst),
        0,
        "estimating never downloads"
    );
    w.installer.request("node").unwrap();
    assert!(matches!(w.settle("node"), PackStatus::Installed(_)));
    let estimate = w.installer.missing_archives("demo").unwrap();
    assert_eq!(estimate.keys().collect::<Vec<_>>(), ["demo"]);
    w.installer.request("demo").unwrap();
    assert!(matches!(w.settle("demo"), PackStatus::Installed(_)));
    assert!(w.installer.missing_archives("demo").unwrap().is_empty());
}

#[test]
fn a_pack_and_what_it_needs_are_downloaded_verified_started_and_activated() {
    let w = World::new(good_node(), good_demo());
    w.installer.request("demo").unwrap();
    let PackStatus::Installed(demo) = w.settle("demo") else {
        panic!("not installed: {:?}", w.installer.status("demo"));
    };
    let PackStatus::Installed(node) = w.settle("node") else {
        panic!("the requirement was not installed");
    };
    assert_eq!(
        fs::read_to_string(demo.join("bin/demo")).unwrap(),
        "demo binary"
    );
    assert_eq!(
        fs::read_to_string(demo.join("data/readme.txt")).unwrap(),
        "hello"
    );
    assert_eq!(
        fs::read_to_string(node.join("node")).unwrap(),
        "node binary"
    );
    assert_eq!(
        demo,
        w.managed.join("packs/demo").join(demo.file_name().unwrap())
    );
    assert!(
        demo.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("1.2.3-")
    );
    assert_eq!(w.installer.installed_dir("demo"), Some(demo.clone()));
    assert_eq!(
        w.downloads.load(Ordering::SeqCst),
        2,
        "the runtime first, then the pack"
    );
    assert!(w.staging_is_empty());

    // The server was started once, from the staged pack, with both directories expanded.
    let smoked = w.smoked.lock().unwrap();
    assert_eq!(smoked.len(), 1);
    assert_eq!(smoked[0].server, "demo");
    assert_eq!(
        smoked[0]
            .program
            .file_name()
            .unwrap()
            .to_string_lossy()
            .trim_end_matches(".exe"),
        "demo"
    );
    assert_eq!(smoked[0].args[0], format!("--runtime={}", node.display()));
    assert!(smoked[0].args[1].starts_with("--pack="));
    assert!(!smoked[0].args[1].contains("${"));
}

#[cfg(unix)]
#[test]
fn executable_permissions_survive_unpacking() {
    use std::os::unix::fs::PermissionsExt;
    let w = World::new(good_node(), good_demo());
    w.installer.request("demo").unwrap();
    let PackStatus::Installed(demo) = w.settle("demo") else {
        panic!("not installed");
    };
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o111;
    assert_ne!(mode(&demo.join("bin/demo")), 0);
    assert_eq!(mode(&demo.join("data/readme.txt")), 0);
}

#[test]
fn an_installed_pack_is_not_downloaded_again_and_progress_is_announced() {
    let w = World::new(good_node(), good_demo());
    let changes = Arc::new(AtomicUsize::new(0));
    let counter = changes.clone();
    let listener: Arc<Listener> = Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    w.installer.subscribe(listener.clone());
    w.installer.request("demo").unwrap();
    w.settle("demo");
    let after_install = w.downloads.load(Ordering::SeqCst);
    assert!(
        changes.load(Ordering::SeqCst) >= 4,
        "queued, working and done are announced"
    );
    assert!(w.installer.generation() > 0);
    w.installer.request("demo").unwrap();
    assert_eq!(
        w.settle("demo"),
        PackStatus::Installed(w.installer.installed_dir("demo").unwrap())
    );
    assert_eq!(w.downloads.load(Ordering::SeqCst), after_install);
    drop(listener);
}

#[test]
fn a_cached_archive_installs_without_the_network() {
    let w = World::new(good_node(), good_demo());
    w.installer.request("demo").unwrap();
    w.settle("demo");
    fs::remove_dir_all(w.managed.join("packs")).unwrap();
    assert_eq!(w.installer.status("demo"), PackStatus::NotInstalled);
    w.served.lock().unwrap().clear();
    let before = w.downloads.load(Ordering::SeqCst);
    w.installer.request("demo").unwrap();
    assert!(matches!(w.settle("demo"), PackStatus::Installed(_)));
    assert_eq!(
        w.downloads.load(Ordering::SeqCst),
        before,
        "both archives came from the cache"
    );
}

#[test]
fn a_download_that_does_not_match_the_pinned_digest_is_refused() {
    let w = World::new(good_node(), good_demo());
    let mut tampered = good_demo().bytes;
    let last = tampered.len() - 9;
    tampered[last] ^= 0xff;
    w.served
        .lock()
        .unwrap()
        .insert(DEMO_URL.to_owned(), tampered);
    w.installer.request("demo").unwrap();
    let error = failure(w.settle("demo"));
    assert!(error.contains("SHA-256"), "{error}");
    assert_eq!(w.installer.installed_dir("demo"), None);
    assert!(w.staging_is_empty());
    assert!(
        !w.managed
            .join("archives")
            .join(format!("{}.tar.gz", good_demo().sha256))
            .exists(),
        "a bad download is never cached"
    );
}

#[test]
fn a_download_of_the_wrong_size_is_refused() {
    for (served, expected) in [
        (good_demo().bytes[..100].to_vec(), "the catalog says"),
        ([good_demo().bytes, vec![0; 50]].concat(), "larger"),
    ] {
        let w = World::new(good_node(), good_demo());
        w.served.lock().unwrap().insert(DEMO_URL.to_owned(), served);
        w.installer.request("demo").unwrap();
        let error = failure(w.settle("demo"));
        assert!(error.contains(expected), "{error}");
        assert_eq!(w.installer.installed_dir("demo"), None);
    }
}

#[test]
fn hostile_archives_are_refused_whole() {
    let mut links = tar::Builder::new(Vec::new());
    raw_entry(&mut links, "demo/ok", b"x", 0o644, tar::EntryType::Regular);
    raw_entry(&mut links, "demo/link", b"", 0o777, tar::EntryType::Symlink);
    let cases: Vec<(&str, Blob, &str)> = vec![
        (
            "traversal",
            archive(&[("demo/../evil", b"x", 0o644)]),
            "unsafe path",
        ),
        (
            "absolute",
            archive(&[("/demo/x", b"x", 0o644)]),
            "unsafe path",
        ),
        (
            "backslash",
            archive(&[("demo\\x", b"x", 0o644)]),
            "unsafe path",
        ),
        (
            "drive",
            archive(&[("demo/C:x", b"x", 0o644)]),
            "unsafe path",
        ),
        (
            "trailing dot",
            archive(&[("demo/x.", b"x", 0o644)]),
            "unsafe path",
        ),
        (
            "other root",
            archive(&[("other/x", b"x", 0o644)]),
            "unexpected root",
        ),
        (
            "repeated",
            archive(&[("demo/a", b"1", 0o644), ("demo/A", b"2", 0o644)]),
            "repeats a path",
        ),
        ("link", gzip(links), "only files and directories"),
        (
            "no pack directory",
            archive(&[("demo", b"x", 0o644)]),
            "no pack directory",
        ),
    ];
    for (name, blob, expected) in cases {
        let w = World::new(good_node(), blob);
        w.installer.request("demo").unwrap();
        let error = failure(w.settle("demo"));
        assert!(error.contains(expected), "{name}: {error}");
        assert_eq!(w.installer.installed_dir("demo"), None, "{name}");
        assert!(w.staging_is_empty(), "{name}");
        assert!(
            w.smoked.lock().unwrap().is_empty(),
            "{name}: nothing was started"
        );
    }
}

#[test]
fn a_pack_whose_server_does_not_start_is_never_activated() {
    let w = World::new(good_node(), good_demo());
    *w.smoke_error.lock().unwrap() = Some("exited with 127".into());
    w.installer.request("demo").unwrap();
    let error = failure(w.settle("demo"));
    assert!(
        error.contains("demo did not start") && error.contains("127"),
        "{error}"
    );
    assert_eq!(w.installer.installed_dir("demo"), None);
    assert!(w.staging_is_empty());
    assert!(
        matches!(w.installer.status("node"), PackStatus::Installed(_)),
        "the runtime that did install stays"
    );
}

#[test]
fn a_requirement_that_cannot_be_installed_fails_the_pack_that_needs_it() {
    let w = World::new(good_node(), good_demo());
    w.served.lock().unwrap().remove(NODE_URL);
    w.installer.request("demo").unwrap();
    let error = failure(w.settle("demo"));
    assert!(error.contains("needs node"), "{error}");
    assert!(matches!(w.installer.status("node"), PackStatus::Failed(_)));
    assert_eq!(w.installer.installed_dir("demo"), None);
}

#[test]
fn a_pack_without_an_archive_for_this_platform_is_unavailable() {
    let node = good_node();
    let demo = good_demo();
    let text = catalog(&node, &demo);
    drop(text);
    let scratch = Scratch::new();
    let only_linux = format!(
        r#"
schema = 1
[distribution]
repository = "owner/loom"
[packs.solo]
version = "1.0.0"
license = "MIT"
[packs.solo.platforms.linux-x86_64]
asset = "solo-linux-x86_64.tar.gz"
release = "lsp-servers-1"
sha256 = "{}"
size = 10
"#,
        node.sha256
    );
    let catalog = Arc::new(Catalog::parse(&only_linux).unwrap());
    let download: Arc<Download> = Arc::new(|_| Err("no network".into()));
    let smoke: Arc<Smoke> = Arc::new(|_| Ok(()));
    let installer = Installer::new(
        catalog,
        scratch.0.join("managed"),
        "windows-aarch64".into(),
        download,
        smoke,
    );
    assert!(
        matches!(installer.status("solo"), PackStatus::Unavailable(m) if m.contains("windows-aarch64"))
    );
    assert!(installer.request("solo").is_err());
    assert!(installer.request("missing").is_err());
}

#[test]
fn a_failure_waits_out_its_cooldown_unless_the_user_retries() {
    for manual in [false, true] {
        let w = World::new(good_node(), good_demo());
        w.served.lock().unwrap().remove(DEMO_URL);
        w.installer.request("demo").unwrap();
        failure(w.settle("demo"));
        let old = w.installer.cancel_target("demo").unwrap();
        let attempts = w.downloads.load(Ordering::SeqCst);
        w.installer.request("demo").unwrap();
        assert!(
            matches!(w.installer.status("demo"), PackStatus::Failed(_)),
            "still cooling down"
        );
        assert_eq!(w.downloads.load(Ordering::SeqCst), attempts);

        w.served
            .lock()
            .unwrap()
            .insert(DEMO_URL.to_owned(), good_demo().bytes);
        if manual {
            w.installer.retry();
        } else {
            if let Some(Job::Failed { retry, .. }) =
                w.installer.work.lock().unwrap().states.get_mut("demo")
            {
                *retry = Instant::now();
            }
            w.installer.request("demo").unwrap();
        }
        assert_ne!(
            w.installer.work.lock().unwrap().requests["demo"],
            old.generation
        );
        w.installer.cancel(&old);
        assert!(matches!(w.settle("demo"), PackStatus::Installed(_)));
        assert!(w.downloads.load(Ordering::SeqCst) > attempts);
    }
}

#[test]
fn older_versions_are_pruned_keeping_the_current_and_one_other() {
    let scratch = Scratch::new();
    let versions = scratch.0.join("demo");
    for name in ["1-aaa", "2-bbb", "3-ccc", "4-ddd"] {
        fs::create_dir_all(versions.join(name)).unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
    prune(&versions, &versions.join("4-ddd"));
    let mut left: Vec<_> = fs::read_dir(&versions)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(left.contains(&"4-ddd".to_owned()));
}

#[test]
fn archive_paths_must_stay_inside_the_pack_with_names_every_platform_can_hold() {
    for good in [
        "demo",
        "demo/a",
        "demo/.hidden/x-y_z.js",
        "demo/@scope/pkg/index.mjs",
    ] {
        assert!(safe_path(good), "{good}");
    }
    for bad in [
        "", "/a", "a//b", "a/./b", "a/../b", "a\\b", "a:b", "a/b.", "a/b ", "a\0b",
    ] {
        assert!(!safe_path(bad), "{bad:?}");
    }
}

fn fake_server() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().and_then(|deps| deps.parent()).unwrap();
    dir.join("examples").join(if cfg!(windows) {
        "fake_lsp.exe"
    } else {
        "fake_lsp"
    })
}

#[test]
fn cancelling_a_real_initialization_stops_and_reaps_the_smoke_process() {
    let scratch = Scratch::new();
    let log = scratch.0.join("messages.jsonl");
    let program = scratch
        .0
        .join(if cfg!(windows) { "smoke.exe" } else { "smoke" });
    fs::copy(fake_server(), &program).unwrap();
    let cancellation = Cancellation::default();
    let request = SmokeRequest {
        cancellation: cancellation.clone(),
        server: "fake".into(),
        program: program.clone(),
        args: vec![
            "--init-delay-ms".into(),
            "20000".into(),
            "--log".into(),
            log.to_string_lossy().into_owned(),
        ],
        options: None,
        cwd: scratch.0.clone(),
    };
    let (done, result) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = done.send(start_and_initialize(&request));
    });
    let until = Instant::now() + Duration::from_secs(10);
    while !fs::read_to_string(&log)
        .unwrap_or_default()
        .contains("initialize")
    {
        assert!(Instant::now() < until, "smoke did not reach initialize");
        std::thread::sleep(Duration::from_millis(10));
    }
    cancellation.0.store(true, Ordering::Release);
    let error = result
        .recv_timeout(Duration::from_secs(8))
        .expect("cancelled initialize stops within its exit grace")
        .unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    worker.join().unwrap();
    fs::remove_file(program).expect("Windows releases the staged executable after cancellation");
}

#[test]
fn a_real_server_process_is_started_and_initialized_by_the_smoke_test() {
    let scratch = Scratch::new();
    let request = |program: PathBuf, args: Vec<String>| SmokeRequest {
        cancellation: Cancellation::default(),
        server: "fake".into(),
        program,
        args,
        options: None,
        cwd: scratch.0.clone(),
    };
    start_and_initialize(&request(
        fake_server(),
        vec!["--sync".into(), "full".into()],
    ))
    .expect("the fake server completes initialize");
    let refused =
        start_and_initialize(&request(fake_server(), vec!["--init-error".into()])).unwrap_err();
    assert!(!refused.is_empty());
    let missing =
        start_and_initialize(&request(scratch.0.join("no-such-server"), Vec::new())).unwrap_err();
    assert!(missing.contains("no-such-server"), "{missing}");
}

/// Opt-in network acceptance: consumes the committed catalog and the production
/// download, verification, unpacking and initialize paths. Keeps its verified
/// packs under target so the editor acceptance tests can use the same binaries.
#[test]
#[ignore = "downloads published rust-analyzer, TypeScript and managed Node packs"]
fn real_published_server_packs_install() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let managed = root.join("target/real-lsp-validation/managed");
    let catalog =
        Arc::new(Catalog::read(&root.join("assets/lsp-servers/lsp-servers.toml")).unwrap());
    let installer = Installer::new(
        catalog.clone(),
        managed.clone(),
        super::super::lsp_catalog::platform(),
        Arc::new(http_download),
        Arc::new(start_and_initialize),
    );
    let mut receipt = serde_json::Map::new();
    let all = std::env::var_os("LOOM_ALL_SERVER_PACKS").is_some();
    let packs: Vec<String> = if all {
        catalog
            .pack_ids()
            .filter(|p| {
                catalog
                    .archive(p, &super::super::lsp_catalog::platform())
                    .is_ok()
            })
            .map(str::to_owned)
            .collect()
    } else {
        vec!["rust-analyzer".into(), "typescript-language-server".into()]
    };
    for pack in &packs {
        installer.request(pack).unwrap();
        let started = Instant::now();
        let mut previous = String::new();
        loop {
            let status = installer.status(pack);
            // Print transitions, not every polling iteration or download byte.
            let stage = std::mem::discriminant(&status);
            let stage = format!("{stage:?}");
            if stage != previous {
                println!(
                    "{pack}: {status:?} ({:.1}s)",
                    started.elapsed().as_secs_f32()
                );
                previous = stage;
            }
            match status {
                PackStatus::Installed(path) => {
                    receipt.insert(pack.clone(), json!(path));
                    break;
                }
                PackStatus::Failed(error) | PackStatus::Unavailable(error) => {
                    panic!("{pack}: {error}; all jobs: {:?}", installer.busy());
                }
                _ => {}
            }
            assert!(
                started.elapsed() < Duration::from_secs(300),
                "{pack}: {:?}",
                installer.busy()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    receipt.insert(
        "node".into(),
        json!(
            installer
                .installed_dir("node")
                .expect("managed Node installed")
        ),
    );
    // A new installer, with network forbidden, must discover the activated packs.
    let offline = Installer::new(
        catalog,
        managed,
        super::super::lsp_catalog::platform(),
        Arc::new(|_| panic!("installed packs must not access the network")),
        Arc::new(start_and_initialize),
    );
    for pack in receipt.keys() {
        assert!(matches!(offline.status(pack), PackStatus::Installed(_)));
        offline.request(pack).unwrap();
    }
    let receipt_path = root.join("target/real-lsp-validation/packs.json");
    fs::write(
        &receipt_path,
        serde_json::to_string_pretty(&receipt).unwrap(),
    )
    .unwrap();
    println!("verified published packs: {}", receipt_path.display());
}
