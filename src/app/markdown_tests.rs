//! End-to-end reader projections use the real asynchronous parser and Skia worker.
use super::{
    context::AppContext, markdown_service::MarkdownService, runtime::AppRuntime, state::AppState,
};
use crate::{
    domain::{identity::PaneId, view::Presentation},
    features::markdown::{model::*, parser::parse_document},
};
use lgui::{
    application::{AppView, ApplicationContext},
    core::{
        ImeEvent, InputEvent, KeyModifiers, KeyState, KeyboardEvent, LogicalKey, Point,
        PointerButton, PointerData, UiId, UiScale,
    },
    prelude::UiRect,
    session::UiSession,
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[derive(Default)]
struct Clipboard(Mutex<String>);
impl lgui::services::Clipboard for Clipboard {
    fn read_text(&self) -> Result<Option<String>, lgui::services::ClipboardError> {
        Ok(Some(self.0.lock().unwrap().clone()))
    }
    fn write_text(&self, text: &str) -> Result<(), lgui::services::ClipboardError> {
        *self.0.lock().unwrap() = text.to_owned();
        Ok(())
    }
}
struct Reader {
    session: UiSession,
    view: AppView,
    state: AppContext,
    tasks: super::test_support::UiTasks,
    runtime: Arc<AppRuntime>,
    clipboard: Arc<Clipboard>,
    context: ApplicationContext,
    rect: UiRect,
}
impl Reader {
    fn new(text: String) -> Self {
        let rect = UiRect::new(0.0, 0.0, 640.0, 340.0);
        let mut app = AppState::new();
        app.workspace.open_path("reader-test.md".into(), text);
        let view = app.workspace.active().unwrap();
        app.workspace.set_presentation(view, Presentation::Markdown);
        let initial = Arc::new(Mutex::new(Some(app)));
        let exposed = Arc::new(Mutex::new(None));
        let output = exposed.clone();
        let runtime = Arc::new(AppRuntime::default());
        let runtime_view = runtime.clone();
        let tasks = super::test_support::UiTasks::default();
        let application = tasks.application();
        let clipboard = Arc::new(Clipboard::default());
        let copy = clipboard.clone();
        let context = ApplicationContext::empty(Default::default());
        let view: AppView = Arc::new(move |cx| {
            let rect = cx.viewport();
            let state =
                AppContext::detached(cx.state_with(|| initial.lock().unwrap().take().unwrap()))
                    .with_runtime(&runtime_view);
            if runtime_view.markdown().is_none() {
                runtime_view.install_markdown(MarkdownService::new(
                    state.clone(),
                    application.clone(),
                    copy.clone(),
                ));
            }
            *output.lock().unwrap() = Some(state.clone());
            let id = UiId::new("file-markdown-test");
            let focus = cx.focus_handle(id.clone());
            super::markdown_view::render(PaneId::new(1), rect, state, id, focus, copy.clone())
        });
        let mut session = UiSession::new();
        session.render_view(&view, rect, UiScale::ONE);
        let state = exposed.lock().unwrap().clone().unwrap();
        state.update(|_| {});
        Self {
            session,
            view,
            state,
            tasks,
            runtime,
            clipboard,
            context,
            rect,
        }
    }
    fn render(&mut self) {
        self.tasks.run_all();
        self.session
            .render_view(&self.view, self.rect, UiScale::ONE);
    }
    fn wait(&mut self, mut predicate: impl FnMut(&Self) -> bool) {
        let limit = Instant::now() + Duration::from_secs(10);
        loop {
            self.render();
            if predicate(self) {
                return;
            }
            assert!(
                Instant::now() < limit,
                "reader did not become ready; tree={:?}",
                self.session
                    .tree()
                    .nodes()
                    .iter()
                    .filter_map(|node| node.text.as_deref())
                    .collect::<Vec<_>>()
            );
            std::thread::sleep(Duration::from_millis(3));
        }
    }
    fn text_rect(&self, wanted: &str) -> Option<UiRect> {
        self.session
            .tree()
            .nodes()
            .iter()
            .find(|node| {
                node.semantics
                    .as_ref()
                    .and_then(|semantics| semantics.value.as_deref())
                    == Some(wanted)
            })
            .map(|node| node.layout_rect)
    }
    fn send(&mut self, event: InputEvent) {
        let output = self.session.handle_input(event);
        lgui::core::dispatch_runtime_output(
            output,
            &self.context,
            &lgui::window::WindowId::new("file-markdown-test"),
            |action| self.session.handle_default_action(action),
            |_| {},
        );
        self.render();
    }
    fn focus(&mut self, word: &str) {
        let rect = self.text_rect(word).unwrap();
        let pointer =
            PointerData::mouse(Point::new(rect.left + 1.0, (rect.top + rect.bottom) * 0.5));
        self.send(InputEvent::PointerDown {
            pointer,
            button: PointerButton::Left,
        });
        self.send(InputEvent::PointerUp {
            pointer,
            button: PointerButton::Left,
        });
    }
    fn key(&mut self, key: &str, modifiers: KeyModifiers) {
        self.send(InputEvent::Keyboard(KeyboardEvent {
            state: KeyState::Down,
            key: LogicalKey::Character(key.into()),
            modifiers,
            ..Default::default()
        }));
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.runtime.clear_markdown();
    }
}
#[test]
fn reader_copies_beyond_the_viewport_and_blocks_text_and_ime_edits() {
    let source = format!(
        "# Heading\n\n{}",
        (0..600)
            .map(|line| format!("**line {line:03}** 中文 e\u{301} 😀\n\n"))
            .collect::<String>()
    );
    let expected = parse_document(
        Arc::new(source.clone()),
        ContentFormat::Markdown,
        SyntaxOptions::default(),
        ParseLimits::automatic(),
        || false,
    )
    .unwrap()
    .display_text;
    let mut reader = Reader::new(source.clone());
    reader.wait(|reader| reader.text_rect("Heading").is_some());
    reader.focus("Heading");
    reader.send(InputEvent::TextInput("forbidden".into()));
    reader.send(InputEvent::Ime(ImeEvent::Preedit {
        text: "拼".into(),
        cursor: None,
    }));
    reader.send(InputEvent::Ime(ImeEvent::Commit("拼音".into())));
    let app = reader.state.get();
    let id = app.workspace.active_document().unwrap();
    assert_eq!(app.workspace.documents[&id].buffer.text(), source);
    assert!(app.editor.ime.is_none());
    drop(app);
    reader.key("a", KeyModifiers::CONTROL);
    reader.key("c", KeyModifiers::CONTROL);
    assert_eq!(*reader.clipboard.0.lock().unwrap(), expected);
    assert!(expected.contains("599"));
    assert!(
        reader.session.tree().nodes().len() < 700,
        "visible scene should stay bounded"
    );
}
#[test]
fn current_buffer_reparses_and_old_publications_cannot_drive_new_events() {
    let mut reader = Reader::new("# Before\n\nbody".into());
    reader.wait(|reader| reader.text_rect("Before").is_some());
    let app = reader.state.get();
    let view = app.workspace.active().unwrap();
    let service = reader.runtime.markdown().unwrap();
    let before = service
        .render
        .latest(crate::infrastructure::markdown_worker::Consumer::Document(
            view,
        ))
        .unwrap();
    drop(app);
    reader.state.update(|app| {
        let document = app.workspace.active_document().unwrap();
        let mut editor = app.workspace.document_editor_mut(document).unwrap();
        editor.select_all();
        editor
            .replace_selection(
                "# After\n\nnew body",
                loom_editor_core::text::EditKind::Other,
            )
            .unwrap();
    });
    reader.wait(|reader| reader.text_rect("After").is_some());
    let after = service
        .render
        .latest(crate::infrastructure::markdown_worker::Consumer::Document(
            view,
        ))
        .unwrap();
    assert_ne!(before.frame.parse, after.frame.parse);
    assert!(!service.render.is_current(
        crate::infrastructure::markdown_worker::Consumer::Document(view),
        before.frame
    ));
    assert!(after.layout.document.display_text.contains("new body"));
}

#[test]
fn side_preview_association_survives_session_with_runtime_ids_reallocated() {
    let mut app = AppState::new();
    app.workspace
        .open_path("session.md".into(), "# session".into());
    assert!(super::markdown_view::side(&mut app));
    let preview = app.workspace.active().unwrap();
    assert!(super::markdown_view::side(&mut app));
    assert_eq!(app.workspace.active(), Some(preview));
    assert_eq!(app.workspace.views.len(), 2);
    let saved = super::workspace_session::saved_editor_layout(&app.workspace).unwrap();
    let wire = serde_json::to_string(&saved).unwrap();
    let decoded = serde_json::from_str(&wire).unwrap();
    let mut restored = AppState::new();
    super::workspace_session::hydrate_editor_layout_with(&mut restored, Some(decoded), |_| {
        Ok("# session".into())
    });
    let view = restored
        .workspace
        .views
        .values()
        .find(|view| view.presentation == Presentation::Markdown)
        .copied()
        .unwrap();
    let source = view.associated_source.unwrap();
    let pane = restored.workspace.pane_of(source).unwrap();
    restored.workspace.activate(pane, source);
    assert!(super::markdown_view::side(&mut restored));
    assert_eq!(restored.workspace.active(), Some(view.id));
    assert_eq!(restored.workspace.views.len(), 2);
}

#[test]
fn dragging_past_the_bottom_continues_scrolling_until_release() {
    let source = format!(
        "# Heading\n\n{}",
        (0..200)
            .map(|line| format!("paragraph {line} 中文 content\n\n"))
            .collect::<String>()
    );
    let mut reader = Reader::new(source);
    reader.wait(|reader| reader.text_rect("Heading").is_some());
    let rect = reader.text_rect("Heading").unwrap();
    let start = PointerData::mouse(Point::new(rect.left + 1.0, rect.top + 8.0));
    reader.send(InputEvent::PointerDown {
        pointer: start,
        button: PointerButton::Left,
    });
    let below = PointerData::mouse(Point::new(rect.left + 20.0, reader.rect.bottom + 70.0));
    reader.send(InputEvent::PointerMove(below));
    let view = reader.state.get().workspace.active().unwrap();
    let first = reader
        .state
        .get()
        .workspace
        .view(view)
        .unwrap()
        .reader
        .source_byte;
    let until = Instant::now() + Duration::from_millis(180);
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
        reader.render();
    }
    let after = reader
        .state
        .get()
        .workspace
        .view(view)
        .unwrap()
        .reader
        .source_byte;
    assert!(after > first, "held drag should continue across frames");
    reader.send(InputEvent::PointerUp {
        pointer: below,
        button: PointerButton::Left,
    });
    let service = reader.runtime.markdown().unwrap();
    assert!(!service.drag_timers.lock().unwrap().contains_key(
        &crate::infrastructure::markdown_worker::Consumer::Document(view)
    ));
    assert!(
        reader.state.get().editor.markdown_views[&view]
            .pressed
            .is_none()
    );
}

#[test]
fn oversized_block_is_explicit_and_does_not_hide_following_content_or_full_copy() {
    let source = format!("# Heading\n\n{}\n\nTail content", "中文 huge ".repeat(3000));
    let expected = parse_document(
        Arc::new(source.clone()),
        ContentFormat::Markdown,
        SyntaxOptions::default(),
        ParseLimits::automatic(),
        || false,
    )
    .unwrap()
    .display_text;
    let mut reader = Reader::new(source);
    reader.wait(|reader| reader.text_rect("Tail").is_some());
    assert!(reader.session.tree().nodes().iter().any(|node| {
        node.text
            .as_deref()
            .is_some_and(|text| text.contains("Large block exceeds"))
    }));
    reader.focus("Heading");
    reader.key("a", KeyModifiers::CONTROL);
    reader.key("c", KeyModifiers::CONTROL);
    assert_eq!(*reader.clipboard.0.lock().unwrap(), expected);
}

fn draw(reader: &mut Reader, surface: &mut lgui::render_skia::SkiaSoftwareSurface) {
    let commit = reader
        .session
        .render_view(&reader.view, reader.rect, UiScale::ONE);
    let frame = lgui::renderer::FrameInfo::new(
        lgui::core::PhysicalRect::new(
            0,
            0,
            reader.rect.width() as i32,
            reader.rect.height() as i32,
        ),
        &[],
        UiScale::ONE,
        lgui::renderer::FrameReason::SceneChange,
        true,
    );
    lgui::assets::backend::with_render_resources_unscoped(
        lgui::assets::RenderResources::new(),
        || surface.draw(&commit.scene, &frame),
    )
    .unwrap();
}
fn save_scene(reader: &mut Reader, path: &std::path::Path) {
    let mut surface = lgui::render_skia::SkiaSoftwareSurface::new(32 * 1024 * 1024);
    draw(reader, &mut surface);
    let (w, h) = surface.size();
    let mut pixels = surface.pixels().to_vec();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    image::save_buffer(path, &pixels, w as u32, h as u32, image::ColorType::Rgba8).unwrap();
}
fn percentile(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() as f64 * 0.95).ceil() as usize).saturating_sub(1)]
}

/// Opt-in real production parse/layout/UI/Skia evidence. It uses an offscreen
/// host, so it does not claim OS input/compositor or native GPU frame latency.
#[test]
#[ignore = "writes reproducible production screenshots and performance evidence"]
fn production_markdown_evidence() {
    let output = std::env::var_os("LOOM_MARKDOWN_EVIDENCE_DIR")
        .map(std::path::PathBuf::from)
        .expect("set LOOM_MARKDOWN_EVIDENCE_DIR");
    std::fs::create_dir_all(&output).unwrap();
    let rich = "# 通用 Markdown 预览\n\n中文无需空格即可自然换行。English **bold** *italic* e\u{301} 😀 [OpenAI](https://openai.com)。\n\n> 阅读界面与源码编辑共享同一份文本，视图各自保留位置。\n\n- [x] 文档与卡片复用\n- [ ] 下一项任务\n\n| Capability | 状态 |\n|:---|---:|\n| 逻辑复制 | 支持 |\n| 中文 / Emoji | 支持 |\n\n\x60\x60\x60rust\nfn main() {\n\tprintln!(\"Hello 中文 😀\");\n}\n\x60\x60\x60\n\n<div>HTML is displayed literally.</div>\n";
    let mut visual = Reader::new(rich.into());
    visual.rect = UiRect::new(0.0, 0.0, 960.0, 760.0);
    visual.wait(|r| r.text_rect("通用").is_some());
    save_scene(&mut visual, &output.join("reader-rich.png"));
    let readme =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap();
    let mut actual = Reader::new(readme);
    actual.rect = UiRect::new(0.0, 0.0, 960.0, 760.0);
    actual.wait(|r| {
        r.runtime
            .markdown()
            .unwrap()
            .render
            .latest(crate::infrastructure::markdown_worker::Consumer::Document(
                r.state.get().workspace.active().unwrap(),
            ))
            .is_some()
    });
    save_scene(&mut actual, &output.join("reader-readme.png"));
    let mut source = String::from("# Heading\n\n");
    let mut count = 0;
    while source.len() < 64 * 1024 {
        source.push_str(&format!("Paragraph {count:04}: 中文与 English 混排 **bold** e\u{301} 😀. This paragraph exercises wrapping and complete document layout.\n\n"));
        count += 1;
    }
    let mut warm = Reader::new(source.clone());
    warm.wait(|r| r.text_rect("Heading").is_some());
    drop(warm);
    let mut first = Vec::new();
    for _ in 0..9 {
        let start = Instant::now();
        let mut reader = Reader::new(source.clone());
        reader.wait(|r| r.text_rect("Heading").is_some());
        first.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let mut reader = Reader::new(source.clone());
    reader.wait(|r| r.text_rect("Heading").is_some());
    let mut edit = Vec::new();
    for sample in 0..9 {
        let marker = format!("Increment {sample}");
        let start = Instant::now();
        reader.state.update(|app| {
            let id = app.workspace.active_document().unwrap();
            let mut editor = app.workspace.document_editor_mut(id).unwrap();
            let end = editor.buffer().len();
            editor.set_cursor(end);
            editor
                .replace_selection(
                    &format!("\n\n{marker}"),
                    loom_editor_core::text::EditKind::Other,
                )
                .unwrap();
        });
        reader.wait(|r| {
            let app = r.state.get();
            let id = app.workspace.active().unwrap();
            r.runtime
                .markdown()
                .unwrap()
                .render
                .latest(crate::infrastructure::markdown_worker::Consumer::Document(
                    id,
                ))
                .is_some_and(|value| value.layout.document.source.contains(&marker))
        });
        edit.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let mut frames = Vec::new();
    let mut surface = lgui::render_skia::SkiaSoftwareSurface::new(32 * 1024 * 1024);
    draw(&mut reader, &mut surface);
    for sample in 0..30 {
        let byte = sample * source.len() / 32;
        reader.state.update(|app| {
            let view = app.workspace.active().unwrap();
            app.workspace.set_reader_position(
                view,
                crate::domain::view::ReaderPosition {
                    source_byte: byte,
                    block_fraction: 0.0,
                },
            );
        });
        reader.render();
        reader.wait(|r| {
            let app = r.state.get();
            let view = app.workspace.active().unwrap();
            r.runtime
                .markdown()
                .unwrap()
                .render
                .latest(crate::infrastructure::markdown_worker::Consumer::Document(
                    view,
                ))
                .is_some_and(|result| {
                    result.key.position == app.workspace.view(view).unwrap().reader
                })
        });
        let start = Instant::now();
        draw(&mut reader, &mut surface);
        frames.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let report = serde_json::json!({"platform":std::env::consts::OS,"arch":std::env::consts::ARCH,"profile":if cfg!(debug_assertions){"dev"}else{"release"},"method":"warm fonts; production runtime + UiSession; software Skia; offscreen, not desktop compositor; nearest rank p95","source_bytes":source.len(),"first_readable_ms":first,"first_readable_p95_ms":percentile(&mut first),"edit_to_preview_ms":edit,"edit_to_preview_p95_ms":percentile(&mut edit),"ui_projection_and_software_draw_ms":frames,"ui_projection_and_software_draw_p95_ms":percentile(&mut frames),"samples":{"first":9,"edit":9,"frames":30},"limits":"retained Rust/native cache estimates are leased; not process RSS or native transient allocation"});
    std::fs::write(
        output.join("production-baseline.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("{report}");
}

#[test]
fn copy_code_button_writes_complete_code_without_editing_source() {
    let source = "# Heading\n\n\x60\x60\x60rust\nlet 中文 = 1;\n\x60\x60\x60";
    let mut reader = Reader::new(source.into());
    reader.wait(|reader| reader.text_rect("Heading").is_some());
    let bounds = reader
        .session
        .tree()
        .nodes()
        .iter()
        .find(|node| {
            node.semantics
                .as_ref()
                .and_then(|semantics| semantics.name.as_deref())
                == Some("Copy code")
        })
        .unwrap()
        .layout_rect;
    let pointer = PointerData::mouse(Point::new(bounds.left + 5.0, bounds.top + 5.0));
    reader.send(InputEvent::PointerDown {
        pointer,
        button: PointerButton::Left,
    });
    reader.send(InputEvent::PointerUp {
        pointer,
        button: PointerButton::Left,
    });
    assert_eq!(*reader.clipboard.0.lock().unwrap(), "let 中文 = 1;\n");
    let app = reader.state.get();
    assert_eq!(
        app.workspace.documents[&app.workspace.active_document().unwrap()]
            .buffer
            .text(),
        source
    );
}

#[test]
fn stale_frame_release_cancels_the_gesture_without_opening_a_link() {
    let mut reader = Reader::new("# Heading\n\n[link](https://example.com)\n\nbody\n\nmore".into());
    reader.wait(|r| r.text_rect("Heading").is_some());
    let service = reader.runtime.markdown().unwrap();
    let view = reader.state.get().workspace.active().unwrap();
    let old = service
        .render
        .latest(crate::infrastructure::markdown_worker::Consumer::Document(
            view,
        ))
        .unwrap();
    let generation = reader
        .state
        .get()
        .workspace
        .view(view)
        .unwrap()
        .input_generation;
    reader.state.update(|app| {
        let state = app.editor.markdown_views.entry(view).or_default();
        state.pressed = Some((old.frame.parse, DisplayByte(0), None));
        state.drag_point = Some((20.0, 400.0));
    });
    reader.rect = UiRect::new(0.0, 0.0, 560.0, 340.0);
    reader.render();
    reader.wait(|r| {
        service
            .render
            .latest(crate::infrastructure::markdown_worker::Consumer::Document(
                view,
            ))
            .is_some_and(|result| result.frame != old.frame)
            && r.text_rect("Heading").is_some()
    });
    super::markdown_view::event(
        &reader.state,
        PaneId::new(1),
        view,
        generation,
        &old,
        crate::features::markdown::reader::ReaderEvent::PointerUp {
            key: old.frame,
            hit: None,
        },
        reader.clipboard.clone(),
    );
    assert!(
        reader.state.get().editor.markdown_views[&view]
            .pressed
            .is_none()
    );
    assert!(
        reader.state.get().editor.markdown_views[&view]
            .drag_point
            .is_none()
    );
    reader.state.update(|app| app.editor.focused = false);
    assert!(!super::markdown_view::key_action(
        &reader.state,
        crate::features::keymap::action::Action::SelectAll,
        reader.clipboard.clone()
    ));
    assert!(
        !super::keymap_context::context_stack(&reader.state.get())
            .iter()
            .any(|context| context.primary() == Some("Markdown"))
    );
}

#[test]
fn visible_snippets_load_known_wasm_resources_and_unknown_languages_remain_plain() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/language-packs");
    let packages = Arc::new(crate::infrastructure::language_packs::Packages::discover(
        &root, None,
    ));
    let mut highlighter = crate::features::syntax::SnippetHighlighter::new(packages.clone());
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    assert!(
        highlighter
            .highlight("unknown-loom-language", "literal", &cancelled)
            .is_empty()
    );
    assert_eq!(packages.activity().wasm_loads, 0);
    let text = "{\"中文\": true, \"count\": 123}";
    let spans = highlighter.highlight("json", text, &cancelled);
    if !root.join("json").is_dir() {
        assert!(
            spans.is_empty(),
            "standalone host must fall back without language artifacts"
        );
        return;
    }
    assert!(spans.iter().any(|span| span.class.is_some()));
    assert!(
        spans
            .iter()
            .all(|span| text.is_char_boundary(span.range.start)
                && text.is_char_boundary(span.range.end)
                && span.range.end <= text.len())
    );
    assert_eq!(packages.activity().wasm_loads, 1);
    assert_eq!(highlighter.highlight("json", text, &cancelled), spans);
    assert_eq!(packages.activity().wasm_loads, 1);
}
