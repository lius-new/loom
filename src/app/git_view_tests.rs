#![allow(unused_imports)]
use crate::app::context::AppContext;
use crate::features::git::actions::*;
use crate::features::git::model::*;
use crate::features::git::*;
use crate::ui::components::input::InputState;
use crate::ui::git::confirm::{GitConfirm, GitConfirmAction};
use crate::ui::git::menu::{GitMenu, GitMenuTarget};
use crate::ui::git::panel::{CONFLICTS, STAGED, UNTRACKED};
use crate::ui::theme;
use lgui::core::*;
use lgui::prelude::*;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
mod panel_virtualization_tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::ui::git::panel::*;
    use lgui::prelude::State;

    #[test]
    fn bottom_sections_stay_pinned_and_scroll_independently_from_changes() {
        use crate::features::git::RepositoryId;
        use crate::features::git::snapshot::{GitPresentation, RepositoryPresentation};
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{
            InputEvent, Point, PointerData, UiScale, WheelDelta, dispatch_runtime_output,
        };
        use lgui::session::UiSession;
        use std::sync::{Arc, Mutex};

        let viewport = UiRect::new(0.0, 0.0, 340.0, 600.0);
        let exposed = Arc::new(Mutex::new(None::<(AppContext, State<GitStoreSnapshot>)>));
        let output = exposed.clone();
        let id = RepositoryId(1);
        let paths = (0..10_000)
            .map(|index| PathBuf::from(format!("src/file{index:05}.rs")))
            .collect::<Vec<_>>();
        let repository = RepositorySnapshot {
            id,
            worktree_root: PathBuf::from("loom"),
            git_dir: PathBuf::from("loom/.git"),
            common_dir: PathBuf::from("loom/.git"),
            head: crate::features::git::HeadState::Branch("main".into()),
            upstream: None,
            ahead: 0,
            behind: 0,
            files: Arc::new(
                paths
                    .iter()
                    .cloned()
                    .map(|path| {
                        (
                            path,
                            FileState {
                                worktree: ChangeKind::Modified,
                                ..Default::default()
                            },
                        )
                    })
                    .collect(),
            ),
            ignored: Default::default(),
            repository_state: RepositoryState::Normal,
            operation_step: None,
            head_oid: None,
            stashes: Arc::new(vec![StashInfo {
                index: 0,
                oid: "stash".into(),
                message: "WIP: search panel".into(),
            }]),
            worktrees: Arc::new(
                [("loom", "main"), ("loom-search", "feature/search")]
                    .into_iter()
                    .map(|(path, branch)| WorktreeInfo {
                        path: path.into(),
                        oid: "abc".into(),
                        branch: Some(branch.into()),
                        bare: false,
                        detached: false,
                        locked: None,
                    })
                    .collect(),
            ),
            features: Default::default(),
            generation: 0,
        };
        let mut prepared = RepositoryPresentation::default();
        prepared.sections[UNSTAGED as usize] = paths;
        let snapshot = GitStoreSnapshot {
            repositories: Arc::new([(id, repository)].into_iter().collect()),
            active_repository: Some(id),
            presentation: Arc::new(GitPresentation {
                repositories: [(id, prepared)].into_iter().collect(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let view: AppView = Arc::new(move |cx| {
            let state = AppContext::detached(cx.state_with(|| {
                let mut app = AppState::new();
                app.git.ui.hovered = true;
                app.git.model.set_history(GitHistory {
                    repository: "loom".into(),
                    head: None,
                    commits: Ok((0..40)
                        .map(|index| CommitSummary {
                            oid: format!("abc{index:05}"),
                            parents: Vec::new(),
                            author: "lius-new".into(),
                            author_email: "dev@example.com".into(),
                            timestamp: 0,
                            subject: format!("Improve editor behavior {index}"),
                            decorations: Vec::new(),
                        })
                        .collect()),
                });
                app
            }));
            let fixture = snapshot.clone();
            let store = cx.state_with(move || fixture);
            *output.lock().unwrap() = Some((state.clone(), store.clone()));
            render(
                viewport,
                crate::app::git_view::binding(&state, &store),
                store.clone().into(),
                cx.focus_handle(UiId::new("git-test-editor")),
                cx.focus_handle(UiId::new("git-test-commit")),
                UiId::new("git-test-commit"),
            )
        });
        let context = ApplicationContext::empty(Default::default());
        context
            .resources()
            .provide(lgui::icons::IconRegistration(Arc::new(
                crate::app::svg_icons(),
            )));
        let _fonts = lgui::backend::install_text_environment(
            &context,
            lgui::render_skia::skia_text_system_handle(),
        );
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let (state, store) = exposed.lock().unwrap().clone().unwrap();
        let assert_scrollbar_edge = |session: &UiSession, y: f32| {
            // Check the complete drawer, including the body's horizontal layout.
            let hit = session
                .tree()
                .hit_test(Point::new(viewport.right - 9.0, y))
                .expect("Scrollbar near the drawer's right edge");
            let thumb = session.tree().node(&hit.id).unwrap().layout_rect;
            assert_eq!(thumb.right, viewport.right - 2.0);
            assert_eq!(thumb.width(), 8.0);
        };
        assert_scrollbar_edge(&session, TOPLINE_H + 12.0);
        let find = |session: &UiSession, name: &str| {
            let node = session
                .tree()
                .nodes()
                .iter()
                .find(|node| {
                    node.semantics
                        .as_ref()
                        .and_then(|semantics| semantics.name.as_deref())
                        == Some(name)
                })
                .unwrap_or_else(|| panic!("Missing section {name}"));
            session.tree().hit_for_id(&node.id).unwrap().rect
        };
        let footer_top = viewport.bottom;
        let worktrees = find(&session, "WORKTREES");
        let stashes = find(&session, "STASHES");
        let commits = find(&session, "COMMITS");
        assert_eq!(worktrees.top, footer_top - SECTION_H * 3.0);
        assert_eq!(stashes.top, worktrees.bottom);
        assert_eq!(commits.bottom, footer_top);
        assert!(find(&session, "UNSTAGED").top < worktrees.top);
        let commit_input = find(&session, "Commit message");
        assert_eq!(commit_input.bottom + 5.0, worktrees.top);
        assert_eq!(commit_input.height(), 22.0);
        let keyed_rect = |key: &'static str| {
            session
                .tree()
                .nodes()
                .iter()
                .find(|node| node.id == UiId::new(key))
                .unwrap_or_else(|| panic!("Missing element {key}"))
                .layout_rect
        };
        let check = keyed_rect("git-commit-button-icon");
        let arrow = keyed_rect("git-commit-options-icon");
        let divider = session
            .tree()
            .nodes()
            .iter()
            .map(|node| node.layout_rect)
            .find(|rect| {
                rect.width() == 1.0
                    && rect.left >= check.right
                    && rect.right <= arrow.left
                    && rect.top >= check.top
                    && rect.bottom <= check.bottom
            })
            .expect("Divider between commit icons");
        let commit = find(&session, "Commit changes");
        let options = find(&session, "Commit options");
        assert_eq!(commit.width(), options.width());
        assert_eq!(check.left - commit.left, divider.left - check.right);
        assert_eq!(divider.left - check.right, arrow.left - divider.right);
        assert_eq!(arrow.left - divider.right, options.right - arrow.right);
        assert_eq!(options.right, viewport.right - 12.0 - 1.0);
        save_drawer_preview(&session, &context, viewport, "git-drawer-long-changes");
        assert!(
            session.tree().nodes().len() < 600,
            "Large lists must stay virtualized"
        );
        let send = |session: &mut UiSession, input| {
            dispatch_runtime_output(
                session.handle_input(input),
                &context,
                &lgui::window::WindowId::new("git-drawer-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
        };
        let collapse_sticky = |session: &mut UiSession, name: &str, filter: u8, top: f32| {
            let header = find(session, name);
            assert_eq!(header.top, top);
            let point = Point::new(header.left + 40.0, header.top + SECTION_H / 2.0);
            send(session, InputEvent::PointerMove(PointerData::mouse(point)));
            session.render_view(&view, viewport, UiScale::ONE);
            assert_eq!(state.get().git.ui.hover, Some(GitHover::Section(filter)));
            for input in [
                InputEvent::PointerDown {
                    pointer: PointerData::mouse(point),
                    button: PointerButton::Left,
                },
                InputEvent::PointerUp {
                    pointer: PointerData::mouse(point),
                    button: PointerButton::Left,
                },
            ] {
                send(session, input);
            }
            session.render_view(&view, viewport, UiScale::ONE);
            assert!(state.get().git.ui.collapsed_sections.contains(&filter));
            assert!(state.get().editor.tabs.file_drag.is_none());
        };
        send(
            &mut session,
            InputEvent::Wheel {
                point: Point::new(80.0, 150.0),
                delta: WheelDelta::lines(0.0, -4.0),
            },
        );
        session.render_view(&view, viewport, UiScale::ONE);
        assert!(state.get().git.ui.scroll > 0.0);
        assert_eq!(find(&session, "UNSTAGED").top, TOPLINE_H);
        assert_eq!(state.get().git.ui.details_scroll, 0.0);
        assert_eq!(find(&session, "WORKTREES"), worktrees);
        assert_eq!(find(&session, "COMMITS"), commits);
        assert_eq!(find(&session, "Commit message"), commit_input);
        let changes_scroll = state.get().git.ui.scroll;
        let point = Point::new(100.0, commits.top + SECTION_H / 2.0);
        for input in [
            InputEvent::PointerDown {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            },
            InputEvent::PointerUp {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            },
        ] {
            send(&mut session, input);
        }
        session.render_view(&view, viewport, UiScale::ONE);
        assert!(!state.get().git.ui.collapsed_sections.contains(&HISTORY));
        let expanded = find(&session, "COMMITS");
        let details_top = find(&session, "WORKTREES").top;
        assert_scrollbar_edge(&session, find(&session, "WORKTREES").top + 12.0);
        assert!(expanded.top >= TOPLINE_H + (footer_top - TOPLINE_H) * 0.55);
        send(
            &mut session,
            InputEvent::Wheel {
                point: Point::new(100.0, expanded.bottom + ROW_H),
                delta: WheelDelta::lines(0.0, -3.0),
            },
        );
        session.render_view(&view, viewport, UiScale::ONE);
        assert!(state.get().git.ui.details_scroll > 0.0);
        assert_eq!(state.get().git.ui.scroll, changes_scroll);
        assert_eq!(find(&session, "WORKTREES").top, details_top);
        assert_eq!(find(&session, "STASHES").top, details_top + SECTION_H);
        assert_eq!(find(&session, "COMMITS").top, details_top + SECTION_H * 2.0);
        save_drawer_preview(&session, &context, viewport, "git-drawer-sticky-commits");
        collapse_sticky(
            &mut session,
            "COMMITS",
            HISTORY,
            details_top + SECTION_H * 2.0,
        );
        assert_eq!(state.get().git.ui.details_scroll, 0.0);
        collapse_sticky(&mut session, "UNSTAGED", UNSTAGED, TOPLINE_H);

        // Multiple change sections keep separate stacked slots while scrolling.
        store.update(|snapshot| {
            let prepared = Arc::make_mut(&mut snapshot.presentation)
                .repositories
                .get_mut(&id)
                .unwrap();
            prepared.sections[STAGED as usize] = prepared.sections[UNSTAGED as usize][..3].to_vec();
        });
        let transition_scroll = 8.0 + SECTION_H + ROW_H * 3.0 + 10.0 - SECTION_H / 2.0;
        state.update(|app| {
            app.git.ui.collapsed_sections.remove(&UNSTAGED);
            app.git.ui.scroll = transition_scroll;
        });
        session.render_view(&view, viewport, UiScale::ONE);
        let incoming = find(&session, "UNSTAGED");
        assert_eq!(incoming.top, TOPLINE_H + SECTION_H);
        assert_eq!(find(&session, "STAGED").top, TOPLINE_H);
        assert_eq!(find(&session, "STAGED").bottom, incoming.top);
        state.update(|app| app.git.ui.scroll += SECTION_H);
        session.render_view(&view, viewport, UiScale::ONE);
        assert_eq!(find(&session, "STAGED").top, TOPLINE_H);
        assert_eq!(find(&session, "UNSTAGED").top, TOPLINE_H + SECTION_H);
        save_drawer_preview(&session, &context, viewport, "git-drawer-stacked-changes");
        collapse_sticky(&mut session, "STAGED", STAGED, TOPLINE_H);
        assert_eq!(find(&session, "UNSTAGED").top, TOPLINE_H + SECTION_H);
        store.update(|snapshot| {
            Arc::make_mut(&mut snapshot.presentation)
                .repositories
                .get_mut(&id)
                .unwrap()
                .sections[STAGED as usize]
                .clear();
        });

        // Worktrees use the same pinning and collapse interaction as commits.
        let original_worktrees = store.get().repositories[&id].worktrees.clone();
        store.update(|snapshot| {
            let repository = Arc::make_mut(&mut snapshot.repositories)
                .get_mut(&id)
                .unwrap();
            repository.worktrees = Arc::new(
                (0..40)
                    .map(|index| WorktreeInfo {
                        path: format!("loom-worktree-{index}").into(),
                        ..original_worktrees[1].clone()
                    })
                    .collect(),
            );
        });
        state.update(|app| {
            app.git.ui.collapsed_sections.remove(&WORKTREES);
            app.git.ui.details_scroll = 0.0;
        });
        session.render_view(&view, viewport, UiScale::ONE);
        let worktrees_top = find(&session, "WORKTREES").top;
        send(
            &mut session,
            InputEvent::Wheel {
                point: Point::new(100.0, worktrees_top + SECTION_H + ROW_H),
                delta: WheelDelta::lines(0.0, -3.0),
            },
        );
        session.render_view(&view, viewport, UiScale::ONE);
        assert!(state.get().git.ui.details_scroll > 0.0);
        save_drawer_preview(&session, &context, viewport, "git-drawer-sticky-worktrees");
        state.update(|app| {
            app.git.ui.collapsed_sections.remove(&STASHES);
            app.git.ui.collapsed_sections.remove(&HISTORY);
        });
        session.render_view(&view, viewport, UiScale::ONE);
        send(
            &mut session,
            InputEvent::Wheel {
                point: Point::new(100.0, worktrees_top + SECTION_H * 4.0),
                delta: WheelDelta::pixels(0.0, -1800.0),
            },
        );
        session.render_view(&view, viewport, UiScale::ONE);
        assert_eq!(find(&session, "WORKTREES").top, worktrees_top);
        assert_eq!(find(&session, "STASHES").top, worktrees_top + SECTION_H);
        assert_eq!(
            find(&session, "COMMITS").top,
            worktrees_top + SECTION_H * 2.0
        );
        save_drawer_preview(&session, &context, viewport, "git-drawer-stacked-details");
        collapse_sticky(&mut session, "WORKTREES", WORKTREES, worktrees_top);
        assert_eq!(state.get().git.ui.details_scroll, 0.0);
        store.update(|snapshot| {
            Arc::make_mut(&mut snapshot.repositories)
                .get_mut(&id)
                .unwrap()
                .worktrees = original_worktrees.clone();
        });

        // Visual fixtures use a short list to expose the empty space between
        // the primary changes and the sections pinned at the bottom.
        store.update(|snapshot| {
            let presentation = Arc::make_mut(&mut snapshot.presentation);
            presentation.repositories.get_mut(&id).unwrap().sections[UNSTAGED as usize].truncate(5);
        });
        state.update(|app| {
            app.git.ui.scroll = 0.0;
            app.git.ui.details_scroll = 0.0;
            app.git.ui.collapsed_sections.insert(STASHES);
            app.git.ui.collapsed_sections.insert(HISTORY);
        });
        session.render_view(&view, viewport, UiScale::ONE);
        let short_input = find(&session, "Commit message");
        assert_eq!(
            short_input.top,
            TOPLINE_H + 8.0 + SECTION_H + 5.0 * ROW_H + 10.0 + 5.0
        );
        assert!(short_input.bottom < find(&session, "WORKTREES").top);
        let point = Point::new(short_input.left + 30.0, short_input.top + PROMPT_H / 2.0);
        for input in [
            InputEvent::PointerDown {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            },
            InputEvent::PointerUp {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            },
            InputEvent::TextInput("Improve search UI".into()),
        ] {
            send(&mut session, input);
        }
        session.render_view(&view, viewport, UiScale::ONE);
        assert_eq!(state.get().git.ui.commit_input.text(), "Improve search UI");
        save_drawer_preview(&session, &context, viewport, "git-drawer-collapsed");
        state.update(|app| {
            app.git.ui.collapsed_sections.remove(&HISTORY);
        });
        session.render_view(&view, viewport, UiScale::ONE);
        save_drawer_preview(&session, &context, viewport, "git-drawer-commits");
        state.update(|app| {
            app.git.ui.collapsed_sections.insert(HISTORY);
            app.git.ui.collapsed_sections.remove(&WORKTREES);
        });
        session.render_view(&view, viewport, UiScale::ONE);
        save_drawer_preview(&session, &context, viewport, "git-drawer-worktrees");
        store.update(|snapshot| {
            Arc::make_mut(&mut snapshot.presentation)
                .repositories
                .get_mut(&id)
                .unwrap()
                .sections[UNSTAGED as usize]
                .clear();
        });
        state.update(|app| {
            app.git.ui.commit_input.clear();
            app.git.ui.commit_input.focused = false;
            app.git.ui.collapsed_sections.insert(WORKTREES);
        });
        session.render_view(&view, viewport, UiScale::ONE);
        assert_eq!(find(&session, "Commit message").top, TOPLINE_H + 8.0 + 5.0);
        save_drawer_preview(&session, &context, viewport, "git-drawer-no-changes");
        // A clean repository with only Commits must retain the same upper
        // area budget, input bounds and spacing when its history expands.
        store.update(|snapshot| {
            let repository = Arc::make_mut(&mut snapshot.repositories)
                .get_mut(&id)
                .unwrap();
            repository.worktrees = Arc::default();
            repository.stashes = Arc::default();
        });
        session.render_view(&view, viewport, UiScale::ONE);
        let empty_input = find(&session, "Commit message");
        save_drawer_preview(&session, &context, viewport, "git-drawer-clean-collapsed");
        let commits = find(&session, "COMMITS");
        let point = Point::new(commits.left + 40.0, commits.top + SECTION_H / 2.0);
        for input in [
            InputEvent::PointerDown {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            },
            InputEvent::PointerUp {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            },
        ] {
            send(&mut session, input);
        }
        session.render_view(&view, viewport, UiScale::ONE);
        assert!(!state.get().git.ui.collapsed_sections.contains(&HISTORY));
        assert_eq!(find(&session, "Commit message"), empty_input);
        let content_available = viewport.height() - TOPLINE_H - PROMPT_H;
        assert!(find(&session, "COMMITS").top >= TOPLINE_H + PROMPT_H + content_available * 0.55);
        save_drawer_preview(&session, &context, viewport, "git-drawer-clean-expanded");
        for height in [120.0, 300.0, 900.0] {
            let body = UiRect::new(0.0, TOPLINE_H, 340.0, height);
            let (changes, prompt, details) = drawer_regions(body, 5000.0, 1500.0, 3);
            assert_eq!(changes.bottom, prompt.top);
            assert_eq!(prompt.bottom, details.top);
            assert_eq!(prompt.height(), PROMPT_H);
            assert_eq!(details.bottom, body.bottom);
            assert!(changes.height() >= 0.0 && details.height() >= 0.0);
        }
        state.update(|app| {
            app.git.ui.scroll = 100.0;
            app.git.ui.details_scroll = 200.0;
        });
        crate::app::git::select_repository(&state, &store, PathBuf::from("loom"));
        assert_eq!(state.get().git.ui.scroll, 0.0);
        assert_eq!(state.get().git.ui.details_scroll, 0.0);
    }

    fn save_drawer_preview(
        session: &lgui::session::UiSession,
        context: &lgui::application::ApplicationContext,
        rect: UiRect,
        name: &str,
    ) {
        let Some(directory) = std::env::var_os("LOOM_GIT_PREVIEW_DIR") else {
            return;
        };
        use lgui::core::{PhysicalRect, UiScale};
        use lgui::renderer::{FrameInfo, FrameReason};
        let physical = PhysicalRect::new(0, 0, rect.width() as i32, rect.height() as i32);
        let dirty = [physical];
        let frame = FrameInfo::new(
            physical,
            &dirty,
            UiScale::ONE,
            FrameReason::SceneChange,
            true,
        );
        let mut surface = lgui::render_skia::SkiaSoftwareSurface::new(16 * 1024 * 1024);
        lgui::assets::backend::with_render_resources(
            context,
            lgui::assets::backend::render_resources(),
            || {
                surface.draw(
                    &session.tree().scene().project_to_physical(UiScale::ONE),
                    &frame,
                )
            },
        )
        .unwrap();
        let mut rgba = surface.pixels().to_vec();
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        image::save_buffer(
            std::path::Path::new(&directory).join(format!("{name}.png")),
            &rgba,
            rect.width() as u32,
            rect.height() as u32,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }

    #[test]
    fn worktree_delete_button_only_requests_confirmation() {
        use crate::ui::git::confirm::GitConfirmAction;
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{InputEvent, Point, PointerData, UiScale, dispatch_runtime_output};
        use lgui::session::UiSession;
        use std::sync::{Arc, Mutex};

        let exposed = Arc::new(Mutex::new(None::<AppContext>));
        let output = exposed.clone();
        let viewport = UiRect::new(0.0, 0.0, 300.0, 100.0);
        let source = PathBuf::from("worktree-ui-test-source");
        let target = PathBuf::from("worktree-ui-test-linked");
        let repository = RepositorySnapshot {
            id: crate::features::git::RepositoryId(1),
            worktree_root: source.clone(),
            git_dir: source.join(".git"),
            common_dir: source.join(".git"),
            head: crate::features::git::HeadState::Branch("main".into()),
            upstream: None,
            ahead: 0,
            behind: 0,
            files: Default::default(),
            ignored: Default::default(),
            repository_state: RepositoryState::Normal,
            operation_step: None,
            head_oid: None,
            stashes: Default::default(),
            worktrees: Default::default(),
            features: Default::default(),
            generation: 0,
        };
        let worktree = WorktreeInfo {
            path: target.clone(),
            oid: "abc".into(),
            branch: Some("feature/x".into()),
            bare: false,
            detached: false,
            locked: None,
        };
        let view: AppView = Arc::new(move |cx| {
            let source = repository.worktree_root.clone();
            let state = AppContext::detached(cx.state_with(move || {
                let mut app = AppState::new();
                app.explorer.model.restore(vec![source], Vec::new());
                app
            }));
            let store = cx.state_with(GitStoreSnapshot::default);
            *output.lock().unwrap() = Some(state.clone());
            panel(viewport, VisualStyle::default()).child(worktree_row(
                UiRect::new(0.0, 0.0, 300.0, ROW_H * 2.0),
                &worktree,
                &repository,
                false,
                true,
                true,
                crate::app::git_view::binding(&state, &store),
                store.clone().into(),
            ))
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let state = exposed.lock().unwrap().clone().unwrap();
        let context = ApplicationContext::empty(Default::default());
        let point = Point::new(282.0, ROW_H);
        for input in [
            InputEvent::PointerDown {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            },
            InputEvent::PointerUp {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            },
        ] {
            let events = session.handle_input(input);
            dispatch_runtime_output(
                events,
                &context,
                &lgui::window::WindowId::new("worktree-actions-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
            session.render_view(&view, viewport, UiScale::ONE);
        }
        let app = state.get();
        assert_eq!(app.explorer.model.roots(), std::slice::from_ref(&source));
        assert!(!app.git.model.has_pinned_repository());
        assert!(app.chrome.toast.is_none()); // The click did not also open the row.
        let confirm = app.git.ui.confirm.unwrap();
        assert!(confirm.destructive);
        assert_eq!(
            confirm.action,
            GitConfirmAction::RemoveWorktree {
                repository: source,
                path: target,
                force: false
            }
        );
    }

    #[test]
    fn scrollbar_is_separate_from_resize_handle_and_drags_outside_its_strip() {
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{InputEvent, Point, PointerData, UiScale, dispatch_runtime_output};
        use lgui::session::UiSession;
        use std::sync::{Arc, Mutex};

        for region in [ScrollRegion::Changes, ScrollRegion::Details] {
            let exposed = Arc::new(Mutex::new(None::<AppContext>));
            let output = exposed.clone();
            let viewport = UiRect::new(0.0, 0.0, 300.0, 300.0);
            let view: AppView = Arc::new(move |cx| {
                let state = AppContext::detached(cx.state_with(AppState::new));
                let store = cx.state(GitStoreSnapshot::default());
                *output.lock().unwrap() = Some(state.clone());
                let scroll = region.scroll(&state.get().git);
                panel(viewport, VisualStyle::default())
                    .child(git_vertical_scrollbar(
                        viewport,
                        1200.0,
                        scroll,
                        crate::app::git_view::binding(&state, &store),
                        region,
                    ))
                    .child(git_resize_handle(
                        viewport,
                        crate::app::git_view::binding(&state, &store),
                        false,
                    ))
            });
            let mut session = UiSession::new();
            session.render_view(&view, viewport, UiScale::ONE);
            let state = exposed.lock().unwrap().clone().unwrap();
            // Grab the thumb near the right edge without entering the resize strip.
            let point = Point::new(viewport.right - 10.0, 12.0);
            assert_eq!(session.tree().cursor_at(point), None);
            assert_eq!(
                session
                    .tree()
                    .cursor_at(Point::new(viewport.right - 4.0, 12.0)),
                Some(CursorIcon::ResizeHorizontal)
            );
            let context = ApplicationContext::empty(Default::default());
            let mut send = |input| {
                let events = session.handle_input(input);
                dispatch_runtime_output(
                    events,
                    &context,
                    &lgui::window::WindowId::new("git-drawer-scrollbar-test"),
                    |action| session.handle_default_action(action),
                    |_| {},
                );
                session.render_view(&view, viewport, UiScale::ONE);
            };
            send(InputEvent::PointerDown {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            });
            assert!(region.drag(&state.get().git).0);
            assert!(!state.get().source_control.resizing);
            let outside = PointerData::mouse(Point::new(30.0, 260.0));
            send(InputEvent::PointerMove(outside));
            assert!(region.scroll(&state.get().git) > 0.0);
            let other = match region {
                ScrollRegion::Changes => ScrollRegion::Details,
                ScrollRegion::Details => ScrollRegion::Changes,
            };
            assert_eq!(other.scroll(&state.get().git), 0.0);
            send(InputEvent::PointerUp {
                pointer: outside,
                button: PointerButton::Left,
            });
            assert!(!region.drag(&state.get().git).0);
            let released = region.scroll(&state.get().git);
            send(InputEvent::PointerMove(PointerData::mouse(point)));
            assert_eq!(region.scroll(&state.get().git), released);
        }
    }

    #[test]
    fn a_hundred_thousand_changes_only_build_the_viewport_rows() {
        let first = visible_rows(0.0, 0.0, 480.0, 100_000);
        let middle = visible_rows(0.0, 50_000.0 * ROW_H, 50_000.0 * ROW_H + 480.0, 100_000);
        let viewport_rows = (480.0 / ROW_H).ceil() as usize + 2;
        assert!(first.len() <= viewport_rows);
        assert!(middle.len() <= viewport_rows);
        assert!(middle.start > 100); // Entries after the old 100-file cap are reachable.
        let end = visible_rows(0.0, 100_000.0 * ROW_H, 100_000.0 * ROW_H + 480.0, 100_000);
        assert_eq!(end.end, 100_000);
    }
}
mod panel_action_tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::ui::git::panel::*;
    use lgui::prelude::State;

    #[test]
    fn change_rows_drag_to_editor_splits_and_buttons_do_not_arm_file_drags() {
        use crate::app::state::{FileDragSource, TabDrop};
        use crate::app::tabs;
        use crate::model::pane_layout::Direction;
        use crate::ui::tabs::PaneRegion;
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{InputEvent, Point, PointerData, UiScale, dispatch_runtime_output};
        use lgui::session::UiSession;
        use std::sync::{Arc, Mutex};

        for section in [STAGED, UNSTAGED, UNTRACKED] {
            let target = if section == STAGED {
                DiffTarget::HeadToIndex
            } else {
                DiffTarget::IndexToWorktree
            };
            let exposed = Arc::new(Mutex::new(None::<AppContext>));
            let output = exposed.clone();
            let viewport = UiRect::new(0.0, 0.0, 900.0, 400.0);
            let view: AppView = Arc::new(move |cx| {
                let state = AppContext::detached(cx.state_with(|| {
                    let mut app = AppState::new();
                    app.workspace.open_path("current.txt".into(), "keep".into());
                    app
                }));
                *output.lock().unwrap() = Some(state.clone());
                let pane = state.get().workspace.active_pane();
                let store = cx.state_with(GitStoreSnapshot::default);
                let id = cx.use_stable_id();
                let region = PaneRegion {
                    pane,
                    strip: UiRect::new(0.0, 0.0, 600.0, 30.0),
                    body: UiRect::new(0.0, 30.0, 600.0, 400.0),
                };
                let clear = state.clone();
                let moving = state.clone();
                let release = state.clone();
                panel(viewport, VisualStyle::default())
                    .on_event_capture(UiEventKind::PointerDown, move |_cx, _payload| {
                        clear.try_update(|app| app.editor.tabs.file_drag.take().is_some());
                    })
                    .on_event_capture(UiEventKind::PointerDrag, move |_cx, payload| {
                        if let UiEventPayload::PointerDrag { pointer } = payload {
                            moving.try_update(|app| {
                                tabs::update_file_drag(
                                    app,
                                    &[region],
                                    pointer.point.x,
                                    pointer.point.y,
                                )
                            });
                        }
                    })
                    .on_event_capture(UiEventKind::PointerUp, move |_cx, _payload| {
                        crate::app::explorer::finish_drawer_file_drag(&release);
                    })
                    .child(file_row(
                        UiRect::new(600.0, 0.0, 900.0, ROW_H),
                        "src/a.rs".into(),
                        false,
                        FileState::default(),
                        section,
                        target,
                        false,
                        true,
                        "repo".into(),
                        crate::app::git_view::binding(&state, &store),
                        store.clone().into(),
                        cx.focus_handle(id),
                    ))
            });
            let context = ApplicationContext::empty(Default::default());
            let mut session = UiSession::new();
            session.render_view(&view, viewport, UiScale::ONE);
            let state = exposed.lock().unwrap().clone().unwrap();
            let send = |session: &mut UiSession, input| {
                let events = session.handle_input(input);
                dispatch_runtime_output(
                    events,
                    &context,
                    &lgui::window::WindowId::new("change-drag-test"),
                    |action| session.handle_default_action(action),
                    |_| {},
                );
                session.render_view(&view, viewport, UiScale::ONE);
            };
            let down = |x, y| InputEvent::PointerDown {
                pointer: PointerData::mouse(Point::new(x, y)),
                button: PointerButton::Left,
            };
            send(&mut session, down(650.0, 10.0));
            let drag = state.get().editor.tabs.file_drag.unwrap();
            assert!(!drag.active);
            assert_eq!(
                drag.source,
                FileDragSource::Changes {
                    request: crate::features::git::DiffRequest {
                        repository_root: "repo".into(),
                        path: "src/a.rs".into(),
                        target
                    },
                    untracked: section == UNTRACKED,
                }
            );
            send(
                &mut session,
                InputEvent::PointerMove(PointerData::mouse(Point::new(649.0, 10.0))),
            );
            assert!(!state.get().editor.tabs.file_drag.unwrap().active);
            send(
                &mut session,
                InputEvent::PointerMove(PointerData::mouse(Point::new(590.0, 150.0))),
            );
            let drag = state.get().editor.tabs.file_drag.unwrap();
            assert!(drag.active);
            assert_eq!(
                drag.drop,
                Some(TabDrop::Pane {
                    pane: state.get().workspace.active_pane(),
                    split: Some(Direction::Right)
                })
            );
            // Release outside the editor cancels the drag without opening Changes.
            send(
                &mut session,
                InputEvent::PointerMove(PointerData::mouse(Point::new(800.0, 100.0))),
            );
            send(
                &mut session,
                InputEvent::PointerUp {
                    pointer: PointerData::mouse(Point::new(800.0, 100.0)),
                    button: PointerButton::Left,
                },
            );
            assert!(state.get().editor.tabs.file_drag.is_none());
            assert_eq!(state.get().workspace.pane_ids().len(), 1);
            assert_eq!(
                state.get().workspace.active_editor().unwrap().text(),
                "keep"
            );
            send(&mut session, down(885.0, 10.0));
            assert!(
                state.get().editor.tabs.file_drag.is_none(),
                "action buttons must not start a file drag"
            );
        }
    }

    #[test]
    fn commit_card_messages_wrap_at_spaces_or_between_cjk_characters() {
        let width = crate::ui::tabs::measure("aaaa bbbb", theme::UI_SIZE, 400);
        assert_eq!(
            wrap_text("aaaa bbbb cccc\n\ndddd", width),
            ["aaaa bbbb", "cccc", "", "dddd"]
        );
        let cjk = "提交信息悬停卡片显示完整内容";
        let lines = wrap_text(cjk, width);
        assert!(lines.len() > 1);
        assert_eq!(lines.concat(), cjk);
        assert!(
            lines
                .iter()
                .all(|line| crate::ui::tabs::measure(line, theme::UI_SIZE, 400) <= width)
        );
        let long = "x".repeat(40);
        assert_eq!(wrap_text(&long, width).concat(), long);
    }

    #[test]
    fn grouping_lists_root_files_then_each_folder_with_its_files() {
        let files = ["README.md", "src/a.rs", "src/ui/b.rs", "src/c.rs"].map(PathBuf::from);
        let describe = |rows: Vec<ChangeRow>| {
            rows.into_iter()
                .map(|row| match row {
                    ChangeRow::Folder { path, files } => {
                        format!("[{}] {}", path.display(), files.len())
                    }
                    ChangeRow::File { path, nested } => {
                        format!("{}{}", if nested { "  " } else { "" }, path.display())
                    }
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(describe(change_rows(&files, false)).len(), 4);
        assert_eq!(
            describe(change_rows(&files, true)),
            [
                "README.md",
                "[src] 2",
                "  src/a.rs",
                "  src/c.rs",
                "[src/ui] 1",
                "  src/ui/b.rs"
            ]
        );
    }

    #[test]
    fn paths_lead_with_the_file_name() {
        assert_eq!(
            split_path(std::path::Path::new("src/api/client.ts")),
            ("client.ts".to_owned(), Some("src/api".to_owned()))
        );
        assert_eq!(
            split_path(std::path::Path::new("README.md")),
            ("README.md".to_owned(), None)
        );
    }

    #[test]
    fn ages_are_short_and_never_negative() {
        let now = 1_000_000_000;
        assert_eq!(relative_time(now, now + 30), "now");
        assert_eq!(relative_time(now, now - 90), "1m");
        assert_eq!(relative_time(now, now - 7_200), "2h");
        assert_eq!(relative_time(now, now - 3 * 86_400), "3d");
        assert_eq!(relative_time(now, now - 14 * 86_400), "2w");
        assert_eq!(relative_time(now, now - 400 * 86_400), "1y");
    }

    #[test]
    fn history_rows_show_state_and_the_expanded_commit_files() {
        let commit = |oid: &str| CommitSummary {
            oid: oid.into(),
            parents: Vec::new(),
            author: "a".into(),
            author_email: "a@b".into(),
            timestamp: 0,
            subject: oid.into(),
            decorations: Vec::new(),
        };
        assert_eq!(
            history_rows(None, None),
            [HistoryRow::Message("Loading commits…".into())]
        );
        let history = GitHistory {
            repository: PathBuf::from("repo"),
            head: Some("b".into()),
            commits: Ok(vec![commit("b"), commit("a")]),
        };
        let expanded = ExpandedCommit {
            oid: "b".into(),
            files: Some(Ok(vec![(ChangeKind::Modified, PathBuf::from("f"))])),
        };
        assert_eq!(
            history_rows(Some(&history), Some(&expanded)),
            [
                HistoryRow::Commit(commit("b"), true),
                HistoryRow::File {
                    oid: "b".into(),
                    kind: ChangeKind::Modified,
                    path: PathBuf::from("f"),
                },
                HistoryRow::Commit(commit("a"), false),
            ]
        );
    }

    #[test]
    fn the_banner_names_the_operation_and_rebase_progress() {
        let mut repository = RepositorySnapshot {
            id: crate::features::git::RepositoryId(1),
            worktree_root: PathBuf::from("repo"),
            git_dir: PathBuf::from("repo/.git"),
            common_dir: PathBuf::from("repo/.git"),
            head: crate::features::git::HeadState::Branch("main".into()),
            upstream: None,
            ahead: 0,
            behind: 0,
            files: Default::default(),
            ignored: Default::default(),
            repository_state: RepositoryState::Normal,
            operation_step: None,
            head_oid: None,
            stashes: Default::default(),
            worktrees: Default::default(),
            features: Default::default(),
            generation: 0,
        };
        assert_eq!(operation_label(&repository), None);
        repository.repository_state = RepositoryState::MergeInProgress;
        assert_eq!(operation_label(&repository).as_deref(), Some("MERGING"));
        repository.repository_state = RepositoryState::RebaseInProgress;
        repository.operation_step = Some((2, 5));
        assert_eq!(
            operation_label(&repository).as_deref(),
            Some("REBASING 2/5")
        );
    }

    #[test]
    fn each_section_offers_its_actions() {
        assert_eq!(section_actions(STAGED), [RowAction::Unstage]);
        assert_eq!(
            section_actions(UNSTAGED),
            [RowAction::Discard, RowAction::Stage]
        );
        assert!(section_actions(CONFLICTS).is_empty());
        assert_eq!(
            file_actions(CONFLICTS),
            [
                RowAction::OpenFile,
                RowAction::KeepCurrent,
                RowAction::TakeIncoming,
                RowAction::Stage
            ]
        );
        assert_eq!(RowAction::Stage.file_name(CONFLICTS), "Mark as Resolved");
        assert_eq!(RowAction::Discard.file_name(UNTRACKED), "Delete File");
        assert_eq!(
            RowAction::Discard.section_name(UNSTAGED),
            "Discard All Changes"
        );
    }
}
mod branch_picker_tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::ui::git::branch_picker::*;
    use lgui::prelude::State;
    use std::sync::Arc;

    #[test]
    fn inline_worktree_action_targets_its_branch_without_switching() {
        use crate::ui::git::confirm::GitConfirmAction;
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{
            InputEvent, Point, PointerButton, PointerData, UiScale, dispatch_runtime_output,
        };
        use lgui::session::UiSession;
        use std::sync::Mutex;

        let exposed = Arc::new(Mutex::new(None::<AppContext>));
        let output = exposed.clone();
        let viewport = UiRect::new(0.0, 0.0, 460.0, 420.0);
        let view: AppView = Arc::new(move |cx| {
            let state = AppContext::detached(cx.state_with(|| {
                let mut app = AppState::new();
                let mut branches = sample();
                let mut occupied = branch("fix/login", false, false, None);
                occupied.worktree_path = Some(PathBuf::from("projects/loom-fix-login"));
                branches.insert(2, occupied);
                open_picker(&mut app, branches);
                let picker = app.git.ui.branch_picker.as_mut().unwrap();
                picker.repository = PathBuf::from("projects/loom");
                picker.repository_name = "loom".into();
                picker.anchor_x = 8.0;
                picker.selected = 2;
                if let BranchListing::Ready { worktrees, .. } = &mut picker.listing {
                    *worktrees = vec![
                        worktree("projects/loom", "main"),
                        worktree("projects/loom-fix-login", "fix/login"),
                        worktree("projects/loom-fix-login-review", "fix/login"),
                    ]
                    .into();
                }
                app
            }));
            let store = cx.state_with(GitStoreSnapshot::default);
            *output.lock().unwrap() = Some(state.clone());
            let app = state.get();
            let input_id = cx.use_stable_id();
            let input_focus = cx.focus_handle(input_id.clone());
            let root = panel(viewport, VisualStyle::filled(theme::c().bg));
            if let Some(picker) = app.git.ui.branch_picker.as_ref() {
                root.child(render(
                    viewport,
                    24.0,
                    picker,
                    app.git.ui.branch_picker_input.text(),
                    crate::app::git_view::binding(&state, &store),
                    store.clone().into(),
                    input_focus,
                    input_id,
                ))
            } else {
                root
            }
        });
        let context = ApplicationContext::empty(Default::default());
        let _text = lgui::backend::install_text_environment(
            &context,
            lgui::render_skia::skia_text_system_handle(),
        );
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let state = exposed.lock().unwrap().clone().unwrap();
        let node = session
            .tree()
            .nodes()
            .iter()
            .find(|node| {
                node.semantics
                    .as_ref()
                    .and_then(|semantics| semantics.name.as_deref())
                    == Some("Check out feature in a new worktree")
            })
            .expect("Selected branch has an inline worktree action");
        let rect = node.layout_rect;
        let point = Point::new(
            (rect.left + rect.right) / 2.0,
            (rect.top + rect.bottom) / 2.0,
        );
        assert!(rect.height() < ROW_H);

        if let Some(path) = std::env::var_os("LOOM_BRANCH_PICKER_PREVIEW") {
            use lgui::core::PhysicalRect;
            use lgui::icons::IconRegistration;
            use lgui::renderer::{FrameInfo, FrameReason};
            let icons = crate::app::svg_icons();
            context
                .resources()
                .provide(IconRegistration(Arc::new(icons)));
            let scale = UiScale::new(2.0);
            let mut surface = lgui::render_skia::SkiaSoftwareSurface::new(4 * 1024 * 1024);
            let frame = FrameInfo::new(
                PhysicalRect::new(0, 0, 920, 840),
                &[],
                scale,
                FrameReason::SceneChange,
                true,
            );
            lgui::assets::backend::with_render_resources(
                &context,
                lgui::RenderResources::new(),
                || {
                    surface
                        .draw(&session.tree().scene().project_to_physical(scale), &frame)
                        .unwrap();
                },
            );
            let mut pixels = surface.pixels().to_vec();
            for pixel in pixels.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
            image::save_buffer(path, &pixels, 920, 840, image::ColorType::Rgba8).unwrap();
            state.update(|app| app.git.ui.branch_picker.as_mut().unwrap().selected = 2);
            session.render_view(&view, viewport, UiScale::ONE);
        }

        for input in [
            InputEvent::PointerDown {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            },
            InputEvent::PointerUp {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            },
        ] {
            let events = session.handle_input(input);
            dispatch_runtime_output(
                events,
                &context,
                &lgui::window::WindowId::new("branch-worktree-action-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
            session.render_view(&view, viewport, UiScale::ONE);
        }
        let app = state.get();
        assert!(app.git.ui.branch_picker.is_none());
        assert_eq!(
            app.git.ui.confirm.unwrap().action,
            GitConfirmAction::CreateWorktree {
                repository: PathBuf::from("projects/loom"),
                path: PathBuf::from("projects/loom-feature"),
                checkout: Checkout::Switch("feature".into()),
            }
        );
    }

    fn worktree(path: &str, branch: &str) -> WorktreeInfo {
        WorktreeInfo {
            path: path.into(),
            oid: "abc".into(),
            branch: Some(branch.into()),
            bare: false,
            detached: false,
            locked: None,
        }
    }

    #[test]
    fn every_worktree_of_a_branch_gets_its_own_child_row() {
        let mut app = AppState::new();
        open_picker(&mut app, sample());
        let picker = app.git.ui.branch_picker.as_mut().unwrap();
        if let BranchListing::Ready { worktrees, .. } = &mut picker.listing {
            *worktrees = vec![
                worktree("/repo-feature", "feature"),
                worktree("/repo-feature-review", "feature"),
            ]
            .into();
        }
        let expanded = picker_rows(picker, "feature");
        assert_eq!(
            names(&expanded),
            [
                "[LOCAL]",
                "feature",
                "@/repo-feature",
                "@/repo-feature-review"
            ]
        );
        assert_eq!(selectable(&expanded).len(), 3);
        picker.collapsed_worktrees.insert("feature".into());
        assert_eq!(
            names(&picker_rows(picker, "feature")),
            ["[LOCAL]", "feature"]
        );
    }

    fn branch(name: &str, remote: bool, current: bool, upstream: Option<&str>) -> BranchInfo {
        BranchInfo {
            name: name.into(),
            remote,
            current,
            oid: "0".into(),
            upstream: upstream.map(Into::into),
            worktree_path: None,
        }
    }

    fn sample() -> Vec<BranchInfo> {
        vec![
            branch("feature", false, false, None),
            branch("main", false, true, Some("origin/main")),
            branch("origin/main", true, false, None),
            branch("origin/release/1.0", true, false, None),
        ]
    }

    fn names(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Header(label) => format!("[{label}]"),
                Row::Create(name) => format!("+{name}"),
                Row::Branch(branch) => branch.name.clone(),
                Row::Worktree(worktree) => format!("@{}", worktree.path.display()),
            })
            .collect()
    }

    #[test]
    fn lists_current_branch_first_then_remotes() {
        assert_eq!(
            names(&rows(&sample(), "")),
            [
                "[LOCAL]",
                "main",
                "feature",
                "[REMOTE]",
                "origin/main",
                "origin/release/1.0"
            ]
        );
    }

    #[test]
    fn filters_case_insensitively_and_offers_to_create() {
        assert_eq!(
            names(&rows(&sample(), "REL")),
            ["+REL", "[REMOTE]", "origin/release/1.0"]
        );
        // An existing name, local or remote, is not offered for creation.
        assert_eq!(names(&rows(&sample(), "feature")), ["[LOCAL]", "feature"]);
        assert_eq!(
            names(&rows(&sample(), "origin/main")),
            ["[REMOTE]", "origin/main"]
        );
        assert_eq!(names(&rows(&sample(), "no such")), Vec::<String>::new());
    }

    #[test]
    fn rejects_invalid_branch_names() {
        for name in [
            "", "-x", "a..b", "a b", "a~1", "x.lock", "a/", "/a", "@", "a@{1",
        ] {
            assert!(!valid_branch_name(name), "{name:?}");
        }
        for name in ["topic", "feature/login", "fix-1.2"] {
            assert!(valid_branch_name(name), "{name:?}");
        }
    }

    #[test]
    fn remote_rows_reuse_matching_local_branches() {
        let branches = sample();
        let remote = |name: &str| Row::Branch(branch(name, true, false, None));
        // Tracked by the current branch: nothing to do.
        assert_eq!(checkout_for(&remote("origin/main"), &branches), None);
        assert_eq!(
            checkout_for(&remote("origin/release/1.0"), &branches),
            Some(Checkout::Track("origin/release/1.0".into()))
        );
        assert_eq!(
            checkout_for(&remote("origin/feature"), &branches),
            Some(Checkout::Switch("feature".into()))
        );
        assert_eq!(
            checkout_for(&Row::Branch(branches[0].clone()), &branches),
            Some(Checkout::Switch("feature".into()))
        );
        assert_eq!(
            checkout_for(&Row::Branch(branches[1].clone()), &branches),
            None
        );
        assert_eq!(
            checkout_for(&Row::Create("topic".into()), &branches),
            Some(Checkout::Create("topic".into()))
        );
    }

    #[test]
    fn occupied_local_and_tracking_branches_open_their_worktree() {
        let mut branches = sample();
        let path = PathBuf::from("../repo-feature");
        branches[0].worktree_path = Some(path.clone());
        branches[0].upstream = Some("origin/feature".into());
        let expected = Some(Checkout::OpenWorktree {
            branch: "feature".into(),
            path,
        });
        assert_eq!(
            checkout_for(&Row::Branch(branches[0].clone()), &branches),
            expected
        );
        let remote = Row::Branch(branch("origin/feature", true, false, None));
        assert_eq!(checkout_for(&remote, &branches), expected);
        assert_eq!(worktree_checkout_for(&remote, &branches), None);
        assert_eq!(
            worktree_checkout_for(&Row::Branch(branches[1].clone()), &branches),
            None
        );
        // A remote with no local branch is available for a new tracking worktree.
        let remote = Row::Branch(branch("origin/release/1.0", true, false, None));
        assert_eq!(
            worktree_checkout_for(&remote, &branches),
            Some(Checkout::Track("origin/release/1.0".into()))
        );
    }

    fn open_picker(app: &mut AppState, branches: Vec<BranchInfo>) {
        app.git.ui.branch_picker = Some(BranchPicker {
            repository: PathBuf::from("repo"),
            repository_name: "repo".into(),
            anchor_x: 0.0,
            listing: BranchListing::Ready {
                branches: branches.into(),
                worktrees: Arc::from([]),
            },
            collapsed_worktrees: BTreeSet::new(),
            selected: 0,
            scroll: 0.0,
            query: String::new(),
            sequence: 1,
            restore_terminal: false,
        });
    }

    #[test]
    fn selection_wraps_and_resets_when_the_filter_changes() {
        let mut app = AppState::new();
        open_picker(&mut app, sample());
        move_selection(&mut app.git, -1);
        assert_eq!(app.git.ui.branch_picker.as_ref().unwrap().selected, 3);
        move_selection(&mut app.git, 1);
        assert_eq!(app.git.ui.branch_picker.as_ref().unwrap().selected, 0);
        move_selection(&mut app.git, 1);
        app.git.ui.branch_picker_input.set_text("rel");
        let picker = app.git.ui.branch_picker.as_ref().unwrap();
        assert_eq!(picker.view("rel"), (0, 0.0));
    }

    #[test]
    fn keyboard_selection_scrolls_into_view() {
        let mut app = AppState::new();
        let branches = (0..30)
            .map(|index| branch(&format!("b{index:02}"), false, index == 0, None))
            .collect();
        open_picker(&mut app, branches);
        move_selection(&mut app.git, -1);
        let picker = app.git.ui.branch_picker.as_ref().unwrap();
        assert_eq!(picker.selected, 29);
        // Header plus 30 rows, scrolled so the last row ends at the bottom.
        assert_eq!(picker.scroll, HEADER_H + 30.0 * ROW_H - MAX_LIST_H);
        move_selection(&mut app.git, 1);
        assert_eq!(app.git.ui.branch_picker.as_ref().unwrap().scroll, 0.0);
    }
}
mod confirm_tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::ui::git::confirm::render;
    use crate::ui::git::confirm::*;
    use lgui::application::{AppView, ApplicationContext};
    use lgui::core::{
        InputEvent, Point, PointerButton, PointerData, UiScale, dispatch_runtime_output,
    };
    use lgui::prelude::State;
    use lgui::session::UiSession;
    use std::sync::{Arc, Mutex};

    #[test]
    fn worktree_form_shows_destination_and_cancels_at_normal_and_narrow_widths() {
        for width in [640.0, 360.0] {
            let exposed = Arc::new(Mutex::new(None::<AppContext>));
            let output = exposed.clone();
            let viewport = UiRect::new(0.0, 0.0, width, 420.0);
            let destination = PathBuf::from(r"D:\Documents\cprojects\loom-feature-login");
            let expected_path = crate::ui::display_path(&destination);
            let view: AppView = Arc::new(move |cx| {
                let state = AppContext::detached(cx.state_with(|| {
                    let mut app = AppState::new();
                    app.git.ui.confirm = Some(GitConfirm {
                        title: "New Worktree".into(),
                        message: String::new(),
                        confirm_label: "Create",
                        destructive: false,
                        action: GitConfirmAction::CreateWorktree {
                            repository: PathBuf::from(r"D:\Documents\cprojects\loom"),
                            path: destination.clone(),
                            checkout: Checkout::Switch("feature/login".into()),
                        },
                        restore_terminal: false,
                    });
                    app
                }));
                let store = cx.state_with(GitStoreSnapshot::default);
                let dialog_id = cx.use_stable_id();
                *output.lock().unwrap() = Some(state.clone());
                let root = panel(viewport, VisualStyle::filled(theme::c().bg));
                if let Some(confirm) = state.get().git.ui.confirm {
                    root.child(render(
                        viewport,
                        &confirm,
                        crate::app::git_view::binding(&state, &store),
                        store.clone().into(),
                        dialog_id,
                    ))
                } else {
                    root
                }
            });
            let context = ApplicationContext::empty(Default::default());
            let _text = lgui::backend::install_text_environment(
                &context,
                lgui::render_skia::skia_text_system_handle(),
            );
            let mut session = UiSession::new();
            session.render_view(&view, viewport, UiScale::ONE);
            let find = |name: &str| {
                session
                    .tree()
                    .nodes()
                    .iter()
                    .find(|node| {
                        node.semantics.as_ref().and_then(|s| s.name.as_deref()) == Some(name)
                    })
                    .unwrap_or_else(|| panic!("Missing dialog control: {name}"))
            };
            let location = find("Worktree location");
            assert_eq!(
                location.semantics.as_ref().unwrap().value.as_deref(),
                Some(expected_path.as_str())
            );
            assert!(location.layout_rect.left >= viewport.left);
            assert!(
                location.layout_rect.right <= find("Choose worktree location").layout_rect.left
            );
            let cancel = find("Cancel").layout_rect;
            assert!(find("Create").layout_rect.right <= viewport.right);

            if width == 640.0
                && let Some(path) = std::env::var_os("LOOM_WORKTREE_DIALOG_PREVIEW")
            {
                use lgui::core::PhysicalRect;
                use lgui::icons::IconRegistration;
                use lgui::renderer::{FrameInfo, FrameReason};
                context
                    .resources()
                    .provide(IconRegistration(Arc::new(crate::app::svg_icons())));
                let scale = UiScale::new(2.0);
                let mut surface = lgui::render_skia::SkiaSoftwareSurface::new(8 * 1024 * 1024);
                let frame = FrameInfo::new(
                    PhysicalRect::new(0, 0, 1280, 840),
                    &[],
                    scale,
                    FrameReason::SceneChange,
                    true,
                );
                lgui::assets::backend::with_render_resources(
                    &context,
                    lgui::RenderResources::new(),
                    || {
                        surface
                            .draw(&session.tree().scene().project_to_physical(scale), &frame)
                            .unwrap();
                    },
                );
                let mut pixels = surface.pixels().to_vec();
                for pixel in pixels.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
                image::save_buffer(path, &pixels, 1280, 840, image::ColorType::Rgba8).unwrap();
            }

            let point = Point::new(
                (cancel.left + cancel.right) / 2.0,
                (cancel.top + cancel.bottom) / 2.0,
            );
            for input in [
                InputEvent::PointerDown {
                    pointer: PointerData::mouse(point),
                    button: PointerButton::Left,
                },
                InputEvent::PointerUp {
                    pointer: PointerData::mouse(point),
                    button: PointerButton::Left,
                },
            ] {
                let events = session.handle_input(input);
                dispatch_runtime_output(
                    events,
                    &context,
                    &lgui::window::WindowId::new("worktree-dialog-test"),
                    |action| session.handle_default_action(action),
                    |_| {},
                );
                session.render_view(&view, viewport, UiScale::ONE);
            }
            assert!(
                exposed
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .get()
                    .git
                    .ui
                    .confirm
                    .is_none()
            );
        }
    }
}
mod menu_tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::ui::git::menu::*;
    use crate::ui::git::panel::UNSTAGED;
    use lgui::prelude::State;

    fn labels(target: &GitMenuTarget, app: &AppState) -> Vec<String> {
        entries(target, &app.git, &GitStoreSnapshot::default())
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Item { label, .. } => Some(label),
                Entry::Separator => None,
            })
            .collect()
    }

    fn file(section: u8) -> GitMenuTarget {
        GitMenuTarget::File {
            repository: PathBuf::from("repo"),
            path: PathBuf::from("src/main.rs"),
            section,
        }
    }

    #[test]
    fn file_menus_offer_the_section_actions() {
        let app = AppState::new();
        assert!(
            labels(&file(STAGED), &app)
                .iter()
                .any(|label| label == "Unstage Changes")
        );
        assert!(
            !labels(&file(STAGED), &app)
                .iter()
                .any(|label| label == "Discard Changes")
        );
        let unstaged = labels(&file(UNSTAGED), &app);
        assert!(
            unstaged.iter().any(|label| label == "Stage Changes")
                && unstaged.iter().any(|label| label == "Discard Changes")
        );
        assert!(
            labels(&file(UNTRACKED), &app)
                .iter()
                .any(|label| label == "Delete File")
        );
        let conflicts = labels(&file(CONFLICTS), &app);
        assert!(
            conflicts.iter().any(|label| label == "Keep Current (Ours)")
                && conflicts.iter().any(|label| label == "Mark as Resolved")
        );
    }

    #[test]
    fn history_commit_menu_offers_open_and_revert() {
        let target = GitMenuTarget::HistoryCommit {
            repository: PathBuf::from("repo"),
            oid: "abc".into(),
        };
        assert_eq!(
            labels(&target, &AppState::new()),
            ["Open Commit Changes", "Revert Commit"]
        );
    }

    #[test]
    fn commit_menu_reflects_the_options() {
        let mut app = AppState::new();
        app.git.ui.commit_amend = true;
        let entries = entries(
            &GitMenuTarget::Commit,
            &app.git,
            &GitStoreSnapshot::default(),
        );
        let checked = entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Item {
                    label,
                    checked: Some(checked),
                    ..
                } => Some((label.as_str(), *checked)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(checked, [("Amend Last Commit", true), ("Sign Off", false)]);
    }
}
mod stash_switch_tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::ui::git::stash_switch::*;
    use lgui::prelude::State;

    fn files(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn names_a_few_files_and_counts_the_rest() {
        assert_eq!(describe_files(&[]), "your files");
        assert_eq!(describe_files(&files(&["a"])), "a");
        assert_eq!(describe_files(&files(&["a", "b"])), "a and b");
        assert_eq!(describe_files(&files(&["a", "b", "c"])), "a, b and c");
        assert_eq!(
            describe_files(&files(&["a", "b", "c", "d", "e"])),
            "a, b, c and 2 more files"
        );
    }
}
mod clone_tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::ui::git::clone::*;
    use lgui::prelude::State;

    #[test]
    fn long_repository_url_keeps_its_editing_tail_visible() {
        assert_eq!(visible_tail("abcdefgh", 5), "…efgh");
        assert_eq!(visible_tail("repo", 8), "repo");
    }
}
