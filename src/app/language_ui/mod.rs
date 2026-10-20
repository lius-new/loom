//! What the editor does with language servers while the user works: hover,
//! navigation, completion and the rest. Each feature keeps its state in
//! [`LanguageUi`], asks the servers through the request channel, and is advanced
//! on the UI thread by [`tick`] when answers arrive.
pub(crate) mod actions;
pub(crate) mod hover;
pub(crate) mod keys;
pub(crate) mod navigate;
pub(crate) mod popup;

use super::context::AppContext;
use super::state::AppState;
use crate::domain::document::DocumentStamp;
use crate::domain::identity::{FileId, PaneId};
use crate::features::language_service::sync::Stamp;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

/// All transient language-server interaction state of the editor.
#[derive(Clone, Default)]
pub struct LanguageUi {
    pub hover: hover::Hover,
    pub history: navigate::History,
    /// The list open over the code, if any.
    pub popup: Option<popup::Popup>,
    /// A question whose answer the person is waiting for.
    pub pending: Option<actions::Pending>,
}

impl LanguageUi {
    /// Closes what is open and drops what is awaited.
    pub fn close_popups(&mut self) {
        self.popup = None;
        self.pending = None;
    }
}

/// The document a view shows, as the servers need to be told about it.
#[derive(Clone)]
pub(crate) struct Target {
    pub pane: PaneId,
    pub document: FileId,
    pub path: PathBuf,
    pub stamp: DocumentStamp,
    pub text: Arc<String>,
    pub cursor: usize,
    pub selection: Option<Range<usize>>,
}

impl Target {
    /// The state the servers know the document in.
    pub fn lsp_stamp(&self) -> Stamp {
        lsp_stamp(&self.stamp)
    }
}

pub(crate) fn lsp_stamp(stamp: &DocumentStamp) -> Stamp {
    Stamp {
        incarnation: stamp.incarnation,
        version: stamp.version,
    }
}

/// The text document `pane` shows, if it is one a server can be asked about.
pub(crate) fn target(app: &AppState, pane: PaneId) -> Option<Target> {
    let id = app.workspace.pane(pane)?.active()?;
    let document = app.workspace.documents.get(&id).filter(|d| d.is_text())?;
    let editor = app.workspace.editor(pane)?;
    Some(Target {
        pane,
        document: id,
        path: document.meta.path.clone(),
        stamp: document.stamp(id),
        text: document.buffer.text_snapshot(),
        cursor: editor.cursor(),
        selection: editor.selection(),
    })
}

/// The language services of the running application, if started.
pub(crate) fn services(
    state: &AppContext,
) -> Option<Arc<super::language_services::LanguageServices>> {
    state
        .services()
        .and_then(|runtime| runtime.language_services())
}

/// Does what a popup row that is not a place stands for.
pub(crate) fn run_payload(
    _state: &AppContext,
    _payload: popup::Payload,
    _rect: lgui::prelude::UiRect,
) {
}

/// Advances every feature that is waiting for a server's answer. Runs on the UI
/// thread whenever the servers report something new.
pub(crate) fn tick(state: &AppContext) {
    let mut effects = Vec::new();
    state.try_update(|app| {
        app.editor.syntax_revision = app.editor.syntax_revision.wrapping_add(1);
        hover::tick(app);
        effects = actions::tick(app);
        true
    });
    for effect in effects {
        match effect {
            actions::Effect::Go(place, rect) => navigate::go(state, place, rect, true),
        }
    }
}
