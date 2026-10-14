use crate::app::context::AppContext;
use crate::app::state::AppState;
use crate::app::terminal_binding::{TerminalTabs, fixtures::Sessions as OwnedTabs};
use crate::app::terminal_runtime::TerminalDisplay;
use crate::features::terminal::{ShellKind, TerminalSnapshot};
fn make_tabs(n: usize) -> OwnedTabs {
    let mut tabs = OwnedTabs::new();
    for _ in 1..n {
        tabs.add(ShellKind::Cmd);
    }
    for tab in tabs.tabs() {
        tab.controller.test_controller().bench_seed(&input());
    }
    tabs
}
fn screen_snapshots(tabs: &OwnedTabs) -> Vec<TerminalSnapshot> {
    tabs.tabs()
        .iter()
        .map(|t| t.controller.snapshot())
        .collect()
}
fn model_snapshot(tabs: &OwnedTabs) -> crate::features::terminal::model::TerminalModel {
    tabs.model().clone()
}
fn frame_projection(tabs: &OwnedTabs) -> crate::ui::terminal::bindings::View {
    crate::app::terminal_view::project(TerminalTabs::clone(tabs))
}
fn make_view(n: usize) -> AppView {
    Arc::new(move |cx| {
        let state = AppContext::detached(cx.state_with(|| {
            let mut app = AppState::new();
            app.terminal_chrome.show_terminal = true;
            app.terminal.ui.terminal_focused = true;
            app
        }));
        let tabs = crate::app::terminal_binding::test_store(cx, Some(state.clone()), || {
            let mut tabs = TerminalTabs::new();
            for _ in 1..n {
                tabs.add(ShellKind::Cmd);
            }
            tabs
        });
        let seeded = cx.state(false);
        if !seeded.get() {
            for tab in tabs.get().tabs() {
                tab.controller.test_controller().bench_seed(&input());
            }
            seeded.set(true);
        }
        let id = cx.use_stable_id();
        let focus = cx.focus_handle(id.clone());
        let editor_id = cx.use_stable_id();
        let editor_focus = cx.focus_handle(editor_id);
        let controller = tabs.get().active().unwrap().controller;
        crate::app::terminal_view::render(
            cx.viewport(),
            state,
            editor_focus,
            focus,
            id,
            controller,
            tabs,
            Arc::new(lgui::ApplicationHandle::new(|task| task(), || {})),
            true,
            720.0,
        )
    })
}

// Shared measurement body injected unchanged into the frozen before source and after source.
use lgui::application::AppView;
use lgui::core::UiScale;
use lgui::prelude::UiRect;
use lgui::session::UiSession;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

fn input() -> Vec<u8> {
    "\x1b[31m中文 terminal frame\x1b[0m\r\n"
        .repeat(120)
        .into_bytes()
}
fn emit(n: usize, metric: &str, start: Instant, iterations: usize, nodes: usize) {
    println!(
        "R7_TERMINAL_PERF {}",
        serde_json::json!({"sessions":n,"metric":metric,"milliseconds":start.elapsed().as_secs_f64()*1000.0,"iterations":iterations,"nodes":nodes,"viewport":"1280x720","scale":1,"screen":"24x80"})
    );
}
#[test]
#[ignore = "opt-in R7 production terminal snapshot, frame and projection comparison"]
fn terminal_production_benchmark() {
    crate::ui::fonts::set(
        &crate::ui::fonts::FontSettings::EDITOR,
        &crate::ui::fonts::FontSettings::TERMINAL,
    );
    for n in [1, 8, 32] {
        let tabs = make_tabs(n);
        black_box(screen_snapshots(&tabs));
        let start = Instant::now();
        for _ in 0..100 {
            black_box(screen_snapshots(&tabs));
        }
        emit(n, "production_screen_snapshot_100", start, 100, 0);
        black_box(model_snapshot(&tabs));
        let start = Instant::now();
        for _ in 0..500 {
            black_box(model_snapshot(&tabs));
        }
        emit(n, "model_snapshot_500", start, 500, 0);
        black_box(frame_projection(&tabs));
        let start = Instant::now();
        for _ in 0..500 {
            black_box(frame_projection(&tabs));
        }
        emit(n, "frame_projection_500", start, 500, 0);
        let view = make_view(n);
        let rect = UiRect::new(0.0, 0.0, 1280.0, 720.0);
        let mut session = UiSession::new();
        session.render_view(&view, rect, UiScale::ONE);
        let nodes = session.tree().nodes().len();
        let start = Instant::now();
        for _ in 0..100 {
            session.render_view(&view, rect, UiScale::ONE);
            black_box(session.tree().nodes().len());
        }
        assert_eq!(session.tree().nodes().len(), nodes);
        emit(n, "terminal_ui_frame_100", start, 100, nodes);
    }
}
