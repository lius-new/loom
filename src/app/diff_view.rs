//! Diff input coordination; geometry and painting stay in UI.
use crate::app::context::AppContext;
use crate::app::state::AppState;
use crate::domain::identity::PaneId;
use crate::ui::chrome::diff_binding::{Binding, Command, EditorFrame, Snapshot};
use crate::ui::diff_editor::{TOOLBAR_H, code_left, row_h, scroll_metrics};
use lgui::core::UiFocusHandle;
use lgui::prelude::{Element, UiRect};
pub fn binding(state: &AppContext) -> Binding {
    let reader = state.clone();
    let writer = state.clone();
    Binding::new(
        move || {
            let app = reader.get();
            Snapshot {
                workspace: app.workspace.core,
                split: app.git.ui.split_diff,
                editor: EditorFrame {
                    diff_split: app.editor.diff_split,
                    editor_hovered: app.editor.editor_hovered,
                    editor_vertical_scrollbar_dragging: app
                        .editor
                        .editor_vertical_scrollbar_dragging,
                    editor_horizontal_scrollbar_dragging: app
                        .editor
                        .editor_horizontal_scrollbar_dragging,
                },
            }
        },
        move |c| run(&writer, c),
    )
}
pub fn run(state: &AppContext, c: Command) -> bool {
    if let Command::Open(path) = c {
        let loader = state.services().and_then(|runtime| runtime.file_loader());
        let mut opened = false;
        state.update(|app| {
            opened = crate::app::explorer::open_tree_file(app, path, 2, loader.as_ref());
        });
        return opened;
    }
    state.update(|app| match c {
        Command::Wheel(pane, dx, dy, mx, my) => {
            let (dx, dy) = if app.editor.modifiers.shift() {
                (dx + dy, 0.0)
            } else {
                (dx, dy)
            };
            let (x, y) = app.workspace.scroll(pane);
            app.workspace
                .set_scroll(pane, (x - dx).clamp(0.0, mx), (y - dy).clamp(0.0, my));
        }
        Command::Activate(pane) => {
            app.workspace.activate_pane(pane);
        }
        Command::SplitStart(pane) => app.editor.diff_split.drag = Some(pane),
        Command::SplitDrag(pane, rect, x) => {
            if app.editor.diff_split.drag == Some(pane) && rect.width() > 0.0 {
                let position = crate::ui::pane_sash::split_position(rect.left, rect.right, x);
                app.editor.diff_split.ratio = (position - rect.left) / rect.width();
            }
        }
        Command::SplitEnd => app.editor.diff_split.drag = None,
        Command::Layout(pane, split) => {
            app.git.ui.split_diff = split;
            app.workspace.activate_pane(pane);
            app.workspace.set_scroll(pane, 0.0, 0.0);
        }
        Command::Navigate(pane, starts, next) => {
            app.workspace.activate_pane(pane);
            navigate_change(app, &starts, next);
        }
        Command::Toggle(pane, request) => {
            app.workspace.activate_pane(pane);
            app.workspace.toggle_diff_section(pane, request);
        }
        Command::Open(_) => unreachable!(),
    });
    true
}
pub fn render(
    pane: PaneId,
    rect: UiRect,
    state: AppContext,
    git: crate::ui::git::commands::Binding,
    focus: UiFocusHandle,
    scrollbars: crate::ui::editor_binding::Binding,
) -> Element {
    crate::ui::diff_editor::render(pane, rect, binding(&state), git, focus, scrollbars)
}
/// Use the same window-level drag handling as the text editor, including
/// movements outside the scrollbar strip.
pub(crate) fn drag_scrollbars(app: &mut AppState, rect: UiRect, x: f32, y: f32) -> bool {
    if !app.editor.editor_vertical_scrollbar_dragging
        && !app.editor.editor_horizontal_scrollbar_dragging
    {
        return false;
    }
    let split = app.git.ui.split_diff;
    let Some(document) = app.workspace.diff(app.workspace.active_pane()) else {
        return crate::app::editor::finish_scrollbar_drag(app);
    };
    let body = UiRect::new(rect.left, rect.top + TOOLBAR_H, rect.right, rect.bottom);
    let rows = if document.sections.is_some() {
        document
            .changes_layout(
                split,
                &app.workspace.collapsed_diffs(app.workspace.active_pane()),
            )
            .row_count
    } else if split {
        document.split_rows.len()
    } else {
        document.rows.len()
    };
    let metrics = scroll_metrics(
        body,
        split,
        rows,
        document.longest_line(split),
        app.editor.diff_split.ratio,
    );
    crate::app::editor::drag_scrollbars_with_metrics(
        app,
        body,
        code_left(body, split),
        metrics,
        x,
        y,
    )
}
fn navigate_change(app: &mut AppState, starts: &[usize], next: bool) {
    if starts.is_empty() {
        return;
    }
    let (x, y) = app.workspace.active_scroll();
    let current = (y / row_h()).floor() as usize;
    let target = if next {
        starts
            .iter()
            .copied()
            .find(|start| *start > current)
            .unwrap_or(starts[0])
    } else {
        starts
            .iter()
            .rev()
            .copied()
            .find(|start| *start < current)
            .unwrap_or(*starts.last().expect("change list is not empty"))
    };
    app.workspace.set_active_scroll(x, target as f32 * row_h());
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::git::GitStoreSnapshot;
    use crate::features::git::{DiffTarget, diff_document::DiffDocument};
    use crate::ui::editor::view::SCROLLBAR_SIZE;
    use crate::ui::{
        diff_editor::{current_change_number, render_inline, render_split, split_middle},
        theme,
    };
    use lgui::prelude::State;
    use std::path::PathBuf;
    fn render(
        pane: PaneId,
        rect: UiRect,
        state: AppContext,
        store: State<GitStoreSnapshot>,
        focus: UiFocusHandle,
    ) -> Element {
        let binding = crate::app::editor_view::binding(state.clone(), store.clone(), pane, rect);
        super::render(
            pane,
            rect,
            state.clone(),
            crate::app::git_view::binding(&state, &store),
            focus,
            binding,
        )
    }

    fn sample_overview(extra_files: usize) -> DiffDocument {
        use crate::features::git::{
            ChangeDiff, ChangeGroup, DiffContent, DiffRequest, diff::parse_unified,
        };
        let file = |path: &str, group, target, patch: &str| ChangeDiff {
            path: path.into(),
            group,
            target,
            content: Ok(DiffContent::Unified(parse_unified(patch).unwrap())),
        };
        let mut files = vec![
            file(
                "src/app.rs",
                ChangeGroup::Staged,
                DiffTarget::HeadToIndex,
                "diff --git a/src/app.rs b/src/app.rs\n--- a/src/app.rs\n+++ b/src/app.rs\n@@ -1,3 +1,3 @@\n pub fn render() {\n-    open_diff(file);\n+    open_changes(repository, file);\n }\n",
            ),
            file(
                "src/ui/git_panel.rs",
                ChangeGroup::Unstaged,
                DiffTarget::IndexToWorktree,
                "diff --git a/src/ui/git_panel.rs b/src/ui/git_panel.rs\n--- a/src/ui/git_panel.rs\n+++ b/src/ui/git_panel.rs\n@@ -1,3 +1,3 @@\n fn on_click() {\n-    show_file_diff();\n+    show_all_changes();\n }\n",
            ),
            ChangeDiff {
                path: "notes/review.md".into(),
                group: ChangeGroup::Untracked,
                target: DiffTarget::IndexToWorktree,
                content: Ok(DiffContent::Untracked(
                    "# Review notes\n\nOne Changes tab for the whole repository.\n".into(),
                )),
            },
        ];
        files.extend((0..extra_files).map(|index| ChangeDiff {
            path: format!("generated/file-{index:04}.txt").into(),
            group: ChangeGroup::Untracked,
            target: DiffTarget::IndexToWorktree,
            content: Ok(DiffContent::Untracked("first\nsecond\nthird\n".into())),
        }));
        DiffDocument::from_content(
            DiffRequest {
                repository_root: "loom".into(),
                path: PathBuf::new(),
                target: DiffTarget::Changes,
            },
            DiffContent::Changes(files),
        )
    }

    #[test]
    fn changes_view_collapses_files_and_virtualizes_inline_and_split_sections() {
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{
            InputEvent, Point, PointerButton, PointerData, UiScale, dispatch_runtime_output,
        };
        use lgui::session::UiSession;
        use std::sync::{Arc, Mutex};
        for split in [false, true] {
            let exposed = Arc::new(Mutex::new(None::<AppContext>));
            let output = exposed.clone();
            let viewport = UiRect::new(0.0, 0.0, 900.0, 600.0);
            let view: AppView = Arc::new(move |cx| {
                let state = AppContext::detached(cx.state_with(|| {
                    let mut app = AppState::new();
                    app.git.ui.split_diff = split;
                    app.workspace.open_diff(sample_overview(0));
                    app
                }));
                *output.lock().unwrap() = Some(state.clone());
                let store = cx.state_with(GitStoreSnapshot::default);
                let id = cx.use_stable_id();
                let focus = cx.focus_handle(id);
                render(
                    state.get().workspace.active_pane(),
                    viewport,
                    state,
                    store,
                    focus,
                )
            });
            let context = ApplicationContext::empty(Default::default());
            let _text = lgui::backend::install_text_environment(
                &context,
                lgui::render_skia::skia_text_system_handle(),
            );
            let mut session = UiSession::new();
            session.render_view(&view, viewport, UiScale::ONE);
            let state = exposed.lock().unwrap().clone().unwrap();
            assert!(!session.tree().nodes().iter().any(|node| {
                node.layout_rect.left == viewport.right - SCROLLBAR_SIZE + 1.0
                    && node.layout_rect.width() == 4.0
                    && matches!(node.style.fill, Some(color) if color == theme::c().diff.add_mark
                        || color == theme::c().diff.del_mark)
            }), "All Changes must not show overview marks");
            let texts = || {
                session
                    .tree()
                    .nodes()
                    .iter()
                    .filter_map(|node| node.text.as_deref())
                    .collect::<Vec<_>>()
            };
            for expected in [
                "STAGED CHANGES",
                "CHANGES",
                "UNTRACKED FILES",
                "src/app.rs",
                "src/ui/git_panel.rs",
                "notes/review.md",
            ] {
                assert!(texts().contains(&expected), "Missing {expected}");
            }
            if !split && let Some(path) = std::env::var_os("LOOM_CHANGES_PREVIEW") {
                use lgui::core::PhysicalRect;
                use lgui::icons::IconRegistration;
                use lgui::renderer::{FrameInfo, FrameReason};
                context
                    .resources()
                    .provide(IconRegistration(Arc::new(crate::app::svg_icons())));
                let scale = UiScale::new(2.0);
                let mut surface = lgui::render_skia::SkiaSoftwareSurface::new(12 * 1024 * 1024);
                let frame = FrameInfo::new(
                    PhysicalRect::new(0, 0, 1800, 1200),
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
                image::save_buffer(path, &pixels, 1800, 1200, image::ColorType::Rgba8).unwrap();
            }
            let header = session
                .tree()
                .nodes()
                .iter()
                .find(|node| {
                    node.semantics.as_ref().and_then(|s| s.name.as_deref())
                        == Some("Collapse src/app.rs")
                })
                .unwrap()
                .layout_rect;
            let point = Point::new(header.left + 48.0, (header.top + header.bottom) / 2.0);
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
                    &lgui::window::WindowId::new("changes-view-test"),
                    |action| session.handle_default_action(action),
                    |_| {},
                );
                session.render_view(&view, viewport, UiScale::ONE);
            }
            let app = state.get();
            let collapsed = app.workspace.collapsed_diffs(app.workspace.active_pane());
            assert_eq!(collapsed.len(), 1);
            assert_eq!(
                collapsed.iter().next().unwrap().target,
                DiffTarget::HeadToIndex
            );
            state.update(|app| {
                app.workspace.open_diff(sample_overview(1000));
                app.workspace.set_active_scroll(0.0, 100_000.0);
            });
            session.render_view(&view, viewport, UiScale::ONE);
            let texts = session
                .tree()
                .nodes()
                .iter()
                .filter_map(|node| node.text.as_deref())
                .collect::<Vec<_>>();
            assert!(texts.contains(&"generated/file-0999.txt"));
            assert!(!texts.contains(&"src/app.rs"));
            assert!(
                session.tree().nodes().len() < 600,
                "Offscreen files must not create UI nodes"
            );
        }
    }

    #[test]
    fn inline_overview_draws_added_and_deleted_display_rows_without_scrolling() {
        use crate::features::git::diff::parse_unified;
        use lgui::application::AppView;
        use lgui::core::UiScale;
        use lgui::session::UiSession;
        use std::sync::Arc;

        // Source line 10 appears at display row 1 after the hunk header.
        let document = DiffDocument::from_unified(
            "repo".into(),
            "a.txt".into(),
            DiffTarget::IndexToWorktree,
            parse_unified(
                "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -10,3 +12,3 @@\n-old\n-older\n+new\n+newer\n same\n",
            )
            .unwrap(),
        );
        let viewport = UiRect::new(0.0, 0.0, 600.0, 300.0);
        let view: AppView = Arc::new(move |cx| {
            let state = AppContext::detached(cx.state_with(|| {
                let mut app = AppState::new();
                app.git.ui.split_diff = false;
                app.workspace.open_diff(document.clone());
                app
            }));
            let store = cx.state_with(GitStoreSnapshot::default);
            let id = cx.use_stable_id();
            render(
                state.get().workspace.active_pane(),
                viewport,
                state,
                store,
                cx.focus_handle(id),
            )
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let marks = session
            .tree()
            .nodes()
            .iter()
            .filter(|node| {
                node.layout_rect.left == viewport.right - SCROLLBAR_SIZE + 1.0
                    && node.layout_rect.width() == 4.0
                    && node.style.fill.is_some()
            })
            .map(|node| (node.layout_rect, node.style.fill.unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(
            marks,
            [
                (
                    UiRect::new(589.0, TOOLBAR_H + row_h(), 593.0, TOOLBAR_H + 3.0 * row_h()),
                    theme::c().diff.del_mark,
                ),
                (
                    UiRect::new(
                        589.0,
                        TOOLBAR_H + 3.0 * row_h(),
                        593.0,
                        TOOLBAR_H + 5.0 * row_h()
                    ),
                    theme::c().diff.add_mark,
                ),
            ]
        );
    }

    /// The text a view shows for `document`.
    fn texts_of(document: DiffDocument) -> Vec<String> {
        use lgui::application::AppView;
        use lgui::core::UiScale;
        use lgui::session::UiSession;
        use std::sync::Arc;
        let viewport = UiRect::new(0.0, 0.0, 600.0, 300.0);
        let view: AppView = Arc::new(move |cx| {
            let state = AppContext::detached(cx.state_with(|| {
                let mut app = AppState::new();
                app.workspace.open_diff(document.clone());
                app
            }));
            let store = cx.state_with(GitStoreSnapshot::default);
            let id = cx.use_stable_id();
            render(
                state.get().workspace.active_pane(),
                viewport,
                state,
                store,
                cx.focus_handle(id),
            )
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        session
            .tree()
            .nodes()
            .iter()
            .filter_map(|node| node.text.as_ref().map(|text| text.to_string()))
            .collect()
    }

    #[test]
    fn an_unresolved_conflict_that_matches_head_says_so_instead_of_no_changes() {
        use crate::features::git::diff::parse_unified;
        use crate::features::git::{ChangeDiff, ChangeGroup, DiffContent, DiffRequest};
        let empty = |target| {
            DiffDocument::from_unified(
                "repo".into(),
                "a.txt".into(),
                target,
                parse_unified("").unwrap(),
            )
        };
        let conflict = texts_of(empty(DiffTarget::HeadToWorktree));
        assert!(
            conflict
                .iter()
                .any(|text| text.starts_with("Matches the current version, still unresolved")),
            "{conflict:?}"
        );
        // An ordinary empty diff keeps its message.
        let ordinary = texts_of(empty(DiffTarget::IndexToWorktree));
        assert!(
            ordinary
                .iter()
                .any(|text| text.starts_with("No textual changes"))
        );

        // The same in the repository-wide Changes tab.
        let section = |group, target| {
            DiffDocument::from_content(
                DiffRequest {
                    repository_root: "repo".into(),
                    path: PathBuf::new(),
                    target: DiffTarget::Changes,
                },
                DiffContent::Changes(vec![ChangeDiff {
                    path: "a.txt".into(),
                    target,
                    group,
                    content: Ok(DiffContent::Unified(parse_unified("").unwrap())),
                }]),
            )
        };
        let changes = texts_of(section(ChangeGroup::Conflicts, DiffTarget::HeadToWorktree));
        assert!(
            changes.contains(
                &"Matches the current version. Mark it as resolved to finish.".to_owned()
            ),
            "{changes:?}"
        );
        let unstaged = texts_of(section(ChangeGroup::Unstaged, DiffTarget::IndexToWorktree));
        assert!(unstaged.contains(&"No textual changes.".to_owned()));
    }

    #[test]
    fn inline_diff_expands_tabs_before_rendering() {
        diff_expands_tabs_before_rendering(false);
    }

    #[test]
    fn split_diff_expands_tabs_before_rendering() {
        diff_expands_tabs_before_rendering(true);
    }

    fn diff_expands_tabs_before_rendering(split: bool) {
        use crate::features::git::diff_document::DiffDocument;
        use crate::features::git::{DiffTarget, diff::parse_unified};
        use lgui::application::AppView;
        use lgui::core::UiScale;
        use lgui::session::UiSession;
        use std::sync::Arc;

        let diff = parse_unified(
            "diff --git a/a.vue b/a.vue\n--- a/a.vue\n+++ b/a.vue\n@@ -1,2 +1,2 @@\n \tend\n-\t\t<div>\n+a\t中\tb\n",
        )
        .unwrap();
        let document = DiffDocument::from_unified(
            "repo".into(),
            "a.vue".into(),
            DiffTarget::IndexToWorktree,
            diff,
        );
        let viewport = UiRect::new(0.0, 0.0, 600.0, 100.0);
        let view: AppView = Arc::new(move |_| {
            if split {
                render_split(viewport, &document.split_rows, 0.0, 0.0, 0.5)
            } else {
                render_inline(viewport, &document.rows, 0.0, 0.0)
            }
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let rendered = session
            .tree()
            .nodes()
            .iter()
            .filter_map(|node| node.text.as_deref())
            .collect::<Vec<_>>();
        for expected in ["    end", "        <div>", "a   中  b"] {
            assert!(rendered.contains(&expected), "rendered text: {rendered:?}");
        }
        assert!(rendered.iter().all(|text| !text.contains('\t')));
    }

    #[test]
    fn inline_scrollbars_drag_and_release_outside_the_track() {
        scrollbars_drag_and_release(false);
    }

    #[test]
    fn split_scrollbars_drag_and_release_outside_the_track() {
        scrollbars_drag_and_release(true);
    }

    fn scrollbars_drag_and_release(split: bool) {
        use crate::features::git::DiffTarget;
        use crate::features::git::diff_document::DiffDocument;
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{
            InputEvent, Point, PointerButton, PointerData, UiEventKind, UiEventPayload, UiScale,
            dispatch_runtime_output,
        };
        use lgui::prelude::group;
        use lgui::session::UiSession;
        use std::sync::{Arc, Mutex};

        let exposed = Arc::new(Mutex::new(None::<AppContext>));
        let output = exposed.clone();
        let viewport = UiRect::new(0.0, 0.0, 500.0, 300.0);
        let view: AppView = Arc::new(move |cx| {
            let state = AppContext::detached(cx.state_with(|| {
                let mut app = AppState::new();
                app.git.ui.split_diff = split;
                app.workspace.open_diff(DiffDocument::added(
                    "test-repo".into(),
                    "long.txt".into(),
                    DiffTarget::HeadToWorktree,
                    &format!("{}\n", "x".repeat(200)).repeat(100),
                ));
                app
            }));
            *output.lock().unwrap() = Some(state.clone());
            let id = cx.use_stable_id();
            let focus = cx.focus_handle(id);
            let drag_state = state.clone();
            let up_state = state.clone();
            group(viewport)
                .on_event_capture(UiEventKind::PointerMove, move |_, payload| {
                    if let UiEventPayload::PointerMove { pointer } = payload {
                        drag_state.try_update(|app| {
                            drag_scrollbars(app, viewport, pointer.point.x, pointer.point.y)
                        });
                    }
                })
                .on_event_capture(UiEventKind::PointerUp, move |_, _| {
                    up_state.try_update(crate::app::editor::finish_scrollbar_drag);
                })
                .child(render(
                    PaneId::new(1),
                    viewport,
                    state,
                    cx.state(crate::features::git::GitStoreSnapshot::default()),
                    focus,
                ))
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let state = exposed.lock().unwrap().clone().unwrap();
        let context = ApplicationContext::empty(Default::default());
        let mut send = |input| {
            let events = session.handle_input(input);
            dispatch_runtime_output(
                events,
                &context,
                &lgui::window::WindowId::new("diff-scrollbar-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
            session.render_view(&view, viewport, UiScale::ONE);
        };
        let pointer = |x, y| PointerData::mouse(Point::new(x, y));
        let body_top = TOOLBAR_H;

        if split {
            let body = UiRect::new(0.0, TOOLBAR_H, viewport.right, viewport.bottom);
            let middle = split_middle(body, 0.5);
            send(InputEvent::PointerDown {
                pointer: pointer(middle, 80.0),
                button: PointerButton::Left,
            });
            assert_eq!(state.get().editor.diff_split.drag, Some(PaneId::new(1)));
            send(InputEvent::PointerMove(pointer(900.0, 80.0)));
            assert!(state.get().editor.diff_split.ratio > 0.5);
            assert!(
                (split_middle(body, state.get().editor.diff_split.ratio) - (body.right - 200.0))
                    .abs()
                    < 0.01
            );
            send(InputEvent::PointerUp {
                pointer: pointer(900.0, 80.0),
                button: PointerButton::Left,
            });
            assert_eq!(state.get().editor.diff_split.drag, None);
            let ratio = state.get().editor.diff_split.ratio;
            send(InputEvent::PointerMove(pointer(20.0, 80.0)));
            assert_eq!(state.get().editor.diff_split.ratio, ratio);
        }

        // Grab the thumb, then move away from its strip and beyond the viewport.
        send(InputEvent::PointerDown {
            pointer: pointer(496.0, body_top + 4.0),
            button: PointerButton::Left,
        });
        assert!(state.get().editor.editor_vertical_scrollbar_dragging);
        send(InputEvent::PointerMove(pointer(30.0, 500.0)));
        let bottom = state.get().workspace.active_scroll().1;
        assert!(bottom > 0.0);
        send(InputEvent::PointerUp {
            pointer: pointer(30.0, 500.0),
            button: PointerButton::Left,
        });
        assert!(!state.get().editor.editor_vertical_scrollbar_dragging);
        send(InputEvent::PointerMove(pointer(30.0, body_top)));
        assert_eq!(state.get().workspace.active_scroll().1, bottom);

        // Clicking the track jumps toward the pointer and starts a new drag.
        send(InputEvent::PointerDown {
            pointer: pointer(499.0, body_top + 40.0),
            button: PointerButton::Left,
        });
        assert!(state.get().workspace.active_scroll().1 < bottom);
        send(InputEvent::PointerUp {
            pointer: pointer(499.0, body_top + 40.0),
            button: PointerButton::Left,
        });

        let left = code_left(viewport, split);
        send(InputEvent::PointerDown {
            pointer: pointer(left + 4.0, 296.0),
            button: PointerButton::Left,
        });
        assert!(state.get().editor.editor_horizontal_scrollbar_dragging);
        send(InputEvent::PointerMove(pointer(900.0, 100.0)));
        let right = state.get().workspace.active_scroll().0;
        assert!(right > 0.0);
        send(InputEvent::PointerUp {
            pointer: pointer(900.0, 100.0),
            button: PointerButton::Left,
        });
        assert!(!state.get().editor.editor_horizontal_scrollbar_dragging);
        send(InputEvent::PointerMove(pointer(left, 100.0)));
        assert_eq!(state.get().workspace.active_scroll().0, right);
        send(InputEvent::PointerDown {
            pointer: pointer(left + 30.0, 299.0),
            button: PointerButton::Left,
        });
        assert!(state.get().workspace.active_scroll().0 < right);
        send(InputEvent::PointerUp {
            pointer: pointer(left + 30.0, 299.0),
            button: PointerButton::Left,
        });
    }

    #[test]
    fn change_counter_tracks_the_nearest_change_above_the_viewport() {
        let starts = vec![3, 10, 20];
        assert_eq!(current_change_number(&starts, 0), Some(1));
        assert_eq!(current_change_number(&starts, 10), Some(2));
        assert_eq!(current_change_number(&starts, 19), Some(2));
        assert_eq!(current_change_number(&[], 19), None);
    }

    #[test]
    fn change_navigation_wraps_in_both_directions() {
        let mut app = AppState::new();
        app.workspace.open_path("a.txt".into(), "text".into());
        navigate_change(&mut app, &[2, 8], true);
        assert_eq!(app.workspace.active_scroll().1, 2.0 * row_h());
        app.workspace.set_active_scroll(0.0, 8.0 * row_h());
        navigate_change(&mut app, &[2, 8], true);
        assert_eq!(app.workspace.active_scroll().1, 2.0 * row_h());
        navigate_change(&mut app, &[2, 8], false);
        assert_eq!(app.workspace.active_scroll().1, 8.0 * row_h());
    }
}
