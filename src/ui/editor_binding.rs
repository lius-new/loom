//! A viewport reads its own immutable scene and emits application intents.
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use lgui::core::PointerButton;
use lgui::prelude::{Element, UiRect};
use loom_editor_core::identity::{FileId, PaneId};
use loom_editor_core::text::{Editor, Selection, TextBuffer};
use loom_editor_core::vim::CursorShape;

use crate::domain::document::DocumentStamp;
use crate::domain::language::ResolvedLanguage;
use crate::domain::line_change::LineChange;

pub(crate) struct DocumentMeta {
    pub path: PathBuf,
    pub lang: ResolvedLanguage,
}
pub(crate) struct ViewDocument {
    pub id: FileId,
    /// Names the exact document lifetime, text and language this view shows.
    pub stamp: DocumentStamp,
    pub meta: DocumentMeta,
    pub buffer: TextBuffer,
    pub selection: Selection,
    pub scroll: (f32, f32),
}
impl ViewDocument {
    pub fn editor(&self) -> Editor<'_> {
        Editor::new(&self.buffer, self.selection)
    }
}
pub(crate) struct Preedit {
    pub pane: PaneId,
    pub document: FileId,
    pub text: String,
    pub cursor: Option<Range<usize>>,
}
pub(crate) struct ViewState {
    pub pane: PaneId,
    pub document: Option<ViewDocument>,
    pub focused_pane: bool,
    pub focused: bool,
    pub accepts_text: bool,
    pub cursor_shape: CursorShape,
    pub selections: Vec<Range<usize>>,
    pub search_matches: Vec<Range<usize>>,
    /// What the language server reports for this exact text.
    pub diagnostics: Arc<[crate::features::language_service::diagnostics::Diagnostic]>,
    pub smooth_caret: bool,
    pub dragging: bool,
    pub caret_dragging: bool,
    pub preedit: Option<Preedit>,
    pub editor_gutter_hover: Option<(PaneId, usize)>,
    pub editor_diagnostic_hover: Option<(PaneId, Range<usize>)>,
    pub editor_hovered: Option<PaneId>,
    pub editor_vertical_scrollbar_dragging: bool,
    pub editor_horizontal_scrollbar_dragging: bool,
    pub line_changes: BTreeMap<usize, LineChange>,
    pub overview_marks: Vec<OverviewMark>,
    pub has_repository: bool,
    pub peek_visible: bool,
    /// What the language servers said about the word the pointer rests on.
    pub hover: Option<HoverView>,
    /// The list open over the code, if any.
    pub popup: Option<crate::ui::editor::popup::View>,
}

/// A server hover text, anchored to the text it describes.
#[derive(Clone)]
pub(crate) struct HoverView {
    pub anchor: Range<usize>,
    pub blocks: Arc<[crate::features::markdown::Block]>,
}
/// One adjacent run of changes in the overview ruler, with a zero-based first line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OverviewMark {
    pub first: usize,
    pub lines: usize,
    pub change: LineChange,
}
impl ViewState {
    pub fn preedit_for(&self, pane: PaneId, document: FileId) -> Option<&Preedit> {
        self.preedit.as_ref().filter(|value| {
            value.pane == pane && value.document == document && !value.text.is_empty()
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Metrics {
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub content_w: f32,
    pub content_h: f32,
    pub max_x: f32,
    pub max_y: f32,
}
pub(crate) enum Event {
    PointerDown {
        document: FileId,
        point: (f32, f32),
        button: PointerButton,
        code_left: f32,
        metrics: Metrics,
    },
    PointerDrag((f32, f32)),
    PointerUp,
    Wheel {
        x: f32,
        y: f32,
        metrics: Metrics,
    },
    GutterHover(Option<(PaneId, usize)>),
    /// The range of the diagnostic now under the pointer, if any.
    DiagnosticHover(Option<(PaneId, Range<usize>)>),
    Peek {
        line: usize,
    },
    /// The pointer moved over the code area.
    HoverMove {
        point: (f32, f32),
        code_left: f32,
        metrics: Metrics,
    },
    /// The pointer left the code area.
    HoverLeave,
    /// The mouse did something in the popup list.
    Popup(crate::ui::editor::popup::Event),
    Text(String),
    Composition {
        text: String,
        cursor: Option<Range<usize>>,
    },
    CompositionEnd,
    Focus,
    Blur,
    ScrollbarDown {
        vertical: bool,
        offset: f32,
        position: Option<f32>,
    },
    ScrollbarDrag {
        vertical: bool,
        pointer: f32,
        start: f32,
        travel: f32,
        max: f32,
    },
    ScrollbarUp {
        vertical: bool,
    },
}
pub(crate) struct PeekPlacement {
    pub rect: UiRect,
    pub code_left: f32,
    pub scroll_y: f32,
    pub metrics: Metrics,
}
type HighlightProvider = dyn Fn(
        &ViewDocument,
        Vec<crate::features::syntax::Window>,
    ) -> Vec<crate::features::syntax::WindowTokens>
    + Send
    + Sync;

#[derive(Clone)]
pub(crate) struct Binding {
    read: Arc<dyn Fn() -> ViewState + Send + Sync>,
    dispatch: Arc<dyn Fn(Event) + Send + Sync>,
    overlay: Arc<dyn Fn(PeekPlacement) -> Option<Element> + Send + Sync>,
    highlights: Arc<HighlightProvider>,
}
impl Binding {
    pub fn new(
        read: impl Fn() -> ViewState + Send + Sync + 'static,
        dispatch: impl Fn(Event) + Send + Sync + 'static,
        overlay: impl Fn(PeekPlacement) -> Option<Element> + Send + Sync + 'static,
    ) -> Self {
        Self {
            read: Arc::new(read),
            dispatch: Arc::new(dispatch),
            overlay: Arc::new(overlay),
            highlights: Arc::new(|_, _| Vec::new()),
        }
    }
    pub fn get(&self) -> ViewState {
        (self.read)()
    }
    pub fn with_highlights(
        mut self,
        provider: impl Fn(
            &ViewDocument,
            Vec<crate::features::syntax::Window>,
        ) -> Vec<crate::features::syntax::WindowTokens>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.highlights = Arc::new(provider);
        self
    }
    pub fn highlights(
        &self,
        document: &ViewDocument,
        windows: Vec<crate::features::syntax::Window>,
    ) -> Vec<crate::features::syntax::WindowTokens> {
        (self.highlights)(document, windows)
    }
    pub fn send(&self, event: Event) {
        (self.dispatch)(event);
    }
    pub fn overlay(&self, placement: PeekPlacement) -> Option<Element> {
        (self.overlay)(placement)
    }
}
