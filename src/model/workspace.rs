//! Open documents and the editor panes that show them.
//!
//! Documents are shared: a file opened in two panes has one text buffer, one
//! undo history and one dirty state. Each pane keeps its own tab list,
//! preview slot and, per document, its own selection and scroll position.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::git::DiffTarget;
use crate::model::buffer::{Editor, EditorMut, Selection, TextBuffer};
use crate::model::diff_document::DiffDocument;
use crate::model::document::{DiskState, FileId, FileMeta};
use crate::model::pane_layout::{Direction, PaneId, PaneNode};
use lgui::prelude::UiRect;

pub const SETTINGS_TITLE: &str = "Settings";
pub const KEYMAP_TITLE: &str = "Keymap";

/// Built-in pages that open in a tab instead of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppPage {
    Settings,
    Keymap,
}

impl AppPage {
    pub fn title(self) -> &'static str {
        match self {
            AppPage::Settings => SETTINGS_TITLE,
            AppPage::Keymap => KEYMAP_TITLE,
        }
    }
}

/// How one pane shows one document.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewState {
    pub selection: Selection,
    pub scroll_x: f32,
    pub scroll_y: f32,
}

#[derive(Clone)]
struct OpenDocument {
    meta: FileMeta,
    buffer: TextBuffer,
    saved_text: String,
    diff: Option<DiffDocument>,
    /// A built-in page (Settings, Keymap) rather than a file.
    page: Option<AppPage>,
    /// One view per pane that has this document as a tab.
    views: HashMap<PaneId, ViewState>,
    disk_state: DiskState,
    disk_conflict: bool,
    missing_on_disk: bool,
}

impl OpenDocument {
    fn new(meta: FileMeta, contents: String, disk_state: DiskState) -> Self {
        Self {
            meta,
            buffer: TextBuffer::new(contents.clone()),
            saved_text: contents,
            diff: None,
            page: None,
            views: HashMap::new(),
            disk_state,
            disk_conflict: false,
            missing_on_disk: false,
        }
    }

    /// Whether this tab edits a file on disk (not a diff or the settings page).
    fn is_text(&self) -> bool {
        self.diff.is_none() && self.page.is_none()
    }

    /// Diffs and built-in pages exist once; only text files can be shown in
    /// several panes at the same time.
    fn is_shareable(&self) -> bool {
        self.page.is_none() && self.diff.is_none()
    }

    fn editor_mut(&mut self, pane: PaneId) -> Option<EditorMut<'_>> {
        let mut own = None;
        let mut peers = Vec::new();
        for (id, view) in self.views.iter_mut() {
            if *id == pane {
                own = Some(&mut view.selection);
            } else {
                peers.push(&mut view.selection);
            }
        }
        Some(EditorMut::new(&mut self.buffer, own?, peers))
    }

    /// Clamp every view after the text was replaced wholesale.
    fn clamp_views(&mut self) {
        for view in self.views.values_mut() {
            view.selection = self.buffer.clamp(view.selection);
        }
    }
}

/// One editor pane: an ordered tab list with an active and a preview tab.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pane {
    items: Vec<FileId>,
    active: Option<FileId>,
    /// The one replaceable tab of this pane.
    preview: Option<FileId>,
}

impl Pane {
    pub fn items(&self) -> &[FileId] {
        &self.items
    }

    pub fn active(&self) -> Option<FileId> {
        self.active
    }

    pub fn preview(&self) -> Option<FileId> {
        self.preview
    }

    pub fn is_preview(&self, id: FileId) -> bool {
        self.preview == Some(id)
    }

    pub fn contains(&self, id: FileId) -> bool {
        self.items.contains(&id)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Remove a tab, activating its right neighbour (or the new last tab).
    fn remove(&mut self, id: FileId) -> bool {
        let Some(index) = self.items.iter().position(|item| *item == id) else {
            return false;
        };
        self.items.remove(index);
        if self.preview == Some(id) {
            self.preview = None;
        }
        if self.active == Some(id) {
            self.active = self
                .items
                .get(index.min(self.items.len().saturating_sub(1)))
                .copied();
        }
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReconcileResult {
    Unchanged(PathBuf),
    Reloaded(PathBuf),
    Conflict(PathBuf),
    Missing(PathBuf),
    Failed(PathBuf, String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenMode {
    Preview,
    Permanent,
}

#[derive(Clone)]
pub struct Workspace {
    documents: HashMap<FileId, OpenDocument>,
    paths: HashMap<PathBuf, FileId>,
    panes: HashMap<PaneId, Pane>,
    layout: PaneNode,
    active_pane: PaneId,
    /// Panes by recency of focus, most recent first.
    pane_mru: Vec<PaneId>,
    next_file_id: u64,
    next_pane_id: u64,
}

impl Workspace {
    pub fn new() -> Self {
        let pane = PaneId::new(1);
        Self {
            documents: HashMap::new(),
            paths: HashMap::new(),
            panes: HashMap::from([(pane, Pane::default())]),
            layout: PaneNode::Leaf(pane),
            active_pane: pane,
            pane_mru: vec![pane],
            next_file_id: 1,
            next_pane_id: 2,
        }
    }

    // ---- Panes ------------------------------------------------------------

    pub fn layout(&self) -> &PaneNode {
        &self.layout
    }

    pub fn layout_mut(&mut self) -> &mut PaneNode {
        &mut self.layout
    }

    /// Panes in reading order.
    pub fn pane_ids(&self) -> Vec<PaneId> {
        self.layout.leaves()
    }

    pub fn pane_count(&self) -> usize {
        self.panes.len()
    }

    pub fn active_pane(&self) -> PaneId {
        self.active_pane
    }

    pub fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.panes.get(&id)
    }

    fn active_pane_ref(&self) -> &Pane {
        &self.panes[&self.active_pane]
    }

    fn active_pane_mut(&mut self) -> &mut Pane {
        self.panes
            .get_mut(&self.active_pane)
            .expect("the active pane exists")
    }

    /// Focus a pane. Returns whether the focus changed.
    pub fn activate_pane(&mut self, pane: PaneId) -> bool {
        if !self.panes.contains_key(&pane) {
            return false;
        }
        self.pane_mru.retain(|id| *id != pane);
        self.pane_mru.insert(0, pane);
        let changed = self.active_pane != pane;
        self.active_pane = pane;
        changed
    }

    /// The most recently focused of several candidate panes.
    pub fn most_recent_pane(&self, candidates: &[PaneId]) -> Option<PaneId> {
        self.pane_mru
            .iter()
            .copied()
            .find(|id| candidates.contains(id))
            .or_else(|| candidates.first().copied())
    }

    /// Which pane shows `id`, preferring the active pane, then recency.
    pub fn pane_of(&self, id: FileId) -> Option<PaneId> {
        let holders = self
            .pane_ids()
            .into_iter()
            .filter(|pane| self.panes[pane].contains(id))
            .collect::<Vec<_>>();
        if holders.contains(&self.active_pane) {
            return Some(self.active_pane);
        }
        self.most_recent_pane(&holders)
    }

    /// Split `pane`, copying its active tab (with its view) into a new pane on
    /// the `direction` side, and focus the new pane. Built-in pages and diffs
    /// exist once, so a pane showing one splits into an empty pane.
    pub fn split(&mut self, pane: PaneId, direction: Direction) -> Option<PaneId> {
        let source = self.panes.get(&pane)?;
        let copied = source.active.filter(|id| {
            self.documents
                .get(id)
                .is_some_and(OpenDocument::is_shareable)
        });
        let new = PaneId::new(self.next_pane_id);
        self.next_pane_id += 1;
        if !self.layout.split(pane, new, direction) {
            return None;
        }
        self.panes.insert(new, Pane::default());
        if let Some(id) = copied {
            self.insert_item(new, id, 0, Some(pane));
            let new_pane = self.panes.get_mut(&new).expect("pane was just added");
            new_pane.active = Some(id);
        }
        self.activate_pane(new);
        Some(new)
    }

    /// Replace the layout of a workspace without open tabs by `shape`, whose
    /// leaves are numbered `0..` in reading order. Each leaf becomes a new empty
    /// pane; the new panes are returned in that order.
    pub fn restore_layout(&mut self, mut shape: PaneNode) -> Vec<PaneId> {
        if !self.documents.is_empty() {
            return self.pane_ids();
        }
        let count = shape.leaves().len();
        let ids = (0..count)
            .map(|_| {
                let id = PaneId::new(self.next_pane_id);
                self.next_pane_id += 1;
                id
            })
            .collect::<Vec<_>>();
        shape.map_leaves(&mut |leaf| ids[leaf.get() as usize]);
        shape.normalize();
        self.panes = ids.iter().map(|id| (*id, Pane::default())).collect();
        self.layout = shape;
        self.active_pane = ids[0];
        self.pane_mru = ids.clone();
        ids
    }

    /// Split `pane`, leaving the new pane empty and unfocused.
    pub fn split_empty(&mut self, pane: PaneId, direction: Direction) -> Option<PaneId> {
        self.add_pane(pane, direction)
    }

    /// Insert an empty pane next to `pane` without focusing it.
    fn add_pane(&mut self, pane: PaneId, direction: Direction) -> Option<PaneId> {
        let new = PaneId::new(self.next_pane_id);
        if !self.panes.contains_key(&pane) || !self.layout.split(pane, new, direction) {
            return None;
        }
        self.next_pane_id += 1;
        self.panes.insert(new, Pane::default());
        Some(new)
    }

    /// The pane next to the active one in `direction`; among several, the
    /// most recently focused.
    pub fn neighbor(&self, direction: Direction) -> Option<PaneId> {
        // Sizes are proportional, so any area gives the same adjacency.
        const AREA: UiRect = UiRect::new(0.0, 0.0, 10_000.0, 10_000.0);
        let candidates = self.layout.neighbors(AREA, self.active_pane, direction);
        self.most_recent_pane(&candidates)
    }

    /// Focus the neighbouring pane in `direction`.
    pub fn activate_neighbor(&mut self, direction: Direction) -> bool {
        self.neighbor(direction)
            .is_some_and(|pane| self.activate_pane(pane))
    }

    /// Focus the next (or previous) pane in reading order, wrapping around.
    pub fn activate_next_pane(&mut self, forward: bool) -> bool {
        let panes = self.pane_ids();
        let Some(index) = panes.iter().position(|pane| *pane == self.active_pane) else {
            return false;
        };
        let len = panes.len();
        let next = if forward {
            (index + 1) % len
        } else {
            (index + len - 1) % len
        };
        self.activate_pane(panes[next])
    }

    /// Move the active tab to the neighbouring pane in `direction`, creating
    /// that pane when there is none. A pane's only tab stays put when there is
    /// no neighbour, since splitting would just move the whole pane.
    pub fn move_active_item(&mut self, direction: Direction) -> bool {
        let from = self.active_pane;
        let Some(id) = self.active() else {
            return false;
        };
        let target = match self.neighbor(direction) {
            Some(pane) => pane,
            None if self.active_pane_ref().items.len() > 1 => {
                match self.add_pane(from, direction) {
                    Some(pane) => pane,
                    None => return false,
                }
            }
            None => return false,
        };
        self.move_item(from, id, target, usize::MAX)
    }

    /// Close a pane and all its tabs without prompting; callers resolve dirty
    /// documents first. The last pane only loses its tabs.
    pub fn close_pane(&mut self, pane: PaneId) {
        let Some(items) = self.panes.get(&pane).map(|p| p.items.clone()) else {
            return;
        };
        for id in items {
            self.close_item(pane, id);
        }
        self.remove_pane_if_empty(pane);
    }

    fn remove_pane_if_empty(&mut self, pane: PaneId) {
        if self.panes.len() <= 1 || !self.panes.get(&pane).is_some_and(Pane::is_empty) {
            return;
        }
        if !self.layout.remove(pane) {
            return;
        }
        self.panes.remove(&pane);
        self.pane_mru.retain(|id| *id != pane);
        if self.active_pane == pane {
            let next = self
                .pane_mru
                .first()
                .copied()
                .unwrap_or_else(|| self.layout.leaves()[0]);
            self.activate_pane(next);
        }
    }

    /// Insert a reference to an open document into a pane, creating its view
    /// from `view_source`'s view (or the most recently focused one).
    fn insert_item(&mut self, pane: PaneId, id: FileId, index: usize, view_source: Option<PaneId>) {
        let Some(target) = self.panes.get(&pane) else {
            return;
        };
        if target.contains(id) {
            return;
        }
        let holders = self
            .pane_mru
            .iter()
            .copied()
            .filter(|p| {
                self.documents
                    .get(&id)
                    .is_some_and(|d| d.views.contains_key(p))
            })
            .collect::<Vec<_>>();
        let source = view_source
            .filter(|p| holders.contains(p))
            .or_else(|| holders.first().copied());
        let Some(document) = self.documents.get_mut(&id) else {
            return;
        };
        let view = source
            .and_then(|p| document.views.get(&p).copied())
            .unwrap_or_default();
        document.views.insert(pane, view);
        let target = self.panes.get_mut(&pane).expect("pane exists");
        target.items.insert(index.min(target.items.len()), id);
    }

    /// Put a tab into a new pane on `direction`'s side of `target`, moving it
    /// (or copying a file when `copy`). Splitting a pane with its only tab is
    /// a no-op, like VSCode.
    pub fn split_with_item(
        &mut self,
        from: PaneId,
        id: FileId,
        target: PaneId,
        direction: Direction,
        copy: bool,
    ) -> bool {
        if !self.panes.get(&from).is_some_and(|p| p.contains(id)) {
            return false;
        }
        let copy = copy
            && self
                .documents
                .get(&id)
                .is_some_and(OpenDocument::is_shareable);
        if !copy && from == target && self.panes[&from].items.len() == 1 {
            return false;
        }
        let Some(new) = self.add_pane(target, direction) else {
            return false;
        };
        if copy {
            self.copy_item(from, id, new, 0)
        } else {
            self.move_item(from, id, new, 0)
        }
    }

    /// Move a tab to another pane (or reorder within one) at `index`, and
    /// activate it there. A pane left empty is removed.
    pub fn move_item(&mut self, from: PaneId, id: FileId, to: PaneId, index: usize) -> bool {
        if from == to {
            return self.move_tab(from, id, index);
        }
        if !self.panes.get(&from).is_some_and(|p| p.contains(id)) || !self.panes.contains_key(&to) {
            return false;
        }
        let was_preview = self.panes[&from].is_preview(id);
        if self.panes[&to].contains(id) {
            self.panes.get_mut(&from).unwrap().remove(id);
            if let Some(document) = self.documents.get_mut(&id) {
                document.views.remove(&from);
            }
        } else {
            self.insert_item(to, id, index, Some(from));
            self.panes.get_mut(&from).unwrap().remove(id);
            if let Some(document) = self.documents.get_mut(&id) {
                document.views.remove(&from);
            }
            if was_preview {
                self.replace_preview_slot(to, id);
            }
        }
        self.panes.get_mut(&to).unwrap().active = Some(id);
        self.activate_pane(to);
        self.remove_pane_if_empty(from);
        true
    }

    /// Copy a tab into another pane at `index` and activate it there.
    /// Built-in pages and diffs cannot be copied.
    pub fn copy_item(&mut self, from: PaneId, id: FileId, to: PaneId, index: usize) -> bool {
        if from == to
            || !self.panes.get(&from).is_some_and(|p| p.contains(id))
            || !self.panes.contains_key(&to)
            || !self
                .documents
                .get(&id)
                .is_some_and(OpenDocument::is_shareable)
        {
            return false;
        }
        self.insert_item(to, id, index, Some(from));
        self.panes.get_mut(&to).unwrap().active = Some(id);
        self.activate_pane(to);
        true
    }

    /// Give `pane`'s preview slot to `id`, closing a clean previous preview.
    fn replace_preview_slot(&mut self, pane: PaneId, id: FileId) {
        if let Some(old) = self.panes[&pane].preview.filter(|old| *old != id) {
            if self.is_dirty(old) {
                self.panes.get_mut(&pane).unwrap().preview = None;
            } else {
                self.close_item(pane, old);
            }
        }
        self.panes.get_mut(&pane).unwrap().preview = Some(id);
    }

    // ---- Tabs of one pane ---------------------------------------------------

    /// Labels for a pane's tabs. Duplicate file names receive the shortest
    /// parent path suffix that distinguishes them within that pane.
    pub fn tab_labels(&self, pane: PaneId) -> HashMap<FileId, String> {
        let Some(pane) = self.panes.get(&pane) else {
            return HashMap::new();
        };
        let mut groups: HashMap<String, Vec<(FileId, &Path)>> = HashMap::new();
        for id in &pane.items {
            let Some(document) = self.documents.get(id) else {
                continue;
            };
            groups
                .entry(document.meta.name.clone())
                .or_default()
                .push((*id, document.meta.path.as_path()));
        }

        let mut labels = HashMap::with_capacity(pane.items.len());
        for (name, documents) in groups {
            if documents.len() == 1 {
                labels.insert(documents[0].0, name);
                continue;
            }

            let parents = documents
                .iter()
                .map(|(_, path)| parent_components(path))
                .collect::<Vec<_>>();
            for (index, (id, _)) in documents.iter().enumerate() {
                let parts = &parents[index];
                let mut distinguishing = String::new();
                for depth in 1..=parts.len().max(1) {
                    let candidate = parent_suffix(parts, depth);
                    let normalized = candidate.to_lowercase();
                    let unique = parents.iter().enumerate().all(|(other_index, other)| {
                        other_index == index
                            || parent_suffix(other, depth).to_lowercase() != normalized
                    });
                    distinguishing = candidate;
                    if unique {
                        break;
                    }
                }
                let label = if distinguishing.is_empty() {
                    name.clone()
                } else {
                    format!("{name} — {distinguishing}")
                };
                labels.insert(*id, label);
            }
        }
        labels
    }

    /// Activate a tab of a pane and focus that pane.
    pub fn activate(&mut self, pane: PaneId, id: FileId) -> bool {
        let Some(target) = self.panes.get_mut(&pane) else {
            return false;
        };
        if !target.contains(id) {
            return false;
        }
        let changed = target.active != Some(id) || self.active_pane != pane;
        target.active = Some(id);
        self.activate_pane(pane);
        changed
    }

    /// Reveal an open document: activate it in the active pane if it is there,
    /// otherwise in the most recent pane that shows it.
    pub fn set_active(&mut self, id: FileId) {
        if let Some(pane) = self.pane_of(id) {
            self.activate(pane, id);
        }
    }

    pub fn activate_at(&mut self, index: usize) -> bool {
        let Some(id) = self.active_pane_ref().items.get(index).copied() else {
            return false;
        };
        let pane = self.active_pane_mut();
        let changed = pane.active != Some(id);
        pane.active = Some(id);
        changed
    }

    pub fn activate_last(&mut self) -> bool {
        let Some(id) = self.active_pane_ref().items.last().copied() else {
            return false;
        };
        let pane = self.active_pane_mut();
        let changed = pane.active != Some(id);
        pane.active = Some(id);
        changed
    }

    pub fn next(&mut self) {
        self.cycle(1);
    }

    pub fn prev(&mut self) {
        self.cycle(-1);
    }

    fn cycle(&mut self, step: isize) {
        let pane = self.active_pane_mut();
        if let Some(position) = pane
            .active
            .and_then(|active| pane.items.iter().position(|id| *id == active))
        {
            let len = pane.items.len() as isize;
            pane.active = Some(pane.items[(position as isize + step).rem_euclid(len) as usize]);
        }
    }

    /// Move a tab to a final index within its pane while preserving its
    /// document and active state. Returns whether the visible order changed.
    pub fn move_tab(&mut self, pane: PaneId, id: FileId, target_index: usize) -> bool {
        let Some(pane) = self.panes.get_mut(&pane) else {
            return false;
        };
        let Some(source_index) = pane.items.iter().position(|&file| file == id) else {
            return false;
        };
        let target_index = target_index.min(pane.items.len().saturating_sub(1));
        if source_index == target_index {
            return false;
        }
        let id = pane.items.remove(source_index);
        pane.items.insert(target_index, id);
        true
    }

    /// Remove a tab from one pane. The document is released when no pane
    /// shows it any more, and an emptied pane (other than the last) closes.
    pub fn close_item(&mut self, pane: PaneId, id: FileId) {
        if !self.panes.get_mut(&pane).is_some_and(|p| p.remove(id)) {
            return;
        }
        let orphaned = self.documents.get_mut(&id).is_some_and(|document| {
            document.views.remove(&pane);
            document.views.is_empty()
        });
        if orphaned
            && let Some(document) = self.documents.remove(&id)
            && document.is_text()
        {
            self.paths.remove(&document.meta.path);
        }
        self.remove_pane_if_empty(pane);
    }

    /// Close a document in every pane that shows it.
    pub fn close(&mut self, id: FileId) {
        for pane in self.pane_ids() {
            self.close_item(pane, id);
        }
    }

    /// Every tab showing a document, in pane order.
    pub fn tabs_of(&self, id: FileId) -> Vec<(PaneId, FileId)> {
        self.pane_ids()
            .into_iter()
            .filter(|pane| self.panes[pane].contains(id))
            .map(|pane| (pane, id))
            .collect()
    }

    /// The documents that closing these tabs would release, i.e. those with no
    /// remaining tab elsewhere.
    pub fn released_by(&self, targets: &[(PaneId, FileId)]) -> Vec<FileId> {
        let mut released = Vec::new();
        for &(_, id) in targets {
            if released.contains(&id) {
                continue;
            }
            let Some(document) = self.documents.get(&id) else {
                continue;
            };
            if document
                .views
                .keys()
                .all(|pane| targets.contains(&(*pane, id)))
            {
                released.push(id);
            }
        }
        released
    }

    // ---- The focused editor ------------------------------------------------

    /// Active tab of the active pane.
    pub fn active(&self) -> Option<FileId> {
        self.active_pane_ref().active
    }

    /// Tabs of the active pane.
    pub fn active_items(&self) -> &[FileId] {
        &self.active_pane_ref().items
    }

    pub fn preview(&self) -> Option<FileId> {
        self.active_pane_ref().preview
    }

    pub fn promote_preview(&mut self, pane: PaneId, id: FileId) -> bool {
        match self.panes.get_mut(&pane) {
            Some(pane) if pane.preview == Some(id) => {
                pane.preview = None;
                true
            }
            _ => false,
        }
    }

    pub fn promote_active_preview(&mut self) -> bool {
        let pane = self.active_pane;
        self.active()
            .is_some_and(|id| self.promote_preview(pane, id))
    }

    pub fn active_meta(&self) -> Option<&FileMeta> {
        self.active().and_then(|id| self.meta(id))
    }

    /// Path of the active file or diff; built-in pages have none.
    pub fn active_path(&self) -> Option<&Path> {
        if self.active_page().is_some() {
            return None;
        }
        self.active_meta().map(|meta| meta.path.as_path())
    }

    pub fn active_diff(&self) -> Option<&DiffDocument> {
        self.active()
            .and_then(|id| self.documents.get(&id))
            .and_then(|document| document.diff.as_ref())
    }

    pub fn active_page(&self) -> Option<AppPage> {
        self.active().and_then(|id| self.page(id))
    }

    pub fn active_is_settings(&self) -> bool {
        self.active_page() == Some(AppPage::Settings)
    }

    pub fn active_editor(&self) -> Option<Editor<'_>> {
        self.editor(self.active_pane)
    }

    pub fn active_editor_mut(&mut self) -> Option<EditorMut<'_>> {
        self.editor_mut(self.active_pane)
    }

    /// The text editor of a pane's active tab, if that tab is a file.
    pub fn editor(&self, pane: PaneId) -> Option<Editor<'_>> {
        let id = self.panes.get(&pane)?.active?;
        let document = self.documents.get(&id).filter(|d| d.is_text())?;
        let view = document.views.get(&pane)?;
        Some(Editor::new(&document.buffer, view.selection))
    }

    pub fn editor_mut(&mut self, pane: PaneId) -> Option<EditorMut<'_>> {
        let id = self.panes.get(&pane)?.active?;
        self.documents
            .get_mut(&id)
            .filter(|d| d.is_text())?
            .editor_mut(pane)
    }

    /// The diff shown by a pane's active tab.
    pub fn diff(&self, pane: PaneId) -> Option<&DiffDocument> {
        let id = self.panes.get(&pane)?.active?;
        self.documents.get(&id)?.diff.as_ref()
    }

    /// The built-in page shown by a pane's active tab.
    pub fn pane_page(&self, pane: PaneId) -> Option<AppPage> {
        let id = self.panes.get(&pane)?.active?;
        self.page(id)
    }

    pub fn view(&self, pane: PaneId, id: FileId) -> Option<&ViewState> {
        self.documents.get(&id)?.views.get(&pane)
    }

    /// Scroll offset of a pane's active tab.
    pub fn scroll(&self, pane: PaneId) -> (f32, f32) {
        self.panes
            .get(&pane)
            .and_then(|p| p.active)
            .and_then(|id| self.view(pane, id))
            .map_or((0.0, 0.0), |view| (view.scroll_x, view.scroll_y))
    }

    pub fn set_scroll(&mut self, pane: PaneId, x: f32, y: f32) {
        let Some(id) = self.panes.get(&pane).and_then(|p| p.active) else {
            return;
        };
        if let Some(view) = self
            .documents
            .get_mut(&id)
            .and_then(|document| document.views.get_mut(&pane))
        {
            view.scroll_x = x.max(0.0);
            view.scroll_y = y.max(0.0);
        }
    }

    pub fn active_scroll(&self) -> (f32, f32) {
        self.scroll(self.active_pane)
    }

    pub fn set_active_scroll(&mut self, x: f32, y: f32) {
        self.set_scroll(self.active_pane, x, y);
    }

    pub fn active_save_snapshot(&self) -> Option<(FileId, PathBuf, String)> {
        self.save_snapshot(self.active()?)
    }

    // ---- Documents ---------------------------------------------------------

    /// Every open document, in pane order then tab order, without repeats.
    fn ordered_documents(&self) -> Vec<FileId> {
        let mut ids = Vec::new();
        for pane in self.pane_ids() {
            for id in &self.panes[&pane].items {
                if !ids.contains(id) {
                    ids.push(*id);
                }
            }
        }
        ids
    }

    pub fn open_paths(&self) -> Vec<PathBuf> {
        self.ordered_documents()
            .into_iter()
            .filter_map(|id| self.documents.get(&id))
            .filter(|document| document.is_text())
            .map(|document| document.meta.path.clone())
            .collect()
    }

    pub fn meta(&self, id: FileId) -> Option<&FileMeta> {
        self.documents.get(&id).map(|document| &document.meta)
    }

    pub fn is_diff(&self, id: FileId) -> bool {
        self.documents
            .get(&id)
            .is_some_and(|document| document.diff.is_some())
    }

    pub fn file_id_for_path(&self, path: &Path) -> Option<FileId> {
        self.paths.get(path).copied()
    }

    pub fn is_dirty(&self, id: FileId) -> bool {
        self.documents.get(&id).is_some_and(|document| {
            document.is_text() && document.buffer.text() != document.saved_text
        })
    }

    pub fn dirty_paths(&self) -> Vec<PathBuf> {
        self.dirty_file_ids()
            .into_iter()
            .filter_map(|id| self.documents.get(&id))
            .map(|document| document.meta.path.clone())
            .collect()
    }

    pub fn dirty_file_ids(&self) -> Vec<FileId> {
        self.ordered_documents()
            .into_iter()
            .filter(|id| self.is_dirty(*id))
            .collect()
    }

    pub fn has_dirty_paths_under(&self, root: &Path) -> bool {
        self.documents
            .iter()
            .any(|(id, document)| document.meta.path.starts_with(root) && self.is_dirty(*id))
    }

    pub fn has_disk_conflict(&self, id: FileId) -> bool {
        self.documents
            .get(&id)
            .is_some_and(|document| document.is_text() && document.disk_conflict)
    }

    pub fn is_missing_on_disk(&self, id: FileId) -> bool {
        self.documents
            .get(&id)
            .is_some_and(|document| document.is_text() && document.missing_on_disk)
    }

    pub fn save_snapshot(&self, id: FileId) -> Option<(FileId, PathBuf, String)> {
        let document = self.documents.get(&id)?;
        if !document.is_text() {
            return None;
        }
        Some((
            id,
            document.meta.path.clone(),
            document.buffer.text().to_owned(),
        ))
    }

    pub fn mark_saved(&mut self, id: FileId) -> bool {
        let Some(document) = self.documents.get_mut(&id) else {
            return false;
        };
        if !document.is_text() {
            return false;
        }
        document.saved_text = document.buffer.text().to_owned();
        document.disk_state = DiskState::capture(&document.meta.path, &document.saved_text);
        document.disk_conflict = false;
        document.missing_on_disk = false;
        document.buffer.break_undo_group();
        true
    }

    // ---- Opening -----------------------------------------------------------

    pub fn open_path(&mut self, path: PathBuf, contents: String) -> FileId {
        self.open_path_with_mode(path, contents, OpenMode::Permanent)
    }

    pub fn preview_path(&mut self, path: PathBuf, contents: String) -> FileId {
        self.open_path_with_mode(path, contents, OpenMode::Preview)
    }

    /// Open a file in the active pane. A file already open elsewhere gets a
    /// second tab here sharing its buffer; `contents` is then ignored.
    fn open_path_with_mode(&mut self, path: PathBuf, contents: String, mode: OpenMode) -> FileId {
        let pane = self.active_pane;
        if let Some(id) = self.file_id_for_path(&path) {
            if !self.panes[&pane].contains(id) {
                let index = self.preview_replacement_index(pane, mode);
                self.insert_item(pane, id, index.unwrap_or(usize::MAX), None);
                if mode == OpenMode::Preview {
                    self.replace_preview_slot(pane, id);
                }
            } else if mode == OpenMode::Permanent {
                self.promote_preview(pane, id);
            }
            self.active_pane_mut().active = Some(id);
            return id;
        }

        let replacement_index = self.preview_replacement_index(pane, mode);
        let id = self.allocate_file_id();
        let disk_state = DiskState::capture(&path, &contents);
        self.documents.insert(
            id,
            OpenDocument::new(FileMeta::from_path(path.clone()), contents, disk_state),
        );
        self.paths.insert(path, id);
        self.insert_item(pane, id, replacement_index.unwrap_or(usize::MAX), None);
        if mode == OpenMode::Preview {
            self.replace_preview_slot(pane, id);
        }
        self.active_pane_mut().active = Some(id);
        id
    }

    /// Where a new preview tab goes: in place of the pane's clean preview.
    fn preview_replacement_index(&mut self, pane: PaneId, mode: OpenMode) -> Option<usize> {
        if mode != OpenMode::Preview {
            return None;
        }
        let preview = self.panes[&pane].preview?;
        if self.is_dirty(preview) {
            // Defensive promotion: UI edit paths promote eagerly, but no
            // missed path may allow a dirty preview to be replaced.
            self.panes.get_mut(&pane).unwrap().preview = None;
            return None;
        }
        self.panes[&pane]
            .items
            .iter()
            .position(|id| *id == preview)
            // The new tab is inserted before the old preview is closed.
            .map(|index| index + 1)
    }

    fn allocate_file_id(&mut self) -> FileId {
        let id = FileId::new(self.next_file_id);
        self.next_file_id += 1;
        id
    }

    /// Activate an existing single-instance tab wherever it is.
    fn reveal(&mut self, id: FileId) -> bool {
        match self.pane_of(id) {
            Some(pane) => {
                self.activate(pane, id);
                true
            }
            None => false,
        }
    }

    /// Open or refresh a read-only diff tab. A tab is uniquely identified by
    /// repository, relative path and comparison target.
    pub fn open_diff(&mut self, diff: DiffDocument) -> FileId {
        if let Some((&id, document)) = self.documents.iter_mut().find(|(_, document)| {
            document.diff.as_ref().is_some_and(|current| {
                current.matches(&diff.repository_root, &diff.path, diff.target)
            })
        }) {
            document.meta.name = diff.title();
            document.meta.path = diff.absolute_path();
            document.diff = Some(diff);
            self.reveal(id);
            return id;
        }

        let id = self.allocate_file_id();
        let absolute_path = diff.absolute_path();
        let mut meta = FileMeta::from_path(absolute_path.clone());
        meta.name = diff.title();
        let mut document =
            OpenDocument::new(meta, String::new(), DiskState::capture(&absolute_path, ""));
        document.diff = Some(diff);
        self.documents.insert(id, document);
        let pane = self.active_pane;
        self.insert_item(pane, id, usize::MAX, None);
        self.active_pane_mut().active = Some(id);
        id
    }

    /// Open the settings page, or focus it if it is already open.
    pub fn open_settings(&mut self) -> FileId {
        self.open_page(AppPage::Settings)
    }

    /// Open the keymap page, or focus it if it is already open.
    pub fn open_keymap(&mut self) -> FileId {
        self.open_page(AppPage::Keymap)
    }

    /// Open a built-in page; each page has at most one tab in the workspace.
    pub fn open_page(&mut self, page: AppPage) -> FileId {
        if let Some(id) = self.page_id(page) {
            self.reveal(id);
            return id;
        }

        let id = self.allocate_file_id();
        let path = PathBuf::from(page.title());
        let mut meta = FileMeta::from_path(path.clone());
        meta.name = page.title().to_owned();
        let mut document = OpenDocument::new(meta, String::new(), DiskState::capture(&path, ""));
        document.page = Some(page);
        self.documents.insert(id, document);
        let pane = self.active_pane;
        self.insert_item(pane, id, usize::MAX, None);
        self.active_pane_mut().active = Some(id);
        id
    }

    pub fn page_id(&self, page: AppPage) -> Option<FileId> {
        self.documents
            .iter()
            .find_map(|(id, document)| (document.page == Some(page)).then_some(*id))
    }

    /// The built-in page a tab shows, if it is not a file or diff.
    pub fn page(&self, id: FileId) -> Option<AppPage> {
        self.documents.get(&id).and_then(|document| document.page)
    }

    pub fn settings_id(&self) -> Option<FileId> {
        self.page_id(AppPage::Settings)
    }

    pub fn is_settings(&self, id: FileId) -> bool {
        self.page(id) == Some(AppPage::Settings)
    }

    /// Whether a tab is backed by a file (rather than a diff or settings).
    pub fn is_file(&self, id: FileId) -> bool {
        self.documents.get(&id).is_some_and(OpenDocument::is_text)
    }

    pub fn diff_id_for(
        &self,
        repository_root: &Path,
        path: &Path,
        target: DiffTarget,
    ) -> Option<FileId> {
        self.documents.iter().find_map(|(&id, document)| {
            document
                .diff
                .as_ref()
                .is_some_and(|diff| diff.matches(repository_root, path, target))
                .then_some(id)
        })
    }

    // ---- Disk --------------------------------------------------------------

    /// Reconcile every open document with disk. Clean documents reload
    /// automatically; dirty documents retain their buffer and are marked as a
    /// conflict for explicit user resolution.
    pub fn reconcile_disk(&mut self) -> Vec<ReconcileResult> {
        self.ordered_documents()
            .into_iter()
            .map(|id| self.reconcile_document(id))
            .collect()
    }

    /// Reconcile open documents touched by a watcher batch. A directory-level
    /// event also covers open descendants; a rescan checks everything.
    pub fn reconcile_changed_paths(
        &mut self,
        changed_paths: &[PathBuf],
        full_rescan: bool,
    ) -> Vec<ReconcileResult> {
        let ids = self
            .ordered_documents()
            .into_iter()
            .filter(|id| {
                full_rescan
                    || self.documents.get(id).is_some_and(|document| {
                        document.is_text()
                            && changed_paths.iter().any(|changed| {
                                document.meta.path == *changed
                                    || document.meta.path.starts_with(changed)
                            })
                    })
            })
            .collect::<Vec<_>>();
        ids.into_iter()
            .map(|id| self.reconcile_document(id))
            .collect()
    }

    pub fn reconcile_document(&mut self, id: FileId) -> ReconcileResult {
        let Some(document) = self.documents.get(&id) else {
            return ReconcileResult::Failed(PathBuf::new(), "Document is not open.".into());
        };
        let path = document.meta.path.clone();
        if !document.is_text() {
            return ReconcileResult::Unchanged(path);
        }
        let dirty = document.buffer.text() != document.saved_text;
        let previous = document.disk_state.clone();
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Some(document) = self.documents.get_mut(&id) {
                    document.missing_on_disk = true;
                    document.disk_conflict = dirty;
                }
                return ReconcileResult::Missing(path);
            }
            Err(error) => return ReconcileResult::Failed(path, error.to_string()),
        };
        let current = DiskState::capture(&path, &contents);
        if previous.content_hash == current.content_hash && previous.size == current.size {
            if let Some(document) = self.documents.get_mut(&id) {
                document.disk_state = current;
                document.missing_on_disk = false;
                document.disk_conflict = false;
            }
            return ReconcileResult::Unchanged(path);
        }
        let document = self.documents.get_mut(&id).expect("document remains open");
        document.missing_on_disk = false;
        if dirty {
            document.disk_conflict = true;
            ReconcileResult::Conflict(path)
        } else {
            replace_buffer_from_disk(document, contents.clone());
            document.saved_text = contents;
            document.disk_state = current;
            document.disk_conflict = false;
            ReconcileResult::Reloaded(path)
        }
    }

    pub fn accept_disk_version(&mut self, id: FileId) -> Result<(), String> {
        let document = self
            .documents
            .get_mut(&id)
            .ok_or_else(|| "Document is not open.".to_owned())?;
        if !document.is_text() {
            return Err("Diff documents are read-only.".to_owned());
        }
        let contents = std::fs::read_to_string(&document.meta.path).map_err(|error| {
            format!("Could not reload {}: {error}", document.meta.path.display())
        })?;
        replace_buffer_from_disk(document, contents.clone());
        document.saved_text = contents;
        document.disk_state = DiskState::capture(&document.meta.path, &document.saved_text);
        document.disk_conflict = false;
        document.missing_on_disk = false;
        Ok(())
    }

    pub fn keep_editor_version(&mut self, id: FileId) -> bool {
        let Some(document) = self.documents.get_mut(&id) else {
            return false;
        };
        if !document.is_text() {
            return false;
        }
        if let Ok(contents) = std::fs::read_to_string(&document.meta.path) {
            document.disk_state = DiskState::capture(&document.meta.path, &contents);
        }
        document.disk_conflict = false;
        true
    }

    pub fn move_open_path(&mut self, old_path: &Path, new_path: PathBuf) -> bool {
        let Some(id) = self.paths.remove(old_path) else {
            return false;
        };
        let Some(document) = self.documents.get_mut(&id) else {
            return false;
        };
        document.meta = FileMeta::from_path(new_path.clone());
        document.disk_state = DiskState::capture(&new_path, document.buffer.text());
        document.missing_on_disk = false;
        self.paths.insert(new_path, id);
        true
    }

    /// Retarget every open file below a renamed directory while preserving its
    /// buffer, dirty state, active tab, and scroll position.
    pub fn move_open_paths_under(&mut self, old_root: &Path, new_root: &Path) -> usize {
        let moves = self
            .paths
            .keys()
            .filter_map(|path| {
                path.strip_prefix(old_root)
                    .ok()
                    .map(|relative| (path.clone(), new_root.join(relative)))
            })
            .collect::<Vec<_>>();
        let mut moved = 0;
        for (old_path, new_path) in moves {
            moved += usize::from(self.move_open_path(&old_path, new_path));
        }
        moved
    }

    /// Close every editable document below a directory removed on disk.
    pub fn close_paths_under(&mut self, root: &Path) -> usize {
        let ids = self
            .paths
            .iter()
            .filter_map(|(path, id)| path.starts_with(root).then_some(*id))
            .collect::<Vec<_>>();
        let count = ids.len();
        for id in ids {
            self.close(id);
        }
        count
    }
}

/// External reloads reset edit history but retain each view's caret and
/// selection as far as the new document length permits. Scroll positions
/// live in the views and are therefore preserved as well.
fn replace_buffer_from_disk(document: &mut OpenDocument, contents: String) {
    document.buffer.reload(contents);
    document.clamp_views();
}

fn parent_components(path: &Path) -> Vec<String> {
    path.parent()
        .into_iter()
        .flat_map(Path::components)
        .filter_map(|component| {
            let value = component.as_os_str().to_string_lossy();
            (!value.is_empty() && value != "\\" && value != "/").then(|| value.into_owned())
        })
        .collect()
}

fn parent_suffix(parts: &[String], depth: usize) -> String {
    let start = parts.len().saturating_sub(depth);
    parts[start..].join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::diff::UnifiedDiff;

    #[test]
    fn settings_tab_is_unique_and_never_treated_as_a_file() {
        let mut workspace = Workspace::new();
        let file = workspace.open_path(PathBuf::from("src/main.rs"), "fn main() {}".into());
        let settings = workspace.open_settings();
        workspace.set_active(file);
        assert_eq!(workspace.open_settings(), settings);

        assert_eq!(workspace.active(), Some(settings));
        assert!(workspace.active_is_settings());
        assert_eq!(workspace.active_path(), None);
        assert!(workspace.active_editor().is_none());
        assert!(workspace.save_snapshot(settings).is_none());
        assert!(!workspace.is_dirty(settings));
        assert!(!workspace.is_file(settings));
        assert_eq!(workspace.open_paths(), vec![PathBuf::from("src/main.rs")]);

        workspace.close(settings);
        assert_eq!(workspace.settings_id(), None);
        assert_eq!(workspace.active(), Some(file));
    }

    #[test]
    fn keymap_page_is_its_own_unique_tab() {
        let mut workspace = Workspace::new();
        let settings = workspace.open_settings();
        let keymap = workspace.open_keymap();
        assert_ne!(keymap, settings);
        assert_eq!(workspace.open_keymap(), keymap);
        assert_eq!(workspace.active_page(), Some(AppPage::Keymap));
        assert!(!workspace.active_is_settings());
        assert!(!workspace.is_file(keymap));
        assert_eq!(workspace.active_path(), None);
        assert!(workspace.open_paths().is_empty());
    }

    #[test]
    fn opens_disk_files_once_and_reactivates_existing_document() {
        let mut workspace = Workspace::new();
        let first = workspace.open_path(PathBuf::from("src/main.rs"), "fn main() {}".into());
        let second = workspace.open_path(PathBuf::from("README.md"), "hello".into());
        let reopened = workspace.open_path(PathBuf::from("src/main.rs"), "ignored".into());

        assert_eq!(reopened, first);
        assert_eq!(workspace.active(), Some(first));
        assert_eq!(workspace.active_items(), &[first, second]);
        assert_eq!(
            workspace.open_paths(),
            vec![PathBuf::from("src/main.rs"), PathBuf::from("README.md")]
        );
        assert_eq!(workspace.active_editor().unwrap().text(), "fn main() {}");
    }

    #[test]
    fn a_new_preview_replaces_the_clean_preview_at_the_same_index() {
        let mut workspace = Workspace::new();
        let permanent = workspace.open_path(PathBuf::from("permanent.rs"), String::new());
        let first = workspace.preview_path(PathBuf::from("first.rs"), "first".into());
        let trailing = workspace.open_path(PathBuf::from("trailing.rs"), String::new());
        workspace.set_active(first);

        let second = workspace.preview_path(PathBuf::from("second.rs"), "second".into());

        assert_eq!(workspace.active_items(), &[permanent, second, trailing]);
        assert_eq!(workspace.preview(), Some(second));
        assert_eq!(workspace.active(), Some(second));
        assert_eq!(workspace.file_id_for_path(Path::new("first.rs")), None);
    }

    #[test]
    fn editing_a_preview_defensively_preserves_it_before_the_next_preview() {
        let mut workspace = Workspace::new();
        let first = workspace.preview_path(PathBuf::from("first.rs"), String::new());
        workspace.active_editor_mut().unwrap().insert("changed");

        let second = workspace.preview_path(PathBuf::from("second.rs"), String::new());

        assert_eq!(workspace.active_items(), &[first, second]);
        assert_ne!(workspace.preview(), Some(first));
        assert_eq!(workspace.preview(), Some(second));
        assert!(workspace.is_dirty(first));
    }

    #[test]
    fn permanently_reopening_a_preview_promotes_without_duplication() {
        let mut workspace = Workspace::new();
        let path = PathBuf::from("main.rs");
        let preview = workspace.preview_path(path.clone(), "original".into());

        let reopened = workspace.open_path(path, "ignored".into());

        assert_eq!(reopened, preview);
        assert_eq!(workspace.active_items(), &[preview]);
        assert_eq!(workspace.preview(), None);
        assert_eq!(workspace.active_editor().unwrap().text(), "original");
    }

    #[test]
    fn activating_an_existing_permanent_tab_keeps_the_preview_slot() {
        let mut workspace = Workspace::new();
        let permanent = workspace.open_path(PathBuf::from("permanent.rs"), String::new());
        let preview = workspace.preview_path(PathBuf::from("preview.rs"), String::new());

        let reopened = workspace.preview_path(PathBuf::from("permanent.rs"), "ignored".into());

        assert_eq!(reopened, permanent);
        assert_eq!(workspace.active(), Some(permanent));
        assert_eq!(workspace.preview(), Some(preview));
        assert_eq!(workspace.active_items(), &[permanent, preview]);
    }

    #[test]
    fn closing_a_preview_clears_the_preview_slot() {
        let mut workspace = Workspace::new();
        let preview = workspace.preview_path(PathBuf::from("preview.rs"), String::new());

        workspace.close(preview);

        assert_eq!(workspace.preview(), None);
        assert!(workspace.active_items().is_empty());
    }

    #[test]
    fn closing_document_removes_its_path_and_activates_a_neighbor() {
        let mut workspace = Workspace::new();
        let first = workspace.open_path(PathBuf::from("first.txt"), "first".into());
        let second = workspace.open_path(PathBuf::from("second.txt"), "second".into());

        workspace.close(second);

        assert_eq!(workspace.active(), Some(first));
        assert_eq!(workspace.file_id_for_path(Path::new("second.txt")), None);
    }

    #[test]
    fn diff_tabs_are_reused_and_are_not_persisted_as_editable_files() {
        let mut workspace = Workspace::new();
        let file_path = PathBuf::from("repo/src/main.rs");
        let file = workspace.open_path(file_path.clone(), "fn main() {}".into());
        let make_diff = || {
            DiffDocument::from_unified(
                PathBuf::from("repo"),
                PathBuf::from("src/main.rs"),
                DiffTarget::IndexToWorktree,
                UnifiedDiff::default(),
            )
        };

        let first = workspace.open_diff(make_diff());
        let reopened = workspace.open_diff(make_diff());

        assert_eq!(first, reopened);
        assert_eq!(workspace.active(), Some(first));
        assert!(workspace.is_diff(first));
        assert!(workspace.active_editor().is_none());
        assert_eq!(workspace.active_items(), &[file, first]);
        assert_eq!(workspace.open_paths(), vec![file_path]);
    }

    #[test]
    fn keeps_an_independent_scroll_position_for_each_document() {
        let mut workspace = Workspace::new();
        let first = workspace.open_path(PathBuf::from("first.txt"), "first".into());
        workspace.set_active_scroll(24.0, 120.0);
        let second = workspace.open_path(PathBuf::from("second.txt"), "second".into());
        workspace.set_active_scroll(8.0, 40.0);

        workspace.set_active(first);
        assert_eq!(workspace.active_scroll(), (24.0, 120.0));
        workspace.set_active(second);
        assert_eq!(workspace.active_scroll(), (8.0, 40.0));
    }

    #[test]
    fn dirty_state_tracks_content_against_the_opened_file() {
        let mut workspace = Workspace::new();
        let file = workspace.open_path(PathBuf::from("notes.txt"), "hello".into());

        assert!(!workspace.is_dirty(file));
        workspace.active_editor_mut().unwrap().move_end();
        assert!(!workspace.is_dirty(file));

        workspace.active_editor_mut().unwrap().insert("!");
        assert!(workspace.is_dirty(file));

        let (snapshot_id, path, contents) = workspace.active_save_snapshot().unwrap();
        assert_eq!(snapshot_id, file);
        assert_eq!(path, PathBuf::from("notes.txt"));
        assert_eq!(contents, "hello!");
        assert!(workspace.mark_saved(file));
        assert!(!workspace.is_dirty(file));

        workspace.active_editor_mut().unwrap().backspace();
        assert!(workspace.is_dirty(file));
    }

    #[test]
    fn dirty_file_ids_follow_tab_order_and_exclude_clean_files() {
        let mut workspace = Workspace::new();
        let first = workspace.open_path(PathBuf::from("first.rs"), String::new());
        workspace
            .active_editor_mut()
            .unwrap()
            .insert("first change");
        let clean = workspace.open_path(PathBuf::from("clean.rs"), String::new());
        let second = workspace.open_path(PathBuf::from("second.rs"), String::new());
        workspace
            .active_editor_mut()
            .unwrap()
            .insert("second change");

        assert_eq!(workspace.dirty_file_ids(), vec![first, second]);
        assert!(!workspace.dirty_file_ids().contains(&clean));
    }

    #[test]
    fn moving_tabs_uses_the_requested_final_index_and_keeps_the_active_document() {
        let mut workspace = Workspace::new();
        let first = workspace.open_path(PathBuf::from("first.rs"), String::new());
        let second = workspace.open_path(PathBuf::from("second.rs"), String::new());
        let third = workspace.open_path(PathBuf::from("third.rs"), String::new());

        assert!(workspace.move_tab(workspace.active_pane(), first, 2));
        assert_eq!(workspace.active_items(), &[second, third, first]);
        assert_eq!(workspace.active(), Some(third));

        assert!(workspace.move_tab(workspace.active_pane(), first, 0));
        assert_eq!(workspace.active_items(), &[first, second, third]);
        assert!(!workspace.move_tab(workspace.active_pane(), first, 0));
    }

    #[test]
    fn duplicate_tab_names_use_the_shortest_distinguishing_parent_suffix() {
        let mut workspace = Workspace::new();
        let first = workspace.open_path(PathBuf::from("alpha/src/main.rs"), String::new());
        let second = workspace.open_path(PathBuf::from("beta/src/main.rs"), String::new());
        let unique = workspace.open_path(PathBuf::from("beta/src/lib.rs"), String::new());

        let labels = workspace.tab_labels(workspace.active_pane());
        assert_eq!(labels.get(&first).unwrap(), "main.rs — alpha/src");
        assert_eq!(labels.get(&second).unwrap(), "main.rs — beta/src");
        assert_eq!(labels.get(&unique).unwrap(), "lib.rs");
    }

    #[test]
    fn positional_activation_selects_existing_indices_and_the_last_tab() {
        let mut workspace = Workspace::new();
        let first = workspace.open_path(PathBuf::from("first.rs"), String::new());
        let second = workspace.open_path(PathBuf::from("second.rs"), String::new());
        let third = workspace.open_path(PathBuf::from("third.rs"), String::new());

        assert!(workspace.activate_at(0));
        assert_eq!(workspace.active(), Some(first));
        assert!(!workspace.activate_at(99));
        assert_eq!(workspace.active(), Some(first));
        assert!(workspace.activate_last());
        assert_eq!(workspace.active(), Some(third));
        assert_ne!(workspace.active(), Some(second));
    }

    #[test]
    fn directory_rename_retargets_open_files_without_reopening_them() {
        let mut workspace = Workspace::new();
        let old_root = PathBuf::from("project/src");
        let new_root = PathBuf::from("project/source");
        let old_file = old_root.join("nested/main.rs");
        let new_file = new_root.join("nested/main.rs");
        let id = workspace.open_path(old_file.clone(), "fn main() {}".into());

        assert_eq!(workspace.move_open_paths_under(&old_root, &new_root), 1);
        assert_eq!(workspace.file_id_for_path(&old_file), None);
        assert_eq!(workspace.file_id_for_path(&new_file), Some(id));
        assert_eq!(workspace.active_path(), Some(new_file.as_path()));
    }

    #[test]
    fn directory_delete_closes_only_open_files_below_that_directory() {
        let mut workspace = Workspace::new();
        let removed = workspace.open_path(PathBuf::from("project/src/main.rs"), String::new());
        let kept_path = PathBuf::from("project/tests/main.rs");
        let kept = workspace.open_path(kept_path.clone(), String::new());

        assert_eq!(workspace.close_paths_under(Path::new("project/src")), 1);
        assert_eq!(
            workspace.file_id_for_path(Path::new("project/src/main.rs")),
            None
        );
        assert_eq!(workspace.file_id_for_path(&kept_path), Some(kept));
        assert_eq!(workspace.active(), Some(kept));
        assert_ne!(workspace.active(), Some(removed));
    }

    #[test]
    fn dirty_document_is_never_replaced_during_disk_reconciliation() {
        let root = std::env::temp_dir().join(format!(
            "loom-reconcile-{}-{}",
            std::process::id(),
            crate::model::document::content_hash(module_path!())
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("note.txt");
        std::fs::write(&path, "disk one").unwrap();
        let mut workspace = Workspace::new();
        let id = workspace.open_path(path.clone(), "disk one".into());
        workspace.active_editor_mut().unwrap().move_end();
        workspace.active_editor_mut().unwrap().insert(" + editor");
        std::fs::write(&path, "disk two with size").unwrap();

        assert_eq!(
            workspace.reconcile_document(id),
            ReconcileResult::Conflict(path.clone())
        );
        assert_eq!(
            workspace.active_editor().unwrap().text(),
            "disk one + editor"
        );
        assert!(workspace.has_disk_conflict(id));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn clean_document_reload_preserves_cursor_and_scroll() {
        let root = std::env::temp_dir().join(format!(
            "loom-reload-clean-{}-{}",
            std::process::id(),
            crate::model::document::content_hash(module_path!())
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("note.txt");
        std::fs::write(&path, "before").unwrap();
        let mut workspace = Workspace::new();
        let id = workspace.open_path(path.clone(), "before".into());
        workspace.active_editor_mut().unwrap().select_range(2..4);
        workspace.set_active_scroll(12.0, 34.0);
        std::fs::write(&path, "after and longer").unwrap();

        assert_eq!(
            workspace.reconcile_document(id),
            ReconcileResult::Reloaded(path.clone())
        );
        assert_eq!(
            workspace.active_editor().unwrap().text(),
            "after and longer"
        );
        assert_eq!(workspace.active_editor().unwrap().cursor(), 4);
        assert_eq!(workspace.active_editor().unwrap().selection(), Some(2..4));
        assert_eq!(workspace.active_scroll(), (12.0, 34.0));
        assert!(!workspace.is_dirty(id));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn deleted_document_remains_open_and_is_marked_missing() {
        let root = std::env::temp_dir().join(format!(
            "loom-reload-missing-{}-{}",
            std::process::id(),
            crate::model::document::content_hash(module_path!())
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("note.txt");
        std::fs::write(&path, "contents").unwrap();
        let mut workspace = Workspace::new();
        let id = workspace.open_path(path.clone(), "contents".into());
        std::fs::remove_file(&path).unwrap();

        assert_eq!(
            workspace.reconcile_document(id),
            ReconcileResult::Missing(path.clone())
        );
        assert_eq!(workspace.active_editor().unwrap().text(), "contents");
        assert!(workspace.is_missing_on_disk(id));
        assert!(!workspace.has_disk_conflict(id));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn recreating_a_missing_file_with_saved_contents_clears_missing_state() {
        let root = std::env::temp_dir().join(format!(
            "loom-reload-recreated-{}-{}",
            std::process::id(),
            crate::model::document::content_hash(module_path!())
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("note.txt");
        std::fs::write(&path, "contents").unwrap();
        let mut workspace = Workspace::new();
        let id = workspace.open_path(path.clone(), "contents".into());
        std::fs::remove_file(&path).unwrap();
        workspace.reconcile_document(id);
        assert!(workspace.is_missing_on_disk(id));

        std::fs::write(&path, "contents").unwrap();
        assert_eq!(
            workspace.reconcile_document(id),
            ReconcileResult::Unchanged(path)
        );
        assert!(!workspace.is_missing_on_disk(id));
        assert!(!workspace.has_disk_conflict(id));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn save_echo_does_not_create_a_disk_conflict() {
        let root = std::env::temp_dir().join(format!(
            "loom-save-echo-{}-{}",
            std::process::id(),
            crate::model::document::content_hash(module_path!())
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("note.txt");
        std::fs::write(&path, "before").unwrap();
        let mut workspace = Workspace::new();
        let id = workspace.open_path(path.clone(), "before".into());
        workspace.active_editor_mut().unwrap().move_end();
        workspace.active_editor_mut().unwrap().insert(" saved");
        std::fs::write(&path, "before saved").unwrap();
        assert!(workspace.mark_saved(id));

        assert_eq!(
            workspace.reconcile_document(id),
            ReconcileResult::Unchanged(path)
        );
        assert!(!workspace.has_disk_conflict(id));
        assert!(!workspace.is_missing_on_disk(id));

        let _ = std::fs::remove_dir_all(root);
    }

    fn two_panes_with(path: &str, text: &str) -> (Workspace, PaneId, PaneId, FileId) {
        let mut workspace = Workspace::new();
        let left = workspace.active_pane();
        let id = workspace.open_path(PathBuf::from(path), text.into());
        let right = workspace.split(left, Direction::Right).unwrap();
        (workspace, left, right, id)
    }

    #[test]
    fn split_copies_the_active_tab_and_its_view_into_a_focused_new_pane() {
        let mut workspace = Workspace::new();
        let left = workspace.active_pane();
        let id = workspace.open_path(PathBuf::from("a.rs"), "hello world".into());
        workspace.active_editor_mut().unwrap().select_range(0..5);
        workspace.set_active_scroll(3.0, 40.0);

        let right = workspace.split(left, Direction::Right).unwrap();

        assert_eq!(workspace.active_pane(), right);
        assert_eq!(workspace.pane_ids(), vec![left, right]);
        assert_eq!(workspace.pane(right).unwrap().items(), &[id]);
        assert_eq!(workspace.active(), Some(id));
        assert_eq!(workspace.active_editor().unwrap().selection(), Some(0..5));
        assert_eq!(workspace.scroll(right), (3.0, 40.0));
        assert_eq!(workspace.pane(left).unwrap().items(), &[id]);
    }

    #[test]
    fn views_of_one_document_share_text_but_not_selection_or_scroll() {
        let (mut workspace, left, right, id) = two_panes_with("a.rs", "one two");
        workspace.editor_mut(left).unwrap().set_cursor(0);
        workspace.editor_mut(right).unwrap().set_cursor(7);
        workspace.set_scroll(right, 0.0, 99.0);

        workspace.editor_mut(left).unwrap().insert(">> ");

        assert_eq!(workspace.editor(right).unwrap().text(), ">> one two");
        assert_eq!(workspace.editor(right).unwrap().cursor(), 10);
        assert_eq!(workspace.editor(left).unwrap().cursor(), 3);
        assert_eq!(workspace.scroll(left), (0.0, 0.0));
        assert_eq!(workspace.scroll(right), (0.0, 99.0));
        assert!(workspace.is_dirty(id));
        assert_eq!(workspace.dirty_file_ids(), vec![id]);
    }

    #[test]
    fn closing_one_tab_keeps_the_document_until_its_last_tab_closes() {
        let (mut workspace, left, right, id) = two_panes_with("a.rs", "text");
        workspace.editor_mut(right).unwrap().insert("x");

        assert!(workspace.released_by(&[(right, id)]).is_empty());
        assert_eq!(workspace.released_by(&[(left, id), (right, id)]), vec![id]);
        assert_eq!(workspace.tabs_of(id), vec![(left, id), (right, id)]);

        workspace.close_item(right, id);
        // The emptied pane closes and focus returns to the remaining one.
        assert_eq!(workspace.pane_ids(), vec![left]);
        assert_eq!(workspace.active_pane(), left);
        assert!(workspace.is_dirty(id));
        assert_eq!(workspace.editor(left).unwrap().text(), "xtext");

        workspace.close_item(left, id);
        assert!(workspace.meta(id).is_none());
        assert_eq!(workspace.file_id_for_path(Path::new("a.rs")), None);
        assert_eq!(workspace.pane_count(), 1);
    }

    #[test]
    fn opening_a_file_shown_elsewhere_adds_a_tab_sharing_its_buffer() {
        let mut workspace = Workspace::new();
        let left = workspace.active_pane();
        let id = workspace.open_path(PathBuf::from("a.rs"), "text".into());
        let right = workspace.split(left, Direction::Down).unwrap();
        let other = workspace.open_path(PathBuf::from("b.rs"), String::new());
        workspace.close_item(right, id);
        assert_eq!(workspace.pane(right).unwrap().items(), &[other]);

        let reopened = workspace.open_path(PathBuf::from("a.rs"), "ignored".into());

        assert_eq!(reopened, id);
        assert_eq!(workspace.pane(right).unwrap().items(), &[other, id]);
        assert_eq!(workspace.editor(right).unwrap().text(), "text");
    }

    #[test]
    fn previews_are_tracked_per_pane() {
        let mut workspace = Workspace::new();
        let left = workspace.active_pane();
        let first = workspace.preview_path(PathBuf::from("first.rs"), String::new());
        let right = workspace.split(left, Direction::Right).unwrap();
        assert_eq!(workspace.pane(right).unwrap().preview(), None);

        let second = workspace.preview_path(PathBuf::from("second.rs"), String::new());
        assert_eq!(workspace.pane(right).unwrap().items(), &[first, second]);
        assert_eq!(workspace.pane(right).unwrap().preview(), Some(second));
        assert_eq!(workspace.pane(left).unwrap().preview(), Some(first));

        let third = workspace.preview_path(PathBuf::from("third.rs"), String::new());
        assert_eq!(workspace.pane(right).unwrap().items(), &[first, third]);
        assert!(workspace.meta(second).is_none());
    }

    #[test]
    fn single_instance_tabs_are_revealed_instead_of_duplicated() {
        let mut workspace = Workspace::new();
        let left = workspace.active_pane();
        let settings = workspace.open_settings();
        let right = workspace.split(left, Direction::Right).unwrap();
        assert!(workspace.pane(right).unwrap().is_empty());

        assert_eq!(workspace.open_settings(), settings);
        assert_eq!(workspace.active_pane(), left);
        assert!(!workspace.copy_item(left, settings, right, 0));
    }

    #[test]
    fn moving_a_tab_between_panes_keeps_its_view_and_closes_an_emptied_source() {
        let mut workspace = Workspace::new();
        let left = workspace.active_pane();
        let keep = workspace.open_path(PathBuf::from("keep.rs"), String::new());
        let right = workspace.split(left, Direction::Right).unwrap();
        let moved = workspace.open_path(PathBuf::from("moved.rs"), "abc".into());
        workspace.close_item(right, keep);
        workspace.editor_mut(right).unwrap().set_cursor(2);

        assert!(workspace.move_item(right, moved, left, 0));

        assert_eq!(workspace.pane_ids(), vec![left]);
        assert_eq!(workspace.pane(left).unwrap().items(), &[moved, keep]);
        assert_eq!(workspace.active(), Some(moved));
        assert_eq!(workspace.active_editor().unwrap().cursor(), 2);
    }

    #[test]
    fn closing_a_pane_focuses_the_most_recent_remaining_pane() {
        let mut workspace = Workspace::new();
        let first = workspace.active_pane();
        workspace.open_path(PathBuf::from("a.rs"), String::new());
        let second = workspace.split(first, Direction::Right).unwrap();
        let third = workspace.split(second, Direction::Down).unwrap();
        workspace.activate_pane(first);
        workspace.activate_pane(third);

        workspace.close_pane(third);

        assert_eq!(workspace.active_pane(), first);
        assert_eq!(workspace.pane_ids(), vec![first, second]);
        // The last pane survives and only loses its tabs.
        workspace.close_pane(second);
        workspace.close_pane(first);
        assert_eq!(workspace.pane_ids(), vec![first]);
        assert!(workspace.active_items().is_empty());
    }

    #[test]
    fn reload_from_disk_clamps_every_view() {
        let root = std::env::temp_dir().join(format!(
            "loom-reload-views-{}-{}",
            std::process::id(),
            crate::model::document::content_hash(module_path!())
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("note.txt");
        std::fs::write(&path, "a long original line").unwrap();
        let mut workspace = Workspace::new();
        let left = workspace.active_pane();
        let id = workspace.open_path(path.clone(), "a long original line".into());
        let right = workspace.split(left, Direction::Right).unwrap();
        workspace.editor_mut(left).unwrap().set_cursor(20);
        workspace.editor_mut(right).unwrap().set_cursor(2);
        std::fs::write(&path, "short").unwrap();

        assert_eq!(
            workspace.reconcile_document(id),
            ReconcileResult::Reloaded(path)
        );
        assert_eq!(workspace.editor(left).unwrap().cursor(), 5);
        assert_eq!(workspace.editor(right).unwrap().cursor(), 2);

        let _ = std::fs::remove_dir_all(root);
    }
}
