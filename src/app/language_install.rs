//! Installation invitation frames and commands; settings use the normal user
//! preference persistence effect, so project files cannot disable or authorize installs.
use super::context::AppContext;
use crate::{
    features::{
        language_service::install_prompt::Offer,
        settings::servers::{InstallPolicy, LanguageServerSetting},
    },
    ui::language_install::{Choice, Frame},
};
use std::collections::{BTreeMap, BTreeSet};

pub fn frame(state: &AppContext) -> Option<Frame> {
    let services = state.services()?.language_services()?;
    offers_frame(&services.install_offers())
}

pub(super) fn offers_frame(offers: &[Offer]) -> Option<Frame> {
    if offers.is_empty() {
        return None;
    }
    let languages = offers
        .iter()
        .flat_map(|offer| offer.languages.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ");
    let names = offers
        .iter()
        .map(|offer| offer.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let archives = offers
        .iter()
        .flat_map(|offer| offer.archives.iter().map(|(id, size)| (id, *size)))
        .collect::<BTreeMap<_, _>>();
    let bytes = archives
        .values()
        .fold(0u64, |total, size| total.saturating_add(*size));
    Some(Frame {
        servers: offers.iter().map(|offer| offer.server.clone()).collect(),
        title: format!("Install language servers for {languages}?"),
        detail: format!("{names} (about {:.1} MB)", bytes as f64 / 1_000_000.0),
    })
}

pub fn respond(state: &AppContext, choice: Choice, requested: Vec<String>) {
    let Some(services) = state
        .services()
        .and_then(|runtime| runtime.language_services())
    else {
        return;
    };
    // A callback from an obsolete notification cannot install or change settings for
    // servers which are no longer offered.
    let current = services.install_offers();
    let servers = requested
        .into_iter()
        .filter(|server| current.iter().any(|offer| offer.server == *server))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if servers.is_empty() {
        return;
    }
    services.dismiss_install_prompt(servers.clone());
    match choice {
        Choice::Install => {
            for server in servers {
                services.install(&server);
            }
        }
        Choice::Later => {}
        Choice::Never => {
            state.update(|app| never(&mut app.preferences.values_mut().language_servers, &servers))
        }
    }
}

fn never(settings: &mut Vec<LanguageServerSetting>, servers: &[String]) {
    for server in servers {
        if let Some(setting) = settings
            .iter_mut()
            .find(|setting| setting.server == *server)
        {
            setting.install = Some(InstallPolicy::Never);
        } else {
            settings.push(LanguageServerSetting {
                install: Some(InstallPolicy::Never),
                ..LanguageServerSetting::named(server)
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merged_invitation_counts_shared_runtime_once_and_lists_each_language() {
        let offer = |name: &str, language: &str| Offer {
            server: name.into(),
            name: name.into(),
            languages: vec![language.into()],
            archives: BTreeMap::from([("node".into(), 10_000_000), (name.into(), 2_000_000)]),
        };
        let frame =
            offers_frame(&[offer("typescript", "TypeScript"), offer("html", "HTML")]).unwrap();
        assert!(frame.title.contains("HTML, TypeScript"));
        assert_eq!(frame.detail, "typescript, html (about 14.0 MB)");
        assert_eq!(frame.servers, ["typescript", "html"]);
    }

    #[test]
    fn never_changes_only_the_requested_policy_preserving_server_options() {
        let mut settings = vec![
            LanguageServerSetting {
                args: Some(vec!["--stdio".into()]),
                env: Some(BTreeMap::from([("A".into(), "B".into())])),
                ..LanguageServerSetting::named("demo")
            },
            LanguageServerSetting::named("other"),
        ];
        let before = settings.clone();
        never(&mut settings, &["demo".into(), "new".into()]);
        assert_eq!(settings[0].args, before[0].args);
        assert_eq!(settings[0].env, before[0].env);
        assert_eq!(settings[1], before[1]);
        assert_eq!(settings[0].install, Some(InstallPolicy::Never));
        assert_eq!(settings[2].install, Some(InstallPolicy::Never));
    }

    struct World {
        state: AppContext,
        runtime: std::sync::Arc<super::super::runtime::AppRuntime>,
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
            let download_gate =
                Arc::new((std::sync::Mutex::new(!blocked), std::sync::Condvar::new()));
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
            let runtime = Arc::new(super::super::runtime::AppRuntime::default());
            runtime.install_language_services(
                super::super::language_services::LanguageServices::with_manager(manager),
            );
            let mut app = super::super::state::AppState::new();
            app.workspace
                .open_path(root.join("a.rs"), "fn a() {}".into());
            let state = super::super::test_support::live(app).with_runtime(&runtime);
            let world = Self {
                state,
                runtime,
                downloads,
                root,
                download_gate,
            };
            world.sync();
            world.wait(|| frame(&world.state).is_some());
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
    fn rendered_install_cancel_and_resume_buttons_preserve_the_cancelled_choice_under_auto_policy()
    {
        use crate::infrastructure::lsp::manager::Status;
        use crate::ui::language_panel::Event;
        use lgui::{
            application::{AppView, ApplicationContext},
            core::{
                InputEvent, Point, PointerButton, PointerData, UiScale, dispatch_runtime_output,
            },
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
                &super::super::chrome::language_panel_frame(&state),
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
        let old = super::super::chrome::language_panel_frame(&world.state).sections[0]
            .cancel_install
            .clone()
            .unwrap();
        click(&mut session, "Cancel");
        world.wait(|| status() == Status::InstallCancelling);
        world.sync();
        assert!(
            super::super::chrome::language_panel_frame(&world.state).sections[0]
                .cancel_install
                .is_none()
        );
        *world.download_gate.0.lock().unwrap() = true;
        world.download_gate.1.notify_all();
        world.wait(|| status() == Status::InstallCancelled);
        world.runtime.language_services().unwrap().restart();
        world.sync();
        world.wait(|| status() == Status::InstallCancelled);
        assert!(frame(&world.state).is_none());
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

    #[test]
    fn later_is_session_only_never_is_a_user_preference_and_manual_install_still_works() {
        use std::sync::atomic::Ordering;
        for choice in [Choice::Later, Choice::Never] {
            let world = World::new();
            let shown = frame(&world.state).unwrap();
            respond(&world.state, choice, shown.servers.clone());
            world.sync();
            world.wait(|| frame(&world.state).is_none());
            let settings = world.state.get().settings();
            assert_eq!(
                crate::features::settings::servers::install_policy(
                    &settings.language_servers,
                    "rust-analyzer"
                ),
                if choice == Choice::Never {
                    InstallPolicy::Never
                } else {
                    InstallPolicy::Ask
                }
            );
            assert_eq!(world.downloads.load(Ordering::SeqCst), 0);
            world.runtime.language_services().unwrap().restart();
            world.sync();
            std::thread::sleep(std::time::Duration::from_millis(80));
            assert!(frame(&world.state).is_none());
            respond(&world.state, Choice::Install, shown.servers);
            assert_eq!(
                world.downloads.load(Ordering::SeqCst),
                0,
                "stale banner actions do nothing"
            );
            world
                .runtime
                .language_services()
                .unwrap()
                .install("rust-analyzer");
            world.wait(|| world.downloads.load(Ordering::SeqCst) == 1);
        }
    }

    #[test]
    fn rendered_invitation_buttons_dispatch_without_blocking_the_editor_or_taking_focus() {
        use lgui::{
            application::{AppView, ApplicationContext},
            core::{
                EventPolicy, InputEvent, Point, PointerButton, PointerData, UiScale,
                dispatch_runtime_output,
            },
            prelude::{UiRect, VisualStyle, panel},
            session::UiSession,
        };
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        // The dismiss control is a vector icon button, so it is found by its name.
        let is_close = |node: &lgui::core::UiNode| {
            node.semantics.as_ref().and_then(|s| s.name.as_deref())
                == Some("Dismiss language server installation")
        };
        for (label, choice) in [
            ("×", Choice::Later),
            ("Not now", Choice::Later),
            ("Don't ask again", Choice::Never),
            ("Install", Choice::Install),
        ] {
            for width in [260.0, 900.0] {
                let world = World::new();
                let state = world.state.clone();
                let outside = Arc::new(AtomicUsize::new(0));
                let clicks = outside.clone();
                let viewport = UiRect::new(0.0, 0.0, width, 400.0);
                let status_rect = UiRect::new(0.0, 376.0, width, 400.0);
                let status = crate::ui::chrome::StatusFrame {
                    terminal_open: false,
                    pending: vec![],
                    file: None,
                    source_control: false,
                    diff_loading: false,
                    vim: None,
                    language: vec![crate::ui::chrome::StatusNote {
                        text: "Rust Demo: not installed".into(),
                        level: crate::ui::chrome::NoteLevel::Quiet,
                    }],
                };
                let view: AppView = Arc::new(move |_| {
                    let clicks = clicks.clone();
                    let state = state.clone();
                    let status = status.clone();
                    let background = panel(viewport, VisualStyle::default())
                        .event_policy(EventPolicy::INTERACTIVE)
                        .on_pointer_down_with_button(move |cx, _, _| {
                            clicks.fetch_add(1, Ordering::SeqCst);
                            cx.stop_propagation();
                        })
                        .child(crate::ui::statusbar::render(
                            status_rect,
                            crate::ui::chrome::StatusBinding::new(move || status.clone(), |_| {}),
                        ));
                    match frame(&state) {
                        Some(frame) => background.child(crate::ui::language_install::render(
                            viewport,
                            &frame,
                            Arc::new(move |choice, servers| respond(&state, choice, servers)),
                        )),
                        None => background,
                    }
                });
                let context = ApplicationContext::empty(Default::default());
                let mut session = UiSession::new();
                session.render_view(&view, viewport, UiScale::ONE);
                let title = session
                    .tree()
                    .nodes()
                    .iter()
                    .find(|node| {
                        node.text
                            .as_deref()
                            .is_some_and(|text| text.starts_with("Install language"))
                    })
                    .unwrap()
                    .paint_bounds;
                assert!(
                    title.top > viewport.height() / 2.0,
                    "notification stays near the status bar: {title:?}"
                );
                let close = session
                    .tree()
                    .nodes()
                    .iter()
                    .find(|node| is_close(node))
                    .unwrap()
                    .paint_bounds;
                assert!(close.bottom < status_rect.top);
                if width > 420.0 {
                    assert!(
                        (close.right - (viewport.right - 8.0 - 6.0)).abs() < 1.0,
                        "notification sits in the window's right corner: {close:?}"
                    );
                }
                let outside_point = PointerData::mouse(Point::new(width / 2.0, 32.0));
                let output = session.handle_input(InputEvent::PointerDown {
                    pointer: outside_point,
                    button: PointerButton::Left,
                });
                dispatch_runtime_output(
                    output,
                    &context,
                    &lgui::window::WindowId::new("ask-test"),
                    |action| session.handle_default_action(action),
                    |_| {},
                );
                let output = session.handle_input(InputEvent::PointerUp {
                    pointer: outside_point,
                    button: PointerButton::Left,
                });
                dispatch_runtime_output(
                    output,
                    &context,
                    &lgui::window::WindowId::new("ask-test"),
                    |action| session.handle_default_action(action),
                    |_| {},
                );
                assert_eq!(outside.load(Ordering::SeqCst), 1);
                let focused = session.runtime().interaction_state().focused.clone();
                let bounds = session
                    .tree()
                    .nodes()
                    .iter()
                    .find(|node| {
                        if label == "×" {
                            is_close(node)
                        } else {
                            node.text.as_deref() == Some(label)
                        }
                    })
                    .unwrap()
                    .paint_bounds;
                assert!(
                    bounds.left >= viewport.left && bounds.right <= viewport.right,
                    "{bounds:?}"
                );
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
                        &lgui::window::WindowId::new("ask-test"),
                        |action| session.handle_default_action(action),
                        |_| {},
                    );
                }
                world.sync();
                world.wait(|| frame(&world.state).is_none());
                assert_eq!(session.runtime().interaction_state().focused, focused);
                assert_eq!(
                    outside.load(Ordering::SeqCst),
                    1,
                    "notification click did not reach editor"
                );
                if choice == Choice::Never {
                    assert_eq!(
                        world.state.get().preferences.values().language_servers[0].install,
                        Some(InstallPolicy::Never)
                    );
                }
                if choice == Choice::Install {
                    world.wait(|| world.downloads.load(Ordering::SeqCst) == 1);
                }
            }
        }
    }
}
