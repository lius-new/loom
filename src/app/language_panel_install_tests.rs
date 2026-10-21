//! The language panel installs, cancels and resumes a server through its own buttons.
use super::context::AppContext;
use crate::features::settings::servers::{InstallPolicy, LanguageServerSetting};

struct World {
    state: AppContext,
    runtime: std::sync::Arc<super::runtime::AppRuntime>,
    downloads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    root: std::path::PathBuf,
    download_gate: std::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
}

impl World {
    fn new() -> Self {
        Self::with_blocked_download(false)
    }

    fn with_blocked_download(blocked: bool) -> Self {
        use crate::{
            domain::language::Registry,
            infrastructure::{
                lsp::manager::{Manager, Options},
                lsp_catalog::Catalog,
                lsp_install::Installer,
            },
        };
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let root = std::env::temp_dir().join(format!(
            "loom-ask-ui-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let catalog = Arc::new(
            Catalog::parse(&format!(
                r#"
schema=1
[distribution]
repository="owner/loom"
[packs.demo]
version="1.0.0"
license="MIT"
[packs.demo.platforms.any]
asset="demo-any.tar.gz"
release="lsp-servers-test"
sha256="{}"
size=100
[servers.rust-analyzer]
name="Rust Demo"
pack="demo"
command="${{pack}}/server"
[servers.rust-analyzer.root]
markers=["Cargo.toml"]
fallback="file_directory"
[[servers.rust-analyzer.languages]]
language="rust"
language_id="rust"
"#,
                "0".repeat(64)
            ))
            .unwrap(),
        );
        let servers = Arc::new(catalog.servers().unwrap());
        let downloads = Arc::new(AtomicUsize::new(0));
        let count = downloads.clone();
        let download_gate = Arc::new((std::sync::Mutex::new(!blocked), std::sync::Condvar::new()));
        let gate = download_gate.clone();
        let installer = Installer::new(
            catalog,
            root.join("managed"),
            "windows-x86_64".into(),
            Arc::new(move |_| {
                count.fetch_add(1, Ordering::SeqCst);
                let (lock, changed) = &*gate;
                let (released, _) = changed
                    .wait_timeout_while(
                        lock.lock().unwrap(),
                        std::time::Duration::from_secs(20),
                        |released| !*released,
                    )
                    .unwrap();
                assert!(*released, "test did not release the download");
                Err("test download refused".into())
            }),
            Arc::new(|_| Ok(())),
        );
        let manager = Manager::start(
            Registry::builtin(),
            servers,
            Some(installer),
            Options::default(),
            || {},
        );
        let runtime = Arc::new(super::runtime::AppRuntime::default());
        runtime.install_language_services(
            super::language_services::LanguageServices::with_manager(manager),
        );
        let mut app = super::state::AppState::new();
        app.workspace
            .open_path(root.join("a.rs"), "fn a() {}".into());
        let state = super::test_support::live(app).with_runtime(&runtime);
        let world = Self {
            state,
            runtime,
            downloads,
            root,
            download_gate,
        };
        world.sync();
        world.wait(|| {
            world
                .runtime
                .language_services()
                .is_some_and(|services| !services.infos().is_empty())
        });
        world
    }

    fn sync(&self) {
        self.runtime
            .language_services()
            .unwrap()
            .sync(&self.state.get());
    }
    fn wait(&self, check: impl Fn() -> bool) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !check() {
            assert!(
                std::time::Instant::now() < until,
                "timed out; {:?}",
                self.runtime.language_services().unwrap().infos()
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
impl Drop for World {
    fn drop(&mut self) {
        *self.download_gate.0.lock().unwrap() = true;
        self.download_gate.1.notify_all();
        self.runtime.clear_language_services();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn rendered_install_cancel_and_resume_buttons_preserve_the_cancelled_choice_under_auto_policy() {
    use crate::infrastructure::lsp::manager::Status;
    use crate::ui::language_panel::Event;
    use lgui::{
        application::{AppView, ApplicationContext},
        core::{InputEvent, Point, PointerButton, PointerData, UiScale, dispatch_runtime_output},
        prelude::UiRect,
        session::UiSession,
    };
    use std::sync::{Arc, atomic::Ordering};
    let world = World::with_blocked_download(true);
    world.state.update(|app| {
        let settings = app.preferences.values_mut();
        settings.language_servers.push(LanguageServerSetting {
            install: Some(InstallPolicy::Auto),
            ..LanguageServerSetting::named("rust-analyzer")
        });
    });
    world.sync();
    world.wait(|| world.downloads.load(Ordering::SeqCst) == 1);
    let state = world.state.clone();
    let runtime = world.runtime.clone();
    let viewport = UiRect::new(0.0, 0.0, 800.0, 600.0);
    let view: AppView = Arc::new(move |_| {
        let runtime = runtime.clone();
        crate::ui::language_panel::render(
            viewport,
            None,
            &super::chrome::language_panel_frame(&state),
            Arc::new(move |event| {
                let services = runtime.language_services().unwrap();
                match event {
                    Event::CancelInstall(target) => {
                        services.cancel_install(&target.server, target.generation)
                    }
                    Event::Install(server) => services.install(&server),
                    _ => {}
                }
            }),
        )
    });
    let context = ApplicationContext::empty(Default::default());
    let mut session = UiSession::new();
    let click = |session: &mut UiSession, label: &str| {
        session.render_view(&view, viewport, UiScale::ONE);
        let bounds = session
            .tree()
            .nodes()
            .iter()
            .find(|node| node.text.as_deref() == Some(label))
            .expect("installation action is visible")
            .paint_bounds;
        let pointer = PointerData::mouse(Point::new(
            (bounds.left + bounds.right) / 2.0,
            (bounds.top + bounds.bottom) / 2.0,
        ));
        for input in [
            InputEvent::PointerDown {
                pointer,
                button: PointerButton::Left,
            },
            InputEvent::PointerUp {
                pointer,
                button: PointerButton::Left,
            },
        ] {
            let output = session.handle_input(input);
            dispatch_runtime_output(
                output,
                &context,
                &lgui::window::WindowId::new("cancel-install-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
        }
    };
    let status = || {
        world.runtime.language_services().unwrap().infos()[0]
            .status
            .clone()
    };
    world.wait(|| matches!(status(), Status::Installing(_)));
    let old = super::chrome::language_panel_frame(&world.state).sections[0]
        .cancel_install
        .clone()
        .unwrap();
    click(&mut session, "Cancel");
    world.wait(|| status() == Status::InstallCancelling);
    world.sync();
    assert!(
        super::chrome::language_panel_frame(&world.state).sections[0]
            .cancel_install
            .is_none()
    );
    *world.download_gate.0.lock().unwrap() = true;
    world.download_gate.1.notify_all();
    world.wait(|| status() == Status::InstallCancelled);
    world.runtime.language_services().unwrap().restart();
    world.sync();
    world.wait(|| status() == Status::InstallCancelled);
    assert_eq!(world.downloads.load(Ordering::SeqCst), 1);
    click(&mut session, "Resume");
    world
        .runtime
        .language_services()
        .unwrap()
        .cancel_install(&old.server, old.generation);
    world.wait(|| matches!(status(), Status::InstallFailed(_)));
    assert_eq!(world.downloads.load(Ordering::SeqCst), 2);
}
