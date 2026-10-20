use super::*;
use crate::app::context::AppContext;
use crate::domain::identity::PaneId;
use crate::features::git::{GitStoreSnapshot, LineChange};
use crate::model::text::TextBuffer;
use lgui::core::{CursorIcon, KeyState, PointerButton, UiId};
use lgui::core::{KeyModifiers, KeyboardEvent, LogicalKey};
use lgui::prelude::State;
use lgui::prelude::{Element, group};
use lgui::services::{Clipboard, ClipboardError};
use std::collections::BTreeMap;
use std::sync::Mutex;

#[test]
fn layout_cache_reuses_geometry_across_scroll_and_resize_with_correct_hit_testing() {
    let context = lgui::application::ApplicationContext::empty(Default::default());
    let _fonts = lgui::backend::install_text_environment(
        &context,
        lgui::render_skia::skia_text_system_handle(),
    );
    clear_layout_cache();
    let text = "ab\t中e\u{301}😀";
    let a = layout_line_at_column(text, 30.0, 40.0, 800.0, 0).unwrap();
    let b = layout_line_at_column(text, -200.0, 180.0, 420.0, 4).unwrap();
    assert!(std::sync::Arc::ptr_eq(&a.layout, &b.layout));
    for index in [0, 1, 2, 3, 4, 6, 7] {
        let x = caret_x(Some(&a), index).unwrap();
        let shifted = caret_x(Some(&b), index).unwrap();
        assert!((shifted - x + 230.0).abs() < 0.01);
        assert_eq!(hit_column(&a, x + 0.01, 45.0), index);
        assert_eq!(hit_column(&b, shifted + 0.01, 185.0), index);
    }
    let different_tabs = layout_line_at_column(text, 30.0, 40.0, 800.0, 1).unwrap();
    assert!(!std::sync::Arc::ptr_eq(&a.layout, &different_tabs.layout));
    assert_eq!(layout_cache_stats().0, 1);
}

#[test]
fn layout_cache_evicts_individual_cold_entries_without_clearing_hot_geometry() {
    let context = lgui::application::ApplicationContext::empty(Default::default());
    let _fonts = lgui::backend::install_text_environment(
        &context,
        lgui::render_skia::skia_text_system_handle(),
    );
    clear_layout_cache();
    let hot = layout_line("hot retained line", 0.0, 0.0, 800.0).unwrap();
    for i in 0..2100 {
        let text = format!("{i}:{}", "cold entry ".repeat(10));
        layout_line(&text, 0.0, 0.0, 800.0).unwrap();
        let reused = layout_line("hot retained line", 0.0, 0.0, 800.0).unwrap();
        assert!(std::sync::Arc::ptr_eq(&hot.layout, &reused.layout));
    }
    let (_, _, evictions, bytes, len) = layout_cache_stats();
    assert!(evictions > 0);
    assert!(bytes <= 8 * 1024 * 1024);
    assert!(len <= 2048);
}

#[test]
fn multi_megabyte_single_line_windows_and_clicks_are_bounded() {
    let buffer = TextBuffer::new("a".repeat(8 * 1024 * 1024));
    let rect = UiRect::new(0.0, 0.0, 800.0, 400.0);
    let left = 80.0;
    let metrics = scroll_metrics(rect, left, &buffer);
    let char_w = fonts::editor().char_w;
    for column in [0, 1000, 1_000_000, 8 * 1024 * 1024 - 100] {
        let sx = column as f32 * char_w;
        let window = line_window(&buffer, 0, sx, 720.0);
        assert!(
            window.len() < 256,
            "only viewport text is submitted for shaping"
        );
        assert!(window.contains(&column));
        let offset = cursor_offset_from_point(
            &buffer,
            rect,
            left,
            metrics,
            sx,
            0.0,
            left + char_w * 4.0,
            10.0,
        );
        assert_eq!(offset, column + 4);
    }
    assert_eq!(
        buffer.len(),
        8 * 1024 * 1024,
        "windowing must preserve all file contents"
    );
}

#[test]
fn long_unicode_windows_remain_on_grapheme_boundaries_near_the_end() {
    let buffer = TextBuffer::new("中e\u{301}👩‍💻\t😀".repeat(100_000));
    let columns = buffer.line_display_width(0);
    for column in [0, columns / 2, columns - 20] {
        let window = line_window(&buffer, 0, column as f32 * fonts::editor().char_w, 700.0);
        assert!(window.len() < 4096);
        assert!(buffer.is_grapheme_boundary(window.start));
        assert!(buffer.is_grapheme_boundary(window.end));
    }
}

#[test]
fn viewport_windows_bound_medium_and_long_lines_without_a_byte_threshold() {
    let width = 720.0;
    let char_w = fonts::editor().char_w;
    for len in [512, 3245, 4095, 4096, 4097, 100_000] {
        let buffer = TextBuffer::new("x".repeat(len));
        for column in [0, len / 2, len - 30] {
            let scroll = column as f32 * char_w;
            let range = line_window(&buffer, 0, scroll, width);
            assert!(range.len() <= (width / char_w).ceil() as usize + 192);
            assert!(range.contains(&column));
        }
        assert_eq!(buffer.len(), len);
    }
}

#[test]
fn real_layout_keeps_scrolled_carets_hits_ime_and_selection_in_agreement() {
    use lgui::{
        application::{AppView, ApplicationContext},
        core::UiScale,
        session::UiSession,
    };
    use std::sync::Arc;
    let context = ApplicationContext::empty(Default::default());
    let _fonts = lgui::backend::install_text_environment(
        &context,
        lgui::render_skia::skia_text_system_handle(),
    );
    fonts::set(&fonts::FontSettings::EDITOR, &fonts::FontSettings::TERMINAL);
    for source in [
        "const item={value:123};function get(a){return a.value;}".repeat(65),
        "中e\u{301}👩‍💻\tabc123 ".repeat(180),
    ] {
        let mut app = document(&source);
        app.editor.focused = true;
        let rect = UiRect::new(0.0, 0.0, 800.0, 400.0);
        let left = fonts::editor().gutter_w() + theme::CODE_PAD;
        let char_w = fonts::editor().char_w;
        let buffer = app.workspace.active_editor().unwrap();
        let start = buffer.offset_at_display_column(0, 1000);
        let end = buffer.offset_at_display_column(0, 1020);
        app.workspace
            .active_editor_mut()
            .unwrap()
            .select_range(start..end);
        app.workspace.set_active_scroll(970.0 * char_w, 0.0);
        let initial = Arc::new(Mutex::new(Some(app)));
        let exposed = Arc::new(Mutex::new(None));
        let output = exposed.clone();
        let runtime = Arc::new(crate::app::runtime::AppRuntime::default());
        let view_runtime = runtime.clone();
        let tasks = crate::app::test_support::UiTasks::default();
        let application = tasks.application();
        let view: AppView = Arc::new(move |cx| {
            let state =
                AppContext::detached(cx.state_with(|| initial.lock().unwrap().take().unwrap()))
                    .with_runtime(&view_runtime);
            if view_runtime.highlighter().is_none() {
                view_runtime.install_highlighter(crate::app::syntax::Highlighter::new(
                    state.clone(),
                    application.clone(),
                ));
            }
            let git = cx.state_with(GitStoreSnapshot::default);
            *output.lock().unwrap() = Some(state.clone());
            let id = UiId::new("bounded-editor-test");
            let focus = cx.focus_handle(id.clone());
            render(PaneId::new(1), rect, state, git, id, focus)
        });
        let mut session = UiSession::new();
        session.render_view(&view, rect, UiScale::ONE);
        let state: AppContext = exposed.lock().unwrap().clone().unwrap();
        let before_ime = session
            .tree()
            .node(&UiId::new("bounded-editor-test"))
            .unwrap()
            .ime_cursor_rect;
        let before_selection = session
            .tree()
            .nodes()
            .iter()
            .find(|node| {
                node.style == lgui::prelude::VisualStyle::filled(theme::c().accent).alpha(55)
            })
            .unwrap()
            .layout_rect;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while tasks.pending_ids().is_empty() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        tasks.run_all();
        assert!(state.get().editor.syntax_revision > 0);
        session.render_view(&view, rect, UiScale::ONE);
        assert_eq!(
            session
                .tree()
                .node(&UiId::new("bounded-editor-test"))
                .unwrap()
                .ime_cursor_rect,
            before_ime
        );
        assert_eq!(
            session
                .tree()
                .nodes()
                .iter()
                .find(|node| node.style
                    == lgui::prelude::VisualStyle::filled(theme::c().accent).alpha(55))
                .unwrap()
                .layout_rect,
            before_selection
        );
        let app = state.get();
        let buffer = app.workspace.active_editor().unwrap();
        let sx = app.workspace.active_scroll().0;
        let metrics = scroll_metrics(rect, left, &buffer);
        let range = line_window(&buffer, 0, sx, metrics.viewport_w);
        let initial_column = buffer.display_column(range.start);
        let origin = left + initial_column as f32 * char_w;
        let layout = layout_line_at_column(
            &buffer.text()[range.clone()],
            origin,
            0.0,
            left + metrics.content_w,
            initial_column,
        )
        .unwrap();
        for offset in [start, end] {
            let chars = buffer.text()[range.start..offset].chars().count();
            let x = caret_x(Some(&layout), chars).unwrap();
            let hit = cursor_offset_from_point(
                &buffer,
                rect,
                left,
                metrics,
                sx,
                0.0,
                x - sx + 0.05,
                fonts::editor().line_h / 2.0,
            );
            assert_eq!(hit, offset);
        }
        let cursor = buffer.cursor();
        let x = caret_x(
            Some(&layout),
            buffer.text()[range.start..cursor].chars().count(),
        )
        .unwrap();
        let ime = session
            .tree()
            .node(&UiId::new("bounded-editor-test"))
            .unwrap()
            .ime_cursor_rect
            .unwrap();
        assert!((ime.left - (x - sx)).abs() < 0.01);
        let selection = session
            .tree()
            .nodes()
            .iter()
            .find(|node| {
                node.style == lgui::prelude::VisualStyle::filled(theme::c().accent).alpha(55)
            })
            .unwrap();
        let expected_left = caret_x(
            Some(&layout),
            buffer.text()[range.start..start].chars().count(),
        )
        .unwrap();
        let expected_right = caret_x(
            Some(&layout),
            buffer.text()[range.start..end].chars().count(),
        )
        .unwrap();
        assert!((selection.layout_rect.width() - (expected_right - expected_left)).abs() < 0.01);
        assert_eq!(buffer.selected_text(), Some(&source[start..end]));
        drop(app);
        state.update(|app| {
            let mut editor = app.workspace.active_editor_mut().unwrap();
            let end = editor.len();
            editor.select_range(0..end);
        });
        session.render_view(&view, rect, UiScale::ONE);
        let selection = session
            .tree()
            .nodes()
            .iter()
            .find(|node| {
                node.style == lgui::prelude::VisualStyle::filled(theme::c().accent).alpha(55)
            })
            .unwrap();
        assert!(
            selection.layout_rect.width() >= metrics.viewport_w,
            "a selection spanning both crop edges must cover the entire visible row"
        );
        assert_eq!(
            state
                .get()
                .workspace
                .active_editor()
                .unwrap()
                .selected_text(),
            Some(source.as_str())
        );
    }
}

#[test]
#[ignore = "manual large-file rendering performance baseline"]
fn large_file_interaction_baseline() {
    use lgui::{application::AppView, core::UiScale, session::UiSession};
    use std::sync::Arc;
    let rect = UiRect::new(0.0, 0.0, 1000.0, 600.0);
    for (name, contents) in [
        ("8 MiB single line", "a".repeat(8 * 1024 * 1024)),
        ("Unicode single line", "中文e\u{301}😀\t".repeat(500_000)),
        (
            "many short lines",
            "{\"record\": \"中文\", \"value\": 12345}\n".repeat(500_000),
        ),
    ] {
        let prepare = std::time::Instant::now();
        let mut app = document(&contents);
        app.editor.focused = true;
        let prepare = prepare.elapsed();
        let exposed = Arc::new(Mutex::new(None));
        let output = exposed.clone();
        let initial = Arc::new(Mutex::new(Some(app)));
        let view: AppView = Arc::new(move |cx| {
            let state =
                AppContext::detached(cx.state_with(|| initial.lock().unwrap().take().unwrap()));
            let git = cx.state(GitStoreSnapshot::default());
            *output.lock().unwrap() = Some(state.clone());
            let id = UiId::new("large-file-benchmark");
            let focus = cx.focus_handle(id.clone());
            render(PaneId::new(1), rect, state, git, id, focus)
        });
        let mut session = UiSession::new();
        session.render_view(&view, rect, UiScale::ONE);
        let state: AppContext = exposed.lock().unwrap().clone().unwrap();
        let start = std::time::Instant::now();
        for i in 0..100 {
            state.update(|app| {
                let buffer = app.workspace.active_editor().unwrap();
                let left = rect.left + fonts::editor().gutter_w() + theme::CODE_PAD;
                let metrics = scroll_metrics(rect, left, &buffer);
                let sx = metrics.max_x * (i as f32 / 100.0);
                let offset = cursor_offset_from_point(
                    &buffer,
                    rect,
                    left,
                    metrics,
                    sx,
                    0.0,
                    left + 20.0,
                    10.0,
                );
                app.workspace
                    .active_editor_mut()
                    .unwrap()
                    .set_cursor(offset);
                app.workspace.set_active_scroll(sx, 0.0);
            });
            session.render_view(&view, rect, UiScale::ONE);
        }
        eprintln!(
            "{name}: {:.2} MiB, prepare {:?}, 100 viewport renders and clicks {:?}",
            contents.len() as f64 / (1024.0 * 1024.0),
            prepare,
            start.elapsed()
        );
        let edits = std::time::Instant::now();
        for _ in 0..20 {
            state.update(|app| {
                let mut editor = app.workspace.active_editor_mut().unwrap();
                let end = editor.len();
                editor.set_cursor(end);
                editor
                    .replace_selection("x", crate::model::text::EditKind::Other)
                    .unwrap();
            });
            session.render_view(&view, rect, UiScale::ONE);
        }
        eprintln!(
            "{name}: 20 end-of-file edits and renders {:?}",
            edits.elapsed()
        );
        state.update(|app| {
            assert!(app.workspace.is_dirty(app.workspace.active().unwrap()));
            for _ in 0..20 {
                assert!(app.workspace.active_editor_mut().unwrap().undo().is_some());
            }
            assert!(!app.workspace.is_dirty(app.workspace.active().unwrap()));
        });
        assert_eq!(
            state.get().workspace.active_editor().unwrap().text(),
            contents
        );
    }
}

#[test]
fn overview_marks_join_adjacent_lines_of_the_same_change() {
    let changes = BTreeMap::from([
        (1, LineChange::Added),
        (2, LineChange::Added),
        (3, LineChange::Modified),
        (5, LineChange::Modified),
        (6, LineChange::Modified),
        (10, LineChange::Deleted),
        // A deletion after the last line marks the end of the file.
        (11, LineChange::Deleted),
    ]);
    let runs = change_marks(&changes, 10)
        .iter()
        .map(|mark| (mark.first, mark.lines, mark.change))
        .collect::<Vec<_>>();
    assert_eq!(
        runs,
        [
            (0, 2, LineChange::Added),
            (2, 1, LineChange::Modified),
            (4, 2, LineChange::Modified),
            (9, 1, LineChange::Deleted),
        ]
    );
}

#[derive(Default)]
struct TestClipboard {
    text: Mutex<Option<String>>,
    fail: bool,
}
impl Clipboard for TestClipboard {
    fn read_text(&self) -> Result<Option<String>, ClipboardError> {
        if self.fail {
            Err(ClipboardError::AccessDenied)
        } else {
            Ok(self.text.lock().unwrap().clone())
        }
    }
    fn write_text(&self, text: &str) -> Result<(), ClipboardError> {
        if self.fail {
            return Err(ClipboardError::AccessDenied);
        }
        *self.text.lock().unwrap() = Some(text.to_owned());
        Ok(())
    }
}
fn document(contents: &str) -> AppState {
    let mut app = AppState::new();
    app.workspace
        .open_path("editing-test.txt".into(), contents.into());
    app
}

/// Exercise real lgui hit testing, focus and event dispatch without opening a native window.
#[test]
fn editor_events_route_drag_selection_shortcuts_and_ime_commit() {
    use lgui::application::{AppView, ApplicationContext};
    use lgui::core::{ImeEvent, InputEvent, Point, PointerData, UiScale, dispatch_runtime_output};
    use lgui::session::UiSession;
    use std::sync::Arc;
    let exposed = Arc::new(Mutex::new(None::<AppContext>));
    let output = exposed.clone();
    let viewport = UiRect::new(0.0, 0.0, 500.0, 300.0);
    let app = ApplicationContext::empty(Default::default());
    let view_app = app.clone();
    let view: AppView = Arc::new(move |cx| {
        let state = AppContext::detached(cx.state(document("hello world\nsecond")));
        let git_store = cx.state(GitStoreSnapshot::default());
        *output.lock().unwrap() = Some(state.clone());
        let id = cx.use_stable_id();
        let focus = cx.focus_handle(id.clone());
        // Shortcuts reach the editor through the keymap, as in the app root.
        let env = crate::app::key_actions::KeyEnv {
            state: state.clone(),
            git_store: git_store.clone(),
            application: view_app.clone(),
            editor_focus: focus.clone(),
            editor_rect: viewport,
            terminal: None,
        };
        crate::app::key_actions::attach(
            group(viewport).child(render(
                PaneId::new(1),
                viewport,
                state,
                git_store,
                id,
                focus,
            )),
            env,
        )
    });
    let mut session = UiSession::new();
    session.render_view(&view, viewport, UiScale::ONE);
    let state = exposed.lock().unwrap().clone().unwrap();
    let mut send = |input| {
        let events = session.handle_input(input);
        let mut prevented = false;
        dispatch_runtime_output(
            events,
            &app,
            &lgui::window::WindowId::new("editor-test"),
            |action| session.handle_default_action(action),
            |cx| prevented |= cx.default_prevented(),
        );
        session.render_view(&view, viewport, UiScale::ONE);
        prevented
    };
    let left = fonts::editor().gutter_w() + theme::CODE_PAD;
    let pointer =
        |column: f32| PointerData::mouse(Point::new(left + column * fonts::editor().char_w, 5.0));
    send(InputEvent::PointerDown {
        pointer: pointer(0.0),
        button: PointerButton::Left,
    });
    send(InputEvent::PointerMove(pointer(5.0)));
    send(InputEvent::PointerUp {
        pointer: pointer(5.0),
        button: PointerButton::Left,
    });
    assert_eq!(
        state
            .get()
            .workspace
            .active_editor()
            .unwrap()
            .selected_text(),
        Some("hello")
    );
    let key = |key, modifiers| {
        InputEvent::Keyboard(KeyboardEvent {
            state: KeyState::Down,
            key,
            modifiers,
            ..Default::default()
        })
    };
    let shortcut = if cfg!(target_os = "macos") {
        KeyModifiers::META
    } else {
        KeyModifiers::CONTROL
    };
    assert!(send(key(
        LogicalKey::Named(lgui::core::NamedKey::Tab),
        KeyModifiers::empty()
    )));
    assert_eq!(
        state.get().workspace.active_editor().unwrap().text(),
        "  hello world\nsecond"
    );
    send(key(LogicalKey::Character("z".into()), shortcut));
    assert_eq!(
        state
            .get()
            .workspace
            .active_editor()
            .unwrap()
            .selected_text(),
        Some("hello")
    );
    send(InputEvent::Ime(ImeEvent::Preedit {
        text: "nihao".into(),
        cursor: Some(0..5),
    }));
    assert_eq!(
        state.get().workspace.active_editor().unwrap().text(),
        "hello world\nsecond"
    );
    send(InputEvent::Ime(ImeEvent::Commit("你好".into())));
    assert_eq!(
        state.get().workspace.active_editor().unwrap().text(),
        "你好 world\nsecond"
    );
    assert!(state.get().editor.ime.is_none());
    send(key(LogicalKey::Character("z".into()), shortcut));
    assert_eq!(
        state
            .get()
            .workspace
            .active_editor()
            .unwrap()
            .selected_text(),
        Some("hello")
    );
}

#[test]
fn smooth_caret_keeps_input_and_ime_logical_and_snaps_during_drag_or_context_changes() {
    use lgui::application::{AppView, ApplicationContext};
    use lgui::core::{
        ImeEvent, InputEvent, NamedKey, Point, PointerData, UiScale, dispatch_runtime_output,
    };
    use lgui::session::UiSession;
    use std::sync::Arc;

    let exposed = Arc::new(Mutex::new(None::<AppContext>));
    let output = exposed.clone();
    let viewport = UiRect::new(0.0, 0.0, 500.0, 300.0);
    let application = ApplicationContext::empty(Default::default());
    let view_app = application.clone();
    let view: AppView = Arc::new(move |cx| {
        let mut initial = document(&format!("hello world\n{}", "line\n".repeat(40)));
        initial.editor.focused = true;
        let state = AppContext::detached(cx.state(initial));
        let git = cx.state(GitStoreSnapshot::default());
        *output.lock().unwrap() = Some(state.clone());
        let id = UiId::new("smooth-editor-test");
        let focus = cx.focus_handle(id.clone());
        let env = crate::app::key_actions::KeyEnv {
            state: state.clone(),
            git_store: git.clone(),
            application: view_app.clone(),
            editor_focus: focus.clone(),
            editor_rect: viewport,
            terminal: None,
        };
        crate::app::key_actions::attach(
            group(viewport).child(render(PaneId::new(1), viewport, state, git, id, focus)),
            env,
        )
    });
    let mut session = UiSession::new();
    session.render_view(&view, viewport, UiScale::ONE);
    let state = exposed.lock().unwrap().clone().unwrap();
    let send = |session: &mut UiSession, input| {
        dispatch_runtime_output(
            session.handle_input(input),
            &application,
            &lgui::window::WindowId::new("smooth-test"),
            |action| session.handle_default_action(action),
            |_| {},
        );
        session.render_view(&view, viewport, UiScale::ONE);
    };
    let caret = |session: &UiSession| {
        let node = session.tree().node(&UiId::new("editor-caret-1")).unwrap();
        let layer = session.tree().node(node.parent.as_ref().unwrap()).unwrap();
        let transform = layer.compositing_layer.unwrap().transform;
        node.layout_rect
            .translate(transform.translation_x(), transform.translation_y())
    };
    // UiSession has no native text service. Use the editor's existing
    // fallback metrics for the painted target, and check IME geometry separately.
    let logical = |state: &AppContext| {
        let app = state.get();
        let buffer = app.workspace.active_editor().unwrap();
        let (line, col) = buffer.line_col();
        let left = viewport.left + fonts::editor().gutter_w() + theme::CODE_PAD;
        let layout = layout_line(
            buffer.line(line),
            left,
            line as f32 * fonts::editor().line_h,
            viewport.right,
        );
        (caret_x(layout.as_ref(), col).unwrap_or(
            left + buffer.display_column(buffer.cursor()) as f32 * fonts::editor().char_w,
        ) - app.workspace.active_scroll().0)
            .round()
    };
    let left = fonts::editor().gutter_w() + theme::CODE_PAD;
    let pointer =
        |column| PointerData::mouse(Point::new(left + column * fonts::editor().char_w, 5.0));
    send(
        &mut session,
        InputEvent::PointerDown {
            pointer: pointer(0.0),
            button: PointerButton::Left,
        },
    );
    send(
        &mut session,
        InputEvent::PointerUp {
            pointer: pointer(0.0),
            button: PointerButton::Left,
        },
    );
    let start = caret(&session);
    send(&mut session, InputEvent::TextInput("中".into()));
    assert!(
        state
            .get()
            .workspace
            .active_editor()
            .unwrap()
            .text()
            .starts_with("中hello")
    );
    assert_eq!(
        state.get().workspace.active_editor().unwrap().cursor(),
        "中".len()
    );
    assert_eq!(caret(&session), start, "only the visual caret should lag");
    assert!(logical(&state) > start.left);
    assert_eq!(
        session
            .tree()
            .node(&UiId::new("smooth-editor-test"))
            .unwrap()
            .ime_cursor_rect
            .unwrap()
            .left
            .round(),
        super::ime_cursor_rect(&state.get(), viewport).left.round()
    );
    session.advance(0.0);
    session.advance(40.0);
    session.render_view(&view, viewport, UiScale::ONE);
    assert!(caret(&session).left > start.left && caret(&session).left < logical(&state));
    let displayed = caret(&session);
    send(
        &mut session,
        InputEvent::Keyboard(KeyboardEvent {
            state: KeyState::Down,
            key: LogicalKey::Named(NamedKey::ArrowRight),
            ..Default::default()
        }),
    );
    assert_eq!(
        caret(&session),
        displayed,
        "navigation retargets without jumping"
    );
    session.advance(0.0);
    session.advance(80.0);
    session.render_view(&view, viewport, UiScale::ONE);
    assert_eq!(caret(&session).left, logical(&state));

    let before_click = caret(&session);
    send(
        &mut session,
        InputEvent::PointerDown {
            pointer: pointer(8.0),
            button: PointerButton::Left,
        },
    );
    assert_eq!(
        caret(&session),
        before_click,
        "single-click placement should animate"
    );
    send(&mut session, InputEvent::PointerMove(pointer(10.0)));
    assert!(
        state
            .get()
            .workspace
            .active_editor()
            .unwrap()
            .selection()
            .is_some()
    );
    assert_eq!(
        caret(&session).left,
        logical(&state),
        "drag selection must track immediately"
    );
    send(
        &mut session,
        InputEvent::PointerUp {
            pointer: pointer(10.0),
            button: PointerButton::Left,
        },
    );
    send(&mut session, InputEvent::TextInput("x".into()));
    send(
        &mut session,
        InputEvent::Ime(ImeEvent::Preedit {
            text: "ni".into(),
            cursor: Some(0..0),
        }),
    );
    assert_eq!(
        caret(&session).left,
        logical(&state),
        "composition cancels visual motion"
    );
    send(&mut session, InputEvent::Ime(ImeEvent::Commit("你".into())));
    assert!(state.get().editor.ime.is_none());
    state.update(|app| app.preferences.values_mut().smooth_caret = false);
    session.render_view(&view, viewport, UiScale::ONE);
    assert_eq!(
        caret(&session).left,
        logical(&state),
        "turning off motion snaps an active animation"
    );
    state.update(|app| {
        app.preferences.values_mut().smooth_caret = true;
        app.workspace.active_editor_mut().unwrap().set_cursor(0);
    });
    session.render_view(&view, viewport, UiScale::ONE);
    state.update(|app| app.workspace.set_active_scroll(0.0, 1.0));
    session.render_view(&view, viewport, UiScale::ONE);
    assert_eq!(
        caret(&session).left,
        logical(&state),
        "scrolling snaps even while moving"
    );
    state.update(|app| app.editor.focused = false);
    session.render_view(&view, viewport, UiScale::ONE);
    assert!(session.tree().node(&UiId::new("editor-caret-1")).is_none());
    state.update(|app| {
        app.editor.focused = true;
        app.workspace.active_editor_mut().unwrap().set_cursor(4);
    });
    session.render_view(&view, viewport, UiScale::ONE);
    assert_eq!(
        caret(&session).left,
        logical(&state),
        "focus restoration starts at the logical caret"
    );
    state.update(|app| {
        app.workspace
            .open_path("other.txt".into(), "other file".into());
    });
    session.render_view(&view, viewport, UiScale::ONE);
    assert_eq!(
        caret(&session).left,
        logical(&state),
        "switching files snaps immediately"
    );
}

#[test]
fn scrollbar_release_survives_editor_child_reordering() {
    use lgui::application::{AppView, ApplicationContext};
    use lgui::core::{
        InputEvent, Point, PointerData, UiEventKind, UiEventPayload, UiScale,
        dispatch_runtime_output,
    };
    use lgui::session::UiSession;
    use std::sync::Arc;

    let exposed = Arc::new(Mutex::new(None::<AppContext>));
    let output = exposed.clone();
    let viewport = UiRect::new(0.0, 0.0, 500.0, 300.0);
    let view: AppView = Arc::new(move |cx| {
        let state = AppContext::detached(
            cx.state(document(&format!("{}\n", "line".repeat(100)).repeat(100))),
        );
        *output.lock().unwrap() = Some(state.clone());
        let id = cx.use_stable_id();
        let focus = cx.focus_handle(id.clone());
        let git_store = cx.state(GitStoreSnapshot::default());

        let drag_state = state.clone();
        let up_state = state.clone();
        group(viewport)
            .on_event_capture(UiEventKind::PointerMove, move |_cx, payload| {
                if let UiEventPayload::PointerMove { pointer } = payload {
                    drag_state.try_update(|app| {
                        drag_scrollbars(app, viewport, pointer.point.x, pointer.point.y)
                    });
                }
            })
            .on_event_capture(UiEventKind::PointerUp, move |_cx, _payload| {
                up_state.try_update(finish_scrollbar_drag);
            })
            .child(render(
                PaneId::new(1),
                viewport,
                state,
                git_store,
                id,
                focus,
            ))
    });
    let mut session = UiSession::new();
    session.render_view(&view, viewport, UiScale::ONE);
    let state = exposed.lock().unwrap().clone().unwrap();
    let app = ApplicationContext::empty(Default::default());
    let thumb_point = Point::new(viewport.right - 4.0, 10.0);
    let thumb_id = session.tree().hit_test(thumb_point).unwrap().id;
    let idle_style = session.tree().node(&thumb_id).unwrap().style;
    assert_eq!(session.tree().cursor_at(thumb_point), None);
    assert_eq!(
        session
            .tree()
            .cursor_at(Point::new(viewport.right - 1.0, 100.0)),
        None
    );
    assert_eq!(
        session
            .tree()
            .cursor_at(Point::new(150.0, viewport.bottom - 4.0)),
        None
    );
    assert_eq!(
        session
            .tree()
            .cursor_at(Point::new(viewport.right - 4.0, viewport.bottom - 4.0)),
        None
    );
    assert_eq!(
        session.tree().cursor_at(Point::new(150.0, 10.0)),
        Some(CursorIcon::Text)
    );
    let events = session.handle_input(InputEvent::PointerMove(PointerData::mouse(thumb_point)));
    dispatch_runtime_output(
        events,
        &app,
        &lgui::window::WindowId::new("scrollbar-test"),
        |action| session.handle_default_action(action),
        |_| {},
    );
    session.render_view(&view, viewport, UiScale::ONE);
    let hover_style = session.tree().node(&thumb_id).unwrap().style;
    assert_ne!(hover_style, idle_style);
    assert_eq!(hover_style.fill, Some(theme::c().text_soft));
    let mut send = |input| {
        let events = session.handle_input(input);
        dispatch_runtime_output(
            events,
            &app,
            &lgui::window::WindowId::new("scrollbar-test"),
            |action| session.handle_default_action(action),
            |_| {},
        );
        session.render_view(&view, viewport, UiScale::ONE);
    };
    let pointer = |x, y| PointerData::mouse(Point::new(x, y));

    send(InputEvent::PointerDown {
        pointer: pointer(viewport.right - 4.0, 10.0),
        button: PointerButton::Left,
    });
    send(InputEvent::PointerMove(pointer(100.0, 260.0)));
    assert!(state.get().editor.editor_vertical_scrollbar_dragging);
    assert!(state.get().workspace.active_scroll().1 > 0.0);

    send(InputEvent::PointerUp {
        pointer: pointer(100.0, 260.0),
        button: PointerButton::Left,
    });
    assert!(!state.get().editor.editor_vertical_scrollbar_dragging);
    let released_scroll = state.get().workspace.active_scroll().1;

    send(InputEvent::PointerMove(pointer(100.0, 20.0)));
    assert_eq!(state.get().workspace.active_scroll().1, released_scroll);
}

#[test]
fn clipboard_cut_paste_and_undo_preserve_unicode_and_selection() {
    let mut app = document("你好 world");
    let clipboard = TestClipboard::default();
    app.workspace
        .active_editor_mut()
        .unwrap()
        .select_range(0..6);
    apply_command(&mut app, Command::Copy, &clipboard).unwrap();
    assert_eq!(clipboard.read_text().unwrap().as_deref(), Some("你好"));
    apply_command(&mut app, Command::Cut, &clipboard).unwrap();
    assert_eq!(app.workspace.active_editor().unwrap().text(), " world");
    apply_command(&mut app, Command::Undo, &clipboard).unwrap();
    assert_eq!(
        app.workspace.active_editor().unwrap().selected_text(),
        Some("你好")
    );
    clipboard.write_text("再见").unwrap();
    apply_command(&mut app, Command::Paste, &clipboard).unwrap();
    assert_eq!(app.workspace.active_editor().unwrap().text(), "再见 world");
}

#[test]
fn clipboard_failure_and_empty_clipboard_never_destroy_selection() {
    let mut app = document("keep this");
    app.workspace.active_editor_mut().unwrap().select_all();
    let unavailable = TestClipboard {
        fail: true,
        ..Default::default()
    };
    for command in [Command::Cut, Command::Copy, Command::Paste] {
        assert!(apply_command(&mut app, command, &unavailable).is_err());
        assert_eq!(
            app.workspace.active_editor().unwrap().selected_text(),
            Some("keep this")
        );
    }
    apply_command(&mut app, Command::Paste, &TestClipboard::default()).unwrap();
    assert_eq!(
        app.workspace.active_editor().unwrap().selected_text(),
        Some("keep this")
    );
    assert!(!app.workspace.active_editor().unwrap().can_undo());
}

#[test]
fn editing_commands_promote_a_preview_while_navigation_does_not() {
    let mut app = AppState::new();
    let preview = app
        .workspace
        .preview_path("preview.txt".into(), "hello".into());
    let clipboard = TestClipboard::default();

    apply_command(
        &mut app,
        Command::Move(normal::Movement::Right, false),
        &clipboard,
    )
    .unwrap();
    assert_eq!(app.workspace.preview(), Some(preview));
    apply_command(&mut app, Command::Undo, &clipboard).unwrap();
    assert_eq!(app.workspace.preview(), Some(preview));

    clipboard.write_text("!").unwrap();
    apply_command(&mut app, Command::Paste, &clipboard).unwrap();
    assert_ne!(app.workspace.preview(), Some(preview));
    assert_eq!(app.workspace.active_editor().unwrap().text(), "h!ello");
}

#[test]
fn a_failed_cut_does_not_promote_or_modify_a_preview() {
    let mut app = AppState::new();
    let preview = app
        .workspace
        .preview_path("preview.txt".into(), "hello".into());
    app.workspace.active_editor_mut().unwrap().select_all();
    let unavailable = TestClipboard {
        fail: true,
        ..Default::default()
    };

    assert!(apply_command(&mut app, Command::Cut, &unavailable).is_err());
    assert_eq!(app.workspace.preview(), Some(preview));
    assert_eq!(app.workspace.active_editor().unwrap().text(), "hello");
}

#[test]
fn tab_expansion_keeps_source_indices_and_visual_columns_in_sync() {
    let (display, mapping, width) = expand_tabs("a\t中\tb", 0);
    assert_eq!(display, "a   中  b");
    assert_eq!(mapping, vec![0, 1, 4, 5, 7, 8]);
    assert_eq!(width, 9);
    assert_eq!(
        fallback_hit_column("a\t中\tb", 5.9 * fonts::editor().char_w),
        3
    );
    assert_eq!(fallback_hit_column("👩‍💻x", 1.5 * fonts::editor().char_w), 3);
}

#[test]
fn held_pointer_scrolls_and_extends_selection_without_mouse_movement() {
    let mut app = document(&(0..100).map(|i| format!("line {i}\n")).collect::<String>());
    let rect = UiRect::new(0.0, 0.0, 400.0, 100.0);
    app.editor.drag = Some(DragSelection {
        document: app.workspace.active().unwrap(),
        origin: 0..0,
        unit: SelectionUnit::Character,
        point: (100.0, 150.0),
    });
    assert!(drag_scroll_tick(&mut app, rect));
    let first = app.workspace.active_scroll().1;
    let first_selection = app.workspace.active_editor().unwrap().selection().unwrap();
    assert!(drag_scroll_tick(&mut app, rect));
    assert!(app.workspace.active_scroll().1 > first);
    assert!(
        app.workspace
            .active_editor()
            .unwrap()
            .selection()
            .unwrap()
            .end
            > first_selection.end
    );
    app.editor.drag = None;
    assert!(!drag_scroll_tick(&mut app, rect));
}

#[test]
fn scrollbar_drag_uses_window_pointer_outside_the_scrollbar_strip() {
    let mut app = document(&(0..100).map(|i| format!("line {i}\n")).collect::<String>());
    let rect = UiRect::new(0.0, 0.0, 400.0, 100.0);
    app.editor.editor_vertical_scrollbar_dragging = true;
    app.editor.editor_vertical_scrollbar_drag_offset = 4.0;

    assert!(drag_scrollbars(&mut app, rect, 40.0, 500.0));
    let bottom = app.workspace.active_scroll().1;
    assert!(bottom > 0.0);

    assert!(drag_scrollbars(&mut app, rect, 40.0, rect.top));
    assert_eq!(app.workspace.active_scroll().1, 0.0);
    assert!(finish_scrollbar_drag(&mut app));
    assert!(!app.editor.editor_vertical_scrollbar_dragging);
    assert!(!finish_scrollbar_drag(&mut app));

    let mut app = document(&"x".repeat(1_000));
    app.editor.editor_horizontal_scrollbar_dragging = true;
    app.editor.editor_horizontal_scrollbar_drag_offset = 4.0;

    assert!(drag_scrollbars(&mut app, rect, 1_000.0, 40.0));
    assert!(app.workspace.active_scroll().0 > 0.0);
    assert!(drag_scrollbars(&mut app, rect, rect.left, 40.0));
    assert_eq!(app.workspace.active_scroll().0, 0.0);
    assert!(finish_scrollbar_drag(&mut app));
    assert!(!app.editor.editor_horizontal_scrollbar_dragging);
}

#[test]
fn gutter_drag_selects_whole_lines_in_both_directions() {
    let mut app = document("one\ntwo\nthree");
    let rect = UiRect::new(0.0, 0.0, 400.0, 300.0);
    app.editor.drag = Some(DragSelection {
        document: app.workspace.active().unwrap(),
        origin: 4..8,
        unit: SelectionUnit::Line,
        point: (5.0, 1.0),
    });
    update_drag_selection(&mut app, rect);
    assert_eq!(
        app.workspace.active_editor().unwrap().selected_text(),
        Some("one\ntwo\n")
    );
    app.editor.drag.as_mut().unwrap().point.1 = fonts::editor().line_h * 2.0 + 1.0;
    update_drag_selection(&mut app, rect);
    assert_eq!(
        app.workspace.active_editor().unwrap().selected_text(),
        Some("two\nthree")
    );
}

#[test]
fn switching_documents_cancels_drag_and_keeps_undo_histories_independent() {
    let mut app = document("first");
    let first = app.workspace.active().unwrap();
    normal::insert(&mut app.workspace.active_editor_mut().unwrap(), "A");
    app.editor.drag = Some(DragSelection {
        document: first,
        origin: 0..0,
        unit: SelectionUnit::Character,
        point: (100.0, 500.0),
    });
    app.workspace
        .open_path("second.txt".into(), "second".into());
    normal::insert(&mut app.workspace.active_editor_mut().unwrap(), "B");
    assert!(drag_scroll_tick(
        &mut app,
        UiRect::new(0.0, 0.0, 400.0, 100.0)
    ));
    assert!(app.editor.drag.is_none());
    app.workspace.active_editor_mut().unwrap().undo();
    assert_eq!(app.workspace.active_editor().unwrap().text(), "second");
    app.workspace.set_active(first);
    assert_eq!(app.workspace.active_editor().unwrap().text(), "Afirst");
    app.workspace.active_editor_mut().unwrap().undo();
    assert_eq!(app.workspace.active_editor().unwrap().text(), "first");
}

#[test]
fn a_late_composition_commit_lands_in_the_view_it_started_in() {
    let rect = UiRect::new(0.0, 0.0, 400.0, 300.0);
    let mut app = document("first");
    let first = app.workspace.active().unwrap();
    let left = app.workspace.active_pane();
    update_composition(&mut app, "ni", None);
    assert!(app.editor.preedit_for(left, first).is_some());

    let second = app
        .workspace
        .open_path("second.txt".into(), "second".into());
    assert!(app.editor.preedit_for(left, second).is_none());
    insert_text(&mut app, "你", rect);
    assert_eq!(app.workspace.active(), Some(second));
    assert_eq!(app.workspace.active_editor().unwrap().text(), "second");
    app.workspace.set_active(first);
    assert_eq!(app.workspace.active_editor().unwrap().text(), "你first");
    assert!(app.editor.ime.is_none());

    // Plain typing without a composition goes to the active view.
    insert_text(&mut app, "x", rect);
    assert_eq!(app.workspace.active_editor().unwrap().text(), "你xfirst");
}

#[test]
fn a_commit_for_a_closed_view_is_dropped() {
    let rect = UiRect::new(0.0, 0.0, 400.0, 300.0);
    let mut app = document("first");
    let first = app.workspace.active().unwrap();
    update_composition(&mut app, "ni", None);
    app.workspace
        .open_path("second.txt".into(), "second".into());
    app.workspace.close(first);
    insert_text(&mut app, "你", rect);
    assert_eq!(app.workspace.active_editor().unwrap().text(), "second");
}

#[test]
fn an_emptied_composition_moves_to_the_view_that_starts_the_next_one() {
    let mut app = document("first");
    let left = app.workspace.active_pane();
    let first = app.workspace.active().unwrap();
    update_composition(&mut app, "ni", None);
    update_composition(&mut app, "", None);
    let right = app
        .workspace
        .split(left, crate::model::pane_layout::Direction::Right)
        .unwrap();
    update_composition(&mut app, "hao", None);
    assert!(app.editor.preedit_for(right, first).is_some());
    assert!(app.editor.preedit_for(left, first).is_none());
}

#[test]
fn keyboard_reveal_scrolls_to_the_document_end() {
    let mut app = document(&"long line\n".repeat(100));
    app.workspace
        .active_editor_mut()
        .unwrap()
        .select_to(usize::MAX, false);
    reveal_cursor(&mut app, UiRect::new(0.0, 0.0, 400.0, 100.0));
    assert_eq!(
        app.workspace.active_scroll().1,
        101.0 * fonts::editor().line_h - 100.0
    );
}

#[test]
fn visible_range_keeps_only_rows_around_the_viewport() {
    assert_eq!(visible_line_range(240.0, 120.0, 100), 9..16);
}

#[test]
fn thumb_geometry_maps_the_full_scroll_range_to_the_track() {
    let start = thumb_geometry(10.0, 100.0, 100.0, 400.0, 0.0);
    let end = thumb_geometry(10.0, 100.0, 100.0, 400.0, 300.0);

    assert_eq!(
        start,
        ThumbGeometry {
            start: 10.0,
            length: 28.0
        }
    );
    assert_eq!(
        end,
        ThumbGeometry {
            start: 82.0,
            length: 28.0
        }
    );
}

#[test]
fn visual_columns_expand_tabs_and_wide_characters() {
    assert_eq!(visual_columns("a\tb"), 5);
    assert_eq!(visual_columns("a界b"), 4);
}

#[test]
fn pointer_line_accounts_for_vertical_scroll_and_clamps_to_document() {
    assert_eq!(
        line_index_from_point(100.0, 100.0, fonts::editor().line_h * 2.0, 10),
        2
    );
    assert_eq!(line_index_from_point(50.0, 100.0, 0.0, 10), 0);
    assert_eq!(line_index_from_point(900.0, 100.0, 0.0, 10), 9);
}

fn render(
    pane: crate::domain::identity::PaneId,
    rect: UiRect,
    state: AppContext,
    git: State<GitStoreSnapshot>,
    id: UiId,
    focus: lgui::core::UiFocusHandle,
) -> Element {
    let binding = crate::app::editor_view::binding(state, git, pane, rect);
    crate::ui::editor::view::render(pane, rect, binding, id, focus)
}

#[test]
fn editor_menu_executes_its_intent_and_keeps_editor_focus() {
    use lgui::application::{AppView, ApplicationContext};
    use lgui::core::{InputEvent, Point, PointerData, UiScale, dispatch_runtime_output};
    use lgui::session::UiSession;
    use std::sync::Arc;
    let exposed = Arc::new(Mutex::new(None::<AppContext>));
    let output = exposed.clone();
    let viewport = UiRect::new(0.0, 0.0, 500.0, 300.0);
    let app = ApplicationContext::empty(Default::default());
    let view: AppView = Arc::new(move |cx| {
        let state = AppContext::detached(cx.state(document("hello")));
        let git = cx.state(GitStoreSnapshot::default());
        *output.lock().unwrap() = Some(state.clone());
        let id = cx.use_stable_id();
        let focus = cx.focus_handle(id.clone());
        let scene = state.get();
        let mut root = group(viewport).child(render(
            PaneId::new(1),
            viewport,
            state.clone(),
            git,
            id,
            focus.clone(),
        ));
        if let Some(pos) = scene.editor.menu {
            root = root.child(crate::app::editor_view::menu(
                viewport, viewport, pos, state, focus,
            ));
        }
        root
    });
    let mut session = UiSession::new();
    session.render_view(&view, viewport, UiScale::ONE);
    let state = exposed.lock().unwrap().clone().unwrap();
    let mut click = |point, button| {
        for input in [
            InputEvent::PointerDown {
                pointer: PointerData::mouse(point),
                button,
            },
            InputEvent::PointerUp {
                pointer: PointerData::mouse(point),
                button,
            },
        ] {
            let events = session.handle_input(input);
            dispatch_runtime_output(
                events,
                &app,
                &lgui::window::WindowId::new("menu-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
            session.render_view(&view, viewport, UiScale::ONE);
        }
    };
    click(Point::new(100.0, 10.0), PointerButton::Right);
    assert_eq!(state.get().editor.menu, Some((100.0, 10.0)));
    // The final row is Select All; menu rows retain focus and emit a core command.
    click(
        Point::new(120.0, 10.0 + 4.0 + 5.0 * 28.0 + 14.0),
        PointerButton::Left,
    );
    let value = state.get();
    assert!(value.editor.menu.is_none());
    assert_eq!(
        value.workspace.active_editor().unwrap().selected_text(),
        Some("hello")
    );
    assert!(value.editor.focused);
}

/// A server's diagnostics are drawn as underlines in the real editor view, from the
/// server process to the scene tree.
#[test]
fn diagnostics_from_a_server_are_underlined_in_the_editor() {
    use lgui::{
        application::{AppView, ApplicationContext},
        core::{InputEvent, Point, PointerData, UiScale, dispatch_runtime_output},
        session::UiSession,
    };
    use std::sync::Arc;
    let context = ApplicationContext::empty(Default::default());
    let _fonts = lgui::backend::install_text_environment(
        &context,
        lgui::render_skia::skia_text_system_handle(),
    );
    fonts::set(&fonts::FontSettings::EDITOR, &fonts::FontSettings::TERMINAL);
    let exe = std::env::current_exe().unwrap();
    let fake = exe
        .parent()
        .and_then(|deps| deps.parent())
        .unwrap()
        .join("examples")
        .join(if cfg!(windows) {
            "fake_lsp.exe"
        } else {
            "fake_lsp"
        });
    let mut app = AppState::new();
    app.workspace.open_path(
        std::env::temp_dir().join("loom-diagnostic-test.rs"),
        "fn a() {}\nsecond line\n".into(),
    );
    app.preferences.values_mut().language_servers =
        vec![crate::features::settings::servers::LanguageServerSetting {
            server: "rust-analyzer".into(),
            enabled: true,
            command: Some(fake.display().to_string()),
            args: Some(vec!["--sync".into(), "full".into(), "--diagnostics".into()]),
            initialization_options: None,
            install: None,
        }];
    let rect = UiRect::new(0.0, 0.0, 800.0, 400.0);
    let initial = Arc::new(Mutex::new(Some(app)));
    let exposed = Arc::new(Mutex::new(None));
    let output = exposed.clone();
    let runtime = Arc::new(crate::app::runtime::AppRuntime::default());
    let view_runtime = runtime.clone();
    let tasks = crate::app::test_support::UiTasks::default();
    let application = tasks.application();
    let view: AppView = Arc::new(move |cx| {
        let state = AppContext::detached(cx.state_with(|| initial.lock().unwrap().take().unwrap()))
            .with_runtime(&view_runtime);
        if view_runtime.highlighter().is_none() {
            view_runtime.install_highlighter(crate::app::syntax::Highlighter::new(
                state.clone(),
                application.clone(),
            ));
            let results_state = state.clone();
            view_runtime.install_language_services(
                crate::app::language_services::LanguageServices::new(
                    crate::features::language_service::server_set::rust_analyzer_for_tests(),
                    move || {
                        results_state.try_update(|app| {
                            app.editor.syntax_revision = app.editor.syntax_revision.wrapping_add(1);
                            true
                        });
                    },
                ),
            );
        }
        let git = cx.state_with(GitStoreSnapshot::default);
        *output.lock().unwrap() = Some(state.clone());
        let id = UiId::new("diagnostic-editor-test");
        let focus = cx.focus_handle(id.clone());
        render(PaneId::new(1), rect, state, git, id, focus)
    });
    let mut session = UiSession::new();
    session.render_view(&view, rect, UiScale::ONE);
    let state: AppContext = exposed.lock().unwrap().clone().unwrap();
    // Make the services see the document, then wait for the server's answer.
    state.update(|app| app.editor.syntax_revision = app.editor.syntax_revision.wrapping_add(1));
    // The underline is a wave stroked in the error colour.
    let squiggle = |session: &UiSession| {
        session
            .tree()
            .nodes()
            .iter()
            .find(|node| {
                node.path.is_some()
                    && node
                        .path_style
                        .stroke
                        .is_some_and(|stroke| stroke.color == theme::c().error)
            })
            .map(|node| node.paint_bounds)
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let underline = loop {
        tasks.run_all();
        session.render_view(&view, rect, UiScale::ONE);
        if let Some(bounds) = squiggle(&session) {
            break bounds;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no underline was drawn; infos {:?}; services {}; docs {:?}",
            runtime.language_services().map(|s| s.infos()),
            runtime.language_services().is_some(),
            state
                .get()
                .workspace
                .documents
                .values()
                .map(|d| (d.is_text(), d.meta.path.clone(), d.meta.lang.name()))
                .collect::<Vec<_>>()
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    // The scrollbar's ruler marks the line too.
    assert!(
        session.tree().nodes().iter().any(|node| {
            node.style == lgui::prelude::VisualStyle::filled(theme::c().error)
                && node.layout_rect.right == rect.right - 1.0
        }),
        "no ruler mark for the error"
    );
    // The pointer over the underlined text opens its message.
    let has_message = |session: &UiSession| {
        session
            .tree()
            .nodes()
            .iter()
            .any(|node| node.text.as_deref() == Some("fake error"))
    };
    assert!(!has_message(&session));
    let over = Point::new(
        (underline.left + underline.right) / 2.0,
        underline.top - 4.0,
    );
    let send = |session: &mut UiSession, input| {
        let events = session.handle_input(input);
        dispatch_runtime_output(
            events,
            &context,
            &lgui::window::WindowId::new("diagnostic-test"),
            |action| session.handle_default_action(action),
            |_| {},
        );
        session.render_view(&view, rect, UiScale::ONE);
    };
    send(
        &mut session,
        InputEvent::PointerMove(PointerData::mouse(over)),
    );
    assert!(has_message(&session), "no hover card over the underline");
    assert!(
        session
            .tree()
            .nodes()
            .iter()
            .any(|node| node.text.as_deref() == Some("fake")),
        "the card names the reporting tool"
    );
    send(
        &mut session,
        InputEvent::PointerMove(PointerData::mouse(Point::new(700.0, 300.0))),
    );
    assert!(
        !has_message(&session),
        "the card stays after the pointer left"
    );
}

use std::sync::Arc;

/// What a language-server feature test needs: the real editor view over a real
/// fake-server process, with a way to pump the UI and move the pointer.
struct LanguageHarness {
    session: lgui::session::UiSession,
    view: lgui::application::AppView,
    context: lgui::application::ApplicationContext,
    state: AppContext,
    tasks: crate::app::test_support::UiTasks,
    runtime: Arc<crate::app::runtime::AppRuntime>,
    rect: UiRect,
}

impl LanguageHarness {
    fn new(source: &str, server_args: &[&str]) -> Self {
        use lgui::{
            application::{AppView, ApplicationContext},
            core::UiScale,
            session::UiSession,
        };
        let context = ApplicationContext::empty(Default::default());
        let fonts_handle = lgui::backend::install_text_environment(
            &context,
            lgui::render_skia::skia_text_system_handle(),
        );
        std::mem::forget(fonts_handle);
        fonts::set(&fonts::FontSettings::EDITOR, &fonts::FontSettings::TERMINAL);
        let exe = std::env::current_exe().unwrap();
        let fake = exe
            .parent()
            .and_then(|deps| deps.parent())
            .unwrap()
            .join("examples")
            .join(if cfg!(windows) {
                "fake_lsp.exe"
            } else {
                "fake_lsp"
            });
        let mut app = AppState::new();
        let name = format!(
            "loom-language-ui-{}-{}.rs",
            std::process::id(),
            LANGUAGE_CASES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        app.workspace
            .open_path(std::env::temp_dir().join(name), source.into());
        app.editor.focused = true;
        let mut args: Vec<String> = ["--sync", "incremental"].map(String::from).into();
        args.extend(server_args.iter().map(|a| a.to_string()));
        app.preferences.values_mut().language_servers =
            vec![crate::features::settings::servers::LanguageServerSetting {
                server: "rust-analyzer".into(),
                enabled: true,
                command: Some(fake.display().to_string()),
                args: Some(args),
                initialization_options: None,
                install: None,
            }];
        let rect = UiRect::new(0.0, 0.0, 800.0, 400.0);
        let initial = Arc::new(Mutex::new(Some(app)));
        let exposed = Arc::new(Mutex::new(None));
        let output = exposed.clone();
        let runtime = Arc::new(crate::app::runtime::AppRuntime::default());
        let view_runtime = runtime.clone();
        let tasks = crate::app::test_support::UiTasks::default();
        let application = tasks.application();
        runtime.set_application(application.clone());
        let view: AppView = Arc::new(move |cx| {
            let state =
                AppContext::detached(cx.state_with(|| initial.lock().unwrap().take().unwrap()))
                    .with_runtime(&view_runtime);
            if view_runtime.highlighter().is_none() {
                view_runtime.install_highlighter(crate::app::syntax::Highlighter::new(
                    state.clone(),
                    application.clone(),
                ));
                let results_state = state.clone();
                view_runtime.install_language_services(
                    crate::app::language_services::LanguageServices::new(
                        crate::features::language_service::server_set::rust_analyzer_for_tests(),
                        move || {
                            crate::app::language_ui::tick(&results_state);
                        },
                    ),
                );
            }
            let git = cx.state_with(GitStoreSnapshot::default);
            *output.lock().unwrap() = Some(state.clone());
            let id = UiId::new("language-ui-test");
            let focus = cx.focus_handle(id.clone());
            render(PaneId::new(1), rect, state, git, id, focus)
        });
        let mut session = UiSession::new();
        session.render_view(&view, rect, UiScale::ONE);
        let state: AppContext = exposed.lock().unwrap().clone().unwrap();
        state.update(|app| app.editor.syntax_revision = app.editor.syntax_revision.wrapping_add(1));
        Self {
            session,
            view,
            context,
            state,
            tasks,
            runtime,
            rect,
        }
    }

    fn render(&mut self) {
        self.tasks.run_all();
        self.session
            .render_view(&self.view, self.rect, lgui::core::UiScale::ONE);
    }

    fn send(&mut self, input: lgui::core::InputEvent) {
        let events = self.session.handle_input(input);
        lgui::core::dispatch_runtime_output(
            events,
            &self.context,
            &lgui::window::WindowId::new("language-ui-test"),
            |action| self.session.handle_default_action(action),
            |_| {},
        );
        self.render();
    }

    fn move_to(&mut self, x: f32, y: f32) {
        use lgui::core::{InputEvent, Point, PointerData};
        self.send(InputEvent::PointerMove(PointerData::mouse(Point::new(
            x, y,
        ))));
    }

    fn has_text(&self, wanted: &str) -> bool {
        self.session
            .tree()
            .nodes()
            .iter()
            .any(|node| node.text.as_deref() == Some(wanted))
    }

    /// Keeps rendering until `check` passes, or fails with `what`.
    fn until(&mut self, what: &str, mut check: impl FnMut(&mut Self) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            self.render();
            if check(self) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {what}; infos {:?}",
                self.runtime.language_services().map(|s| s.infos())
            );
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    }

    /// Waits until a server is ready and holds the document.
    fn until_ready(&mut self) {
        self.until("the server to be ready", |h| {
            h.runtime.language_services().is_some_and(|s| {
                s.infos()
                    .iter()
                    .any(|i| i.status == crate::infrastructure::lsp::manager::Status::Ready)
            })
        });
    }

    /// The screen position of the character at `column` of `line`.
    fn point_at(&self, line: usize, column: usize) -> (f32, f32) {
        let code_left = self.rect.left + fonts::editor().gutter_w() + theme::CODE_PAD;
        (
            code_left + (column as f32 + 0.5) * fonts::editor().char_w,
            self.rect.top + (line as f32 + 0.5) * fonts::editor().line_h,
        )
    }
}

static LANGUAGE_CASES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[test]
fn resting_the_pointer_on_a_word_shows_what_the_server_says_about_it() {
    let mut h = LanguageHarness::new(
        "fn alpha() {}



last line
",
        &["--features", "hover", "--diagnostics"],
    );
    h.until_ready();
    let (x, y) = h.point_at(0, 1);
    h.move_to(x, y);
    assert!(!h.has_text("0:1"), "nothing before the pointer has rested");
    // The middle of "fn" is column 1 of line 0.
    h.until("the hover card", |h| h.has_text("0:1"));
    assert!(h.has_text("fake"), "the server's text is on the card");
    // Moving to another word replaces the card; leaving the text removes it.
    let (x, y) = h.point_at(4, 2);
    h.move_to(x, y);
    assert!(!h.has_text("0:1"), "the old card went with the pointer");
    h.until("the new hover card", |h| h.has_text("4:2"));
    h.move_to(790.0, 390.0);
    assert!(!h.has_text("4:2"), "the card stays after the pointer left");
    // Typing hides the card too.
    let (x, y) = h.point_at(0, 1);
    h.move_to(x, y);
    h.until("the card again", |h| h.has_text("0:1"));
    h.state.update(|app| {
        crate::app::editor::insert_text(app, "x", UiRect::new(0.0, 0.0, 800.0, 400.0));
    });
    h.render();
    assert!(!h.has_text("0:1"), "typing hides the card");
}

#[test]
fn a_server_without_hover_leaves_the_pointer_alone() {
    let mut h = LanguageHarness::new("fn alpha() {}\n", &["--features", "definition"]);
    h.until_ready();
    let (x, y) = h.point_at(0, 1);
    h.move_to(x, y);
    std::thread::sleep(std::time::Duration::from_millis(500));
    h.render();
    assert!(!h.has_text("0:1"));
    assert!(h.state.get().editor.language.hover.card.is_none());
}

impl LanguageHarness {
    /// Performs a keymap action the way the key handler does.
    fn act(&mut self, action: crate::features::keymap::action::Action) {
        crate::app::language_ui::keys::perform_with(&self.state, self.rect, action);
        self.render();
    }

    fn set_cursor(&mut self, offset: usize) {
        self.state.update(|app| {
            app.workspace
                .active_editor_mut()
                .unwrap()
                .set_cursor(offset)
        });
        self.render();
    }

    fn selection(&self) -> Option<std::ops::Range<usize>> {
        self.state
            .get()
            .workspace
            .active_editor()
            .unwrap()
            .selection()
    }

    fn cursor(&self) -> usize {
        self.state.get().workspace.active_editor().unwrap().cursor()
    }

    fn popup_labels(&self) -> Vec<String> {
        self.state
            .get()
            .editor
            .language
            .popup
            .as_ref()
            .map(|p| {
                p.shown
                    .iter()
                    .map(|(at, _)| p.items[*at].label.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn until_popup(&mut self, what: &str) {
        self.until(what, |h| h.state.get().editor.language.popup.is_some());
    }
}

const NAV_SOURCE: &str = "fn alpha() {}\nsecond line\nthird\n";

#[test]
fn go_to_definition_jumps_selects_and_goes_back() {
    use crate::features::keymap::action::Action;
    let mut h = LanguageHarness::new(NAV_SOURCE, &["--features", "definition"]);
    h.until_ready();
    h.set_cursor(15);
    h.act(Action::GoToDefinition);
    h.until("the jump", |h| h.selection() == Some(0..1));
    h.act(Action::GoBack);
    h.until("coming back", |h| h.cursor() == 15);
    h.act(Action::GoForward);
    h.until("going forward again", |h| h.cursor() == 1);
}

#[test]
fn a_server_without_definitions_says_so() {
    use crate::features::keymap::action::Action;
    let mut h = LanguageHarness::new(NAV_SOURCE, &["--features", "hover"]);
    h.until_ready();
    h.act(Action::GoToDefinition);
    h.until("the message", |h| {
        h.state
            .get()
            .chrome
            .toast
            .as_ref()
            .is_some_and(|toast| toast.message.contains("No language server"))
    });
    assert!(h.state.get().editor.language.popup.is_none());
}

#[test]
fn references_open_a_list_whose_rows_go_to_their_places() {
    use crate::features::keymap::action::Action;
    let mut h = LanguageHarness::new(NAV_SOURCE, &["--features", "references"]);
    h.until_ready();
    h.act(Action::FindAllReferences);
    h.until_popup("the references");
    assert_eq!(h.popup_labels().len(), 2);
    assert!(h.has_text("2 references"));
    h.act(Action::MenuSelectNext);
    h.act(Action::MenuConfirm);
    // The second reference is line 1, columns 2 to 4.
    h.until("the jump", |h| h.selection() == Some(16..18));
    assert!(h.state.get().editor.language.popup.is_none());
}

#[test]
fn escape_closes_the_list_without_going_anywhere() {
    use crate::features::keymap::action::Action;
    let mut h = LanguageHarness::new(NAV_SOURCE, &["--features", "references"]);
    h.until_ready();
    h.set_cursor(15);
    h.act(Action::FindAllReferences);
    h.until_popup("the references");
    h.act(Action::MenuCancel);
    assert!(h.state.get().editor.language.popup.is_none());
    assert_eq!(h.cursor(), 15);
}

#[test]
fn the_outline_filters_as_you_type_and_goes_to_the_symbol() {
    use crate::features::keymap::action::Action;
    let mut h = LanguageHarness::new(NAV_SOURCE, &["--features", "symbols"]);
    h.until_ready();
    h.act(Action::ToggleOutline);
    h.until_popup("the outline");
    assert_eq!(h.popup_labels(), ["main"]);
    assert!(crate::app::language_ui::keys::type_text(
        &h.state, "zz", h.rect
    ));
    assert!(h.popup_labels().is_empty(), "nothing matches zz");
    h.act(Action::Backspace);
    h.act(Action::Backspace);
    assert!(crate::app::language_ui::keys::type_text(
        &h.state, "ma", h.rect
    ));
    assert_eq!(h.popup_labels(), ["main"]);
    h.act(Action::MenuConfirm);
    h.until("the jump", |h| h.selection() == Some(3..7));
}

#[test]
fn the_workspace_symbol_search_asks_again_as_the_text_changes() {
    use crate::features::keymap::action::Action;
    let mut h = LanguageHarness::new(NAV_SOURCE, &["--features", "workspaceSymbols"]);
    h.until_ready();
    h.act(Action::ToggleProjectSymbols);
    h.until("the first answer", |h| h.popup_labels() == ["sym-"]);
    assert!(crate::app::language_ui::keys::type_text(
        &h.state, "ab", h.rect
    ));
    h.until("the answer for ab", |h| h.popup_labels() == ["sym-ab"]);
}

#[test]
fn problems_are_visited_in_order_and_wrap() {
    use crate::features::keymap::action::Action;
    let mut h = LanguageHarness::new(NAV_SOURCE, &["--diagnostics"]);
    h.until_ready();
    h.until("diagnostics", |h| {
        let app = h.state.get();
        let doc = app
            .workspace
            .documents
            .values()
            .find(|d| d.is_text())
            .unwrap();
        let id = *app.workspace.documents.keys().next().unwrap();
        h.runtime
            .language_services()
            .and_then(|s| s.results(&doc.meta.path, &doc.stamp(id)).diagnostics)
            .is_some()
    });
    h.set_cursor(10);
    h.act(Action::GoToDiagnostic);
    assert_eq!(h.cursor(), 0, "after the last problem comes the first");
    h.set_cursor(10);
    h.act(Action::GoToPreviousDiagnostic);
    assert_eq!(h.cursor(), 0);
}

impl LanguageHarness {
    fn type_text(&mut self, text: &str) {
        let rect = self.rect;
        for ch in text.chars() {
            self.state.update(|app| {
                crate::app::editor::insert_text(app, &ch.to_string(), rect);
            });
            self.render();
        }
    }

    fn document_text(&self) -> String {
        self.state
            .get()
            .workspace
            .active_editor()
            .unwrap()
            .text()
            .to_owned()
    }

    fn completion_labels(&self) -> Vec<String> {
        let app = self.state.get();
        app.editor
            .language
            .completion
            .as_ref()
            .map(|s| {
                s.shown
                    .iter()
                    .map(|shown| s.data.entries[shown.entry].item.label.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[test]
fn typing_opens_a_completion_list_that_narrows_with_more_typing() {
    let mut h = LanguageHarness::new("\n", &["--features", "completion"]);
    h.until_ready();
    h.type_text("a");
    h.until("the list", |h| h.completion_labels().len() == 2);
    h.render();
    assert!(h.has_text("alpha-fake"));
    // More typing narrows it without another round trip.
    h.type_text("l");
    assert_eq!(h.completion_labels(), ["alpha-fake"]);
    // Typing something that matches nothing ends it.
    h.type_text("zz");
    assert!(h.completion_labels().is_empty());
    assert!(h.state.get().editor.language.completion.is_none());
}

#[test]
fn enter_accepts_the_highlighted_item_in_place_of_the_typed_word() {
    use crate::features::keymap::action::Action;
    let mut h = LanguageHarness::new("x = \n", &["--features", "completion"]);
    h.until_ready();
    h.set_cursor(4);
    h.type_text("al");
    h.until("the list", |h| h.completion_labels() == ["alpha-fake"]);
    h.act(Action::MenuConfirm);
    assert_eq!(h.document_text(), "x = alpha-fake\n");
    assert!(h.state.get().editor.language.completion.is_none());
    assert_eq!(h.cursor(), 14);
}

#[test]
fn a_snippet_item_leaves_its_stops_to_fill_in() {
    use crate::features::keymap::action::Action;
    let mut h = LanguageHarness::new("\n", &["--features", "completion"]);
    h.until_ready();
    h.type_text("be");
    h.until("the list", |h| h.completion_labels() == ["beta"]);
    h.act(Action::MenuConfirm);
    assert_eq!(h.document_text(), "beta()\n");
    assert_eq!(
        h.cursor(),
        5,
        "the caret is on the first stop, between the brackets"
    );
    assert!(h.state.get().editor.language.snippet.is_some());
    h.type_text("1, 2");
    assert_eq!(h.document_text(), "beta(1, 2)\n");
    h.act(Action::NextSnippetTabstop);
    assert_eq!(
        h.cursor(),
        10,
        "the final stop is after the closing bracket"
    );
    assert!(h.state.get().editor.language.snippet.is_none());
}

#[test]
fn escape_closes_the_list_and_leaves_the_text_alone() {
    use crate::features::keymap::action::Action;
    let mut h = LanguageHarness::new("\n", &["--features", "completion"]);
    h.until_ready();
    h.type_text("a");
    h.until("the list", |h| !h.completion_labels().is_empty());
    h.act(Action::MenuCancel);
    assert!(h.completion_labels().is_empty());
    assert_eq!(h.document_text(), "a\n");
}

#[test]
fn the_completion_key_asks_even_with_nothing_typed() {
    use crate::features::keymap::action::Action;
    let mut h = LanguageHarness::new("\n", &["--features", "completion"]);
    h.until_ready();
    h.act(Action::ShowCompletions);
    h.until("the list", |h| h.completion_labels().len() == 2);
}

#[test]
fn a_trigger_character_opens_the_list_without_a_word() {
    let mut h = LanguageHarness::new("x\n", &["--features", "completion"]);
    h.until_ready();
    h.set_cursor(1);
    h.type_text(".");
    h.until("the list", |h| h.completion_labels().len() == 2);
    // Typing a letter narrows it, as for any list.
    h.type_text("b");
    assert_eq!(h.completion_labels(), ["beta"]);
}

#[test]
fn the_highlighted_item_shows_its_documentation_once_the_server_resolves_it() {
    use crate::features::keymap::action::Action;
    let mut h = LanguageHarness::new("\n", &["--features", "completion"]);
    h.until_ready();
    h.type_text("a");
    h.until("the list", |h| h.completion_labels().len() == 2);
    // Down to "beta", whose documentation the server only gives on request.
    h.act(Action::MenuSelectNext);
    h.until("the documentation", |h| h.has_text("resolved"));
}

#[test]
fn a_server_without_completion_never_opens_a_list() {
    let mut h = LanguageHarness::new("\n", &["--features", "hover"]);
    h.until_ready();
    h.type_text("abc");
    std::thread::sleep(std::time::Duration::from_millis(200));
    h.render();
    assert!(h.state.get().editor.language.completion.is_none());
}

fn server_log(h: &LanguageHarness, path: &std::path::Path) -> Vec<String> {
    let _ = h;
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|message| message["method"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn renaming_asks_for_a_name_and_applies_the_servers_edit() {
    use crate::features::keymap::action::Action;
    let mut h = LanguageHarness::new("fn alpha() {}\n", &["--features", "rename"]);
    h.until_ready();
    h.set_cursor(4);
    h.act(Action::Rename);
    h.until_popup("the rename box");
    let query = |h: &LanguageHarness| {
        h.state
            .get()
            .editor
            .language
            .popup
            .as_ref()
            .and_then(|p| p.query.clone())
    };
    // The server's placeholder is what the box starts with.
    assert_eq!(query(&h).as_deref(), Some("name"));
    for _ in 0..4 {
        h.act(Action::Backspace);
    }
    assert!(crate::app::language_ui::keys::type_text(
        &h.state, "renamed", h.rect
    ));
    h.act(Action::MenuConfirm);
    h.until("the rename", |h| h.document_text().starts_with("renamed"));
    assert!(h.state.get().editor.language.popup.is_none());
}

#[test]
fn a_position_that_cannot_be_renamed_says_so() {
    use crate::features::keymap::action::Action;
    let mut h = LanguageHarness::new("fn alpha() {}\n", &["--features", "hover"]);
    h.until_ready();
    h.act(Action::Rename);
    h.until("the message", |h| h.state.get().chrome.toast.is_some());
    assert!(h.state.get().editor.language.popup.is_none());
}

#[test]
fn code_actions_are_listed_and_the_chosen_ones_edit_or_resolve_first() {
    use crate::features::keymap::action::Action;
    let log = std::env::temp_dir().join(format!("loom-actions-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&log);
    let log_arg = log.display().to_string();
    let mut h = LanguageHarness::new(
        "fn alpha() {}\n",
        &["--features", "codeAction,execute", "--log", &log_arg],
    );
    h.until_ready();
    h.act(Action::ToggleCodeActions);
    h.until_popup("the actions");
    let labels = h.popup_labels();
    assert_eq!(labels.len(), 2);
    assert!(
        labels[0].contains("Fix it"),
        "the preferred fix first: {labels:?}"
    );
    // The second needs resolving, which brings a command to run.
    h.act(Action::MenuSelectNext);
    h.act(Action::MenuConfirm);
    h.until("the command", |h| {
        server_log(h, &log).contains(&"workspace/executeCommand".to_owned())
    });
    assert!(server_log(&h, &log).contains(&"codeAction/resolve".to_owned()));
    // The first has its edit with it.
    h.act(Action::ToggleCodeActions);
    h.until_popup("the actions again");
    h.act(Action::MenuConfirm);
    h.until("the fix", |h| h.document_text().starts_with("// fixed\n"));
}

#[test]
fn formatting_applies_the_servers_edits_and_undo_takes_them_back() {
    use crate::features::keymap::action::Action;
    let mut h = LanguageHarness::new("fn alpha() {}\n", &["--features", "formatting"]);
    h.until_ready();
    h.act(Action::Format);
    h.until("the formatting", |h| {
        h.document_text().starts_with("// formatted\n")
    });
    h.state.update(|app| {
        let rect = UiRect::new(0.0, 0.0, 800.0, 400.0);
        let _ = rect;
        app.workspace.active_editor_mut().unwrap().undo();
    });
    assert_eq!(h.document_text(), "fn alpha() {}\n");
}
