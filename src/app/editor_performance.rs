//! Opt-in comparisons using real Skia layout; input text is never logged.
use super::{context::AppContext, state::AppState};
use crate::{features::git::GitStoreSnapshot, ui::editor::view};
use lgui::{
    application::{AppView, ApplicationContext},
    core::{UiId, UiScale},
    prelude::UiRect,
    session::UiSession,
};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

#[test]
#[ignore = "real text layout benchmark; optional LOOM_EDITOR_PROFILE_FILE stays in memory"]
fn editor_viewport_benchmark() {
    let js = "const item={name:\"hello\\\"world\",value:12345};function get(a){return a.value+item.value;}";
    let mut cases = vec![
        ("minified-3k.js", (js.repeat(36) + "\n").repeat(50)),
        ("minified-1m.js", js.repeat(12_000)),
    ];
    if let Some(path) = std::env::var_os("LOOM_EDITOR_PROFILE_FILE") {
        let label = if crate::domain::language::Language::from_path(std::path::Path::new(&path))
            == crate::domain::language::Language::JavaScript
        {
            "local-input.js"
        } else {
            "local-input.txt"
        };
        cases.insert(
            0,
            (
                label,
                std::fs::read_to_string(path).expect("read local profiling input"),
            ),
        );
    }
    let context = ApplicationContext::empty(Default::default());
    let _fonts = lgui::backend::install_text_environment(
        &context,
        lgui::render_skia::skia_text_system_handle(),
    );
    for (name, source) in cases {
        let bytes = source.len();
        let prepare = Instant::now();
        let mut app = AppState::new();
        app.workspace.open_path(name.into(), source);
        app.editor.focused = true;
        let prepare_ms = prepare.elapsed().as_secs_f64() * 1000.0;
        let initial = Arc::new(Mutex::new(Some(app)));
        let exposed = Arc::new(Mutex::new(None));
        let output = exposed.clone();
        let rect = UiRect::new(0.0, 0.0, 1000.0, 600.0);
        let ui: AppView = Arc::new(move |cx| {
            let state =
                AppContext::detached(cx.state_with(|| initial.lock().unwrap().take().unwrap()));
            let git = cx.state_with(GitStoreSnapshot::default);
            *output.lock().unwrap() = Some(state.clone());
            let id = UiId::new("editor-viewport-bench");
            let focus = cx.focus_handle(id.clone());
            view::render(
                state.get().workspace.active_pane(),
                rect,
                super::editor_view::binding(
                    state,
                    git,
                    crate::domain::identity::PaneId::new(1),
                    rect,
                ),
                id,
                focus,
            )
        });
        let mut session = UiSession::new();
        let cold = Instant::now();
        session.render_view(&ui, rect, UiScale::ONE);
        let cold_ms = cold.elapsed().as_secs_f64() * 1000.0;
        let nodes = session.tree().nodes().len();
        let submitted_chars: usize = session
            .tree()
            .nodes()
            .iter()
            .filter_map(|node| node.text.as_deref())
            .map(|text| text.chars().count())
            .sum();
        let state: AppContext = exposed.lock().unwrap().clone().unwrap();
        let mut frames = Vec::new();
        for i in 0..20 {
            state.update(|app| app.workspace.set_active_scroll(i as f32 * 12.0, 0.0));
            let start = Instant::now();
            session.render_view(&ui, rect, UiScale::ONE);
            frames.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        // Check the actual input's position mapping without logging any text.
        let app = state.get();
        let buffer = app.workspace.active_editor().unwrap();
        let left = crate::ui::fonts::editor().gutter_w() + crate::ui::theme::CODE_PAD;
        let metrics = view::scroll_metrics(rect, left, &buffer);
        for line in [0, buffer.line_count() / 2, buffer.line_count() - 1] {
            let visual = buffer.line_display_width(line) / 2;
            let offset = buffer.offset_at_display_column(line, visual);
            let column = buffer.buffer().line_col(offset).1;
            let sx = (visual as f32 * crate::ui::fonts::editor().char_w - metrics.viewport_w / 2.0)
                .max(0.0);
            let x = view::document_caret(
                &buffer,
                line,
                column,
                left,
                0.0,
                left + metrics.content_w,
                (sx, metrics.viewport_w),
            );
            let hit = view::cursor_offset_from_point(
                &buffer,
                rect,
                left,
                metrics,
                sx,
                line as f32 * crate::ui::fonts::editor().line_h,
                x - sx + 0.05,
                crate::ui::fonts::editor().line_h / 2.0,
            );
            assert_eq!(
                hit, offset,
                "position mapping must round-trip for the profiling input"
            );
        }
        frames.sort_by(f64::total_cmp);
        println!(
            "EDITOR_VIEWPORT {}",
            serde_json::json!({"case":name,"bytes":bytes,
            "prepare_ms":prepare_ms,"cold_ms":cold_ms,"scroll_median_ms":frames[10],
            "scroll_max_ms":frames[19],"nodes":nodes,"submitted_chars":submitted_chars})
        );
    }
}
