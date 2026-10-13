//! Open documents and the editor panes that show them.
//!
//! Documents are shared: a file opened in two panes has one text buffer, one
//! undo history and one dirty state. Each pane keeps its own tab list,
//! preview slot and, per document, its own selection and scroll position.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::domain::document::{DiskRead, DiskState, FileId, FileMeta};
use crate::domain::geometry::Rect;
use crate::domain::pane_layout::{Direction, PaneId, PaneNode};
use loom_editor_core::text::{Editor, EditorMut, Selection, TextBuffer};

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
pub(crate) struct OpenDocument<D, K> {
    pub(crate) meta: FileMeta,
    pub(crate) buffer: TextBuffer,
    pub(crate) saved_text: Arc<String>,
    pub(crate) dirty_cache: Arc<Mutex<(u64, usize, usize, bool)>>,
    pub(crate) diff: Option<Arc<D>>,
    pub(crate) collapsed_diffs: BTreeSet<K>,
    pub(crate) focused_diff: Option<K>,
    /// A built-in page (Settings, Keymap) rather than a file.
    pub(crate) page: Option<AppPage>,
    /// One view per pane that has this document as a tab.
    pub(crate) views: HashMap<PaneId, ViewState>,
    pub(crate) disk_state: DiskState,
    pub(crate) disk_conflict: bool,
    pub(crate) missing_on_disk: bool,
    /// A restored tab has metadata before its text is ready. Never expose its
    /// empty staging buffer to editing, saving or disk reconciliation.
    pub(crate) loading: Option<bool>,
}

impl<D: Clone, K: Clone + Ord> OpenDocument<D, K> {
    pub(crate) fn new(meta: FileMeta, contents: String, disk_state: DiskState) -> Self {
        let buffer = TextBuffer::new(contents);
        let saved_text = buffer.text_snapshot();
        Self {
            meta,
            buffer,
            saved_text,
            dirty_cache: Arc::new(Mutex::new((u64::MAX, 0, 0, false))),
            diff: None,
            collapsed_diffs: BTreeSet::new(),
            focused_diff: None,
            page: None,
            views: HashMap::new(),
            disk_state,
            disk_conflict: false,
            missing_on_disk: false,
            loading: None,
        }
    }

    /// Whether this tab edits a file on disk (not a diff or the settings page).
    pub(crate) fn is_text(&self) -> bool {
        self.is_shareable() && self.loading.is_none()
    }

    pub(crate) fn is_dirty(&self) -> bool {
        let version = self.buffer.version();
        let saved = Arc::as_ptr(&self.saved_text) as usize;
        let text = self.buffer.snapshot_identity();
        let mut cache = self.dirty_cache.lock().unwrap();
        let (previous, identity, snapshot, dirty) = *cache;
        if version == previous && saved == identity && snapshot == text {
            return dirty;
        }
        let dirty = self.is_text() && !self.buffer.matches_snapshot(&self.saved_text);
        *cache = (version, saved, text, dirty);
        dirty
    }

    /// Diffs and built-in pages exist once; only text files can be shown in
    /// several panes at the same time.
    pub(crate) fn is_shareable(&self) -> bool {
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
    pub(crate) fn clamp_views(&mut self) {
        for view in self.views.values_mut() {
            view.selection = self.buffer.clamp(view.selection);
        }
    }
}

/// One editor pane: an ordered tab list with an active and a preview tab.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pane {
    pub(crate) items: Vec<FileId>,
    pub(crate) active: Option<FileId>,
    /// The one replaceable tab of this pane.
    pub(crate) preview: Option<FileId>,
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

#[derive(Clone)]
pub struct PreparedDiskUpdate<D, K> {
    pub(crate) id: FileId,
    pub(crate) path: PathBuf,
    pub(crate) expected_disk: DiskState,
    pub(crate) expected_saved: Arc<String>,
    pub(crate) expected_version: u64,
    pub(crate) replacement: OpenDocument<D, K>,
    pub(crate) outcome: ReconcileResult,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenMode {
    Preview,
    Permanent,
}

/// Constructed entirely on a worker; installing it does not scan the text.
pub(crate) struct PreparedTextFile<D, K>(OpenDocument<D, K>);

impl<D: Clone, K: Clone + Ord> PreparedTextFile<D, K> {
    pub fn with_disk_state(path: PathBuf, contents: String, disk: DiskState) -> Self {
        Self(OpenDocument::new(FileMeta::from_path(path), contents, disk))
    }
}

#[derive(Clone)]
pub struct Workspace<D, K> {
    pub(crate) documents: HashMap<FileId, OpenDocument<D, K>>,
    pub(crate) paths: HashMap<PathBuf, FileId>,
    pub(crate) panes: HashMap<PaneId, Pane>,
    pub(crate) layout: PaneNode,
    pub(crate) active_pane: PaneId,
    /// Panes by recency of focus, most recent first.
    pub(crate) pane_mru: Vec<PaneId>,
    pub(crate) next_file_id: u64,
    pub(crate) next_pane_id: u64,
    retired: Vec<Retired<D, K>>,
}

#[derive(Clone)]
pub(crate) enum Retired<D, K> {
    Document(OpenDocument<D, K>),
    Update(PreparedDiskUpdate<D, K>),
}

impl<D: Clone, K: Clone + Ord> Workspace<D, K> {
    pub(crate) fn take_retired(&mut self) -> Vec<Retired<D, K>> {
        std::mem::take(&mut self.retired)
    }

    pub fn mark_saved_with_disk(&mut self, id: FileId, disk: DiskState) -> bool {
        let Some(document) = self.documents.get_mut(&id) else {
            return false;
        };
        if !document.is_text() {
            return false;
        }
        document.saved_text = Arc::new(document.buffer.text().to_owned());
        document.disk_state = disk;
        document.disk_conflict = false;
        document.missing_on_disk = false;
        document.buffer.break_undo_group();
        true
    }

    pub fn reconcile_with_disk(&mut self, id: FileId, observation: DiskRead) -> ReconcileResult {
        let Some(document) = self.documents.get(&id) else {
            return ReconcileResult::Failed(PathBuf::new(), "Document is not open.".into());
        };
        let path = document.meta.path.clone();
        if !document.is_text() {
            return ReconcileResult::Unchanged(path);
        }
        let dirty = document.buffer.text() != document.saved_text.as_str();
        let previous = document.disk_state.clone();
        let (contents, current) = match observation {
            DiskRead::Found { contents, state } => (contents, state),
            DiskRead::Missing => {
                if let Some(document) = self.documents.get_mut(&id) {
                    document.missing_on_disk = true;
                    document.disk_conflict = dirty;
                }
                return ReconcileResult::Missing(path);
            }
            DiskRead::Failed(error) => return ReconcileResult::Failed(path, error),
        };
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
            document.saved_text = Arc::new(contents);
            document.disk_state = current;
            document.disk_conflict = false;
            ReconcileResult::Reloaded(path)
        }
    }

    pub fn accept_disk_with(
        &mut self,
        id: FileId,
        contents: String,
        disk: DiskState,
    ) -> Result<(), String> {
        let document = self
            .documents
            .get_mut(&id)
            .ok_or_else(|| "Document is not open.".to_owned())?;
        if !document.is_text() {
            return Err("Diff documents are read-only.".to_owned());
        }
        replace_buffer_from_disk(document, contents.clone());
        document.saved_text = Arc::new(contents);
        document.disk_state = disk;
        document.disk_conflict = false;
        document.missing_on_disk = false;
        Ok(())
    }

    pub fn keep_editor_with_disk(&mut self, id: FileId, disk: Option<DiskState>) -> bool {
        let Some(document) = self.documents.get_mut(&id) else {
            return false;
        };
        if !document.is_text() {
            return false;
        }
        if let Some(state) = disk {
            document.disk_state = state;
        }
        document.disk_conflict = false;
        true
    }

    pub fn move_path_with_disk(
        &mut self,
        old_path: &Path,
        new_path: PathBuf,
        disk: DiskState,
    ) -> bool {
        let Some(id) = self.paths.remove(old_path) else {
            return false;
        };
        let Some(document) = self.documents.get_mut(&id) else {
            return false;
        };
        document.meta = FileMeta::from_path(new_path.clone());
        document.disk_state = disk;
        document.missing_on_disk = false;
        self.paths.insert(new_path, id);
        true
    }

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
            retired: Vec::new(),
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

    pub(crate) fn active_pane_ref(&self) -> &Pane {
        &self.panes[&self.active_pane]
    }

    pub(crate) fn active_pane_mut(&mut self) -> &mut Pane {
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
        const AREA: Rect = Rect::new(0.0, 0.0, 10_000.0, 10_000.0);
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
    pub(crate) fn insert_item(
        &mut self,
        pane: PaneId,
        id: FileId,
        index: usize,
        view_source: Option<PaneId>,
    ) {
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
        if orphaned && let Some(document) = self.documents.remove(&id) {
            if document.is_shareable() {
                self.paths.remove(&document.meta.path);
            }
            self.retired.push(Retired::Document(document));
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

    pub fn active_diff(&self) -> Option<&D> {
        self.active()
            .and_then(|id| self.documents.get(&id))
            .and_then(|document| document.diff.as_deref())
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

    /// The editor of one specific view: `document` as shown in `pane`, whether
    /// or not it is the pane's active tab.
    pub fn view_editor_mut(&mut self, pane: PaneId, document: FileId) -> Option<EditorMut<'_>> {
        if !self.panes.get(&pane)?.contains(document) {
            return None;
        }
        self.documents
            .get_mut(&document)
            .filter(|d| d.is_text())?
            .editor_mut(pane)
    }

    pub fn editor_mut(&mut self, pane: PaneId) -> Option<EditorMut<'_>> {
        let id = self.panes.get(&pane)?.active?;
        self.documents
            .get_mut(&id)
            .filter(|d| d.is_text())?
            .editor_mut(pane)
    }

    /// The diff shown by a pane's active tab.
    pub fn diff(&self, pane: PaneId) -> Option<&D> {
        let id = self.panes.get(&pane)?.active?;
        self.documents.get(&id)?.diff.as_deref()
    }

    pub fn collapsed_diffs(&self, pane: PaneId) -> BTreeSet<K> {
        self.panes
            .get(&pane)
            .and_then(|p| p.active)
            .and_then(|id| self.documents.get(&id))
            .map(|document| document.collapsed_diffs.clone())
            .unwrap_or_default()
    }

    pub fn toggle_diff_section(&mut self, pane: PaneId, request: K) {
        let Some(id) = self.panes.get(&pane).and_then(|p| p.active) else {
            return;
        };
        let Some(document) = self.documents.get_mut(&id) else {
            return;
        };
        if !document.collapsed_diffs.insert(request.clone()) {
            document.collapsed_diffs.remove(&request);
        }
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

    /// Keeps every code view on the same line after the code font changes.
    /// `text` and `diff` are the new-to-old ratios of the editor line height
    /// and the diff row height.
    pub fn rescale_code_scroll(&mut self, text: f32, diff: f32) {
        for document in self.documents.values_mut() {
            if document.page.is_some() {
                continue;
            }
            let factor = if document.diff.is_some() { diff } else { text };
            for view in document.views.values_mut() {
                view.scroll_y *= factor;
            }
        }
    }

    pub fn set_active_scroll(&mut self, x: f32, y: f32) {
        self.set_scroll(self.active_pane, x, y);
    }

    pub fn active_save_snapshot(&self) -> Option<(FileId, PathBuf, String)> {
        self.save_snapshot(self.active()?)
    }

    // ---- Documents ---------------------------------------------------------

    /// Every open document, in pane order then tab order, without repeats.
    pub(crate) fn ordered_documents(&self) -> Vec<FileId> {
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

    /// The diff shown by tab `id`, if it is a diff tab.
    pub fn diff_of(&self, id: FileId) -> Option<&D> {
        self.documents.get(&id)?.diff.as_deref()
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
        self.documents
            .get(&id)
            .is_some_and(|document| document.is_dirty())
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

    /// Documents are shared across panes, so each text document appears once.
    pub fn text_snapshots(&self) -> Vec<crate::domain::document_snapshot::DocumentSnapshot> {
        self.documents
            .values()
            .filter(|document| document.is_text())
            .map(
                |document| crate::domain::document_snapshot::DocumentSnapshot {
                    path: document.meta.path.clone(),
                    text: document.buffer.text_snapshot(),
                    version: document.buffer.version(),
                },
            )
            .collect()
    }

    pub fn dirty_text_versions(&self) -> Vec<(PathBuf, u64)> {
        let mut versions = self
            .documents
            .values()
            .filter(|document| {
                document.is_text() && document.buffer.text() != document.saved_text.as_str()
            })
            .map(|document| (document.meta.path.clone(), document.buffer.version()))
            .collect::<Vec<_>>();
        versions.sort();
        versions
    }

    // ---- Opening -----------------------------------------------------------

    /// Restore only metadata and tab order on the UI thread, without touching
    /// the file or building a line index. The background restorer fills it in.
    pub(crate) fn open_deferred(&mut self, path: PathBuf) -> FileId {
        if let Some(id) = self.file_id_for_path(&path) {
            return self.attach_text_tab(id, OpenMode::Permanent);
        }
        let mut document = OpenDocument::new(
            FileMeta::from_path(path.clone()),
            String::new(),
            DiskState {
                modified_time: None,
                size: 0,
                content_hash: 0,
            },
        );
        document.loading = Some(false);
        let id = self.allocate_file_id();
        self.documents.insert(id, document);
        self.paths.insert(path, id);
        let pane = self.active_pane;
        self.insert_item(pane, id, usize::MAX, None);
        self.active_pane_mut().active = Some(id);
        id
    }

    pub(crate) fn is_loading(&self, id: FileId) -> bool {
        self.documents.get(&id).is_some_and(|d| d.loading.is_some())
    }

    pub(crate) fn loading_indicator(&self, id: FileId) -> bool {
        self.documents
            .get(&id)
            .is_some_and(|d| d.loading == Some(true))
    }

    pub(crate) fn show_restore_indicators(&mut self) -> bool {
        let mut changed = false;
        for document in self.documents.values_mut() {
            if document.loading == Some(false) {
                document.loading = Some(true);
                changed = true;
            }
        }
        changed
    }

    /// Active documents first, followed by tabs in visible order. Shared
    /// documents are prepared once even when shown in more than one pane.
    pub(crate) fn loading_files(&self) -> Vec<(FileId, PathBuf)> {
        let mut ids = Vec::new();
        if let Some(id) = self.active() {
            ids.push(id);
        }
        for pane in self.pane_ids() {
            if let Some(id) = self.panes[&pane].active {
                ids.push(id);
            }
        }
        for pane in self.pane_ids() {
            ids.extend(&self.panes[&pane].items);
        }
        let mut seen = std::collections::HashSet::new();
        ids.into_iter()
            .filter(|id| seen.insert(*id) && self.is_loading(*id))
            .filter_map(|id| self.meta(id).map(|m| (id, m.path.clone())))
            .collect()
    }

    /// Fill a surviving placeholder in place. Loading must not reactivate,
    /// reorder or resurrect a tab the user moved or closed in the meantime.
    pub(crate) fn install_restored(
        &mut self,
        id: FileId,
        path: &Path,
        prepared: PreparedTextFile<D, K>,
    ) -> bool {
        let Some(document) = self
            .documents
            .get_mut(&id)
            .filter(|d| d.loading.is_some() && d.meta.path == path && prepared.0.meta.path == path)
        else {
            self.retired.push(Retired::Document(prepared.0));
            return false;
        };
        let mut replacement = prepared.0;
        replacement.views = std::mem::take(&mut document.views);
        self.retired
            .push(Retired::Document(std::mem::replace(document, replacement)));
        true
    }

    pub(crate) fn attach_text_tab(&mut self, id: FileId, mode: OpenMode) -> FileId {
        let pane = self.active_pane;
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
        id
    }

    pub(crate) fn open_prepared_text(
        &mut self,
        prepared: PreparedTextFile<D, K>,
        mode: OpenMode,
    ) -> FileId {
        let path = prepared.0.meta.path.clone();
        if let Some(id) = self.file_id_for_path(&path) {
            self.retired.push(Retired::Document(prepared.0));
            return self.attach_text_tab(id, mode);
        }
        let pane = self.active_pane;
        let replacement_index = self.preview_replacement_index(pane, mode);
        let id = self.allocate_file_id();
        self.documents.insert(id, prepared.0);
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

    pub(crate) fn allocate_file_id(&mut self) -> FileId {
        let id = FileId::new(self.next_file_id);
        self.next_file_id += 1;
        id
    }

    /// Activate an existing single-instance tab wherever it is.
    pub(crate) fn reveal(&mut self, id: FileId) -> bool {
        match self.pane_of(id) {
            Some(pane) => {
                self.activate(pane, id);
                true
            }
            None => false,
        }
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
        let mut document = OpenDocument::new(meta, String::new(), DiskState::from_contents(""));
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
        self.documents
            .get(&id)
            .is_some_and(OpenDocument::is_shareable)
    }

    // ---- Disk --------------------------------------------------------------

    /// Delayed disk notifications never overwrite a save, rename, closed tab,
    /// or editor changes made while the worker was reading the file.
    pub fn apply_disk_updates(
        &mut self,
        updates: Vec<PreparedDiskUpdate<D, K>>,
    ) -> Vec<ReconcileResult> {
        let mut outcomes = Vec::new();
        let mut retired = Vec::new();
        for update in updates {
            let Some(document) = self.documents.get_mut(&update.id) else {
                retired.push(update);
                continue;
            };
            if document.meta.path != update.path
                || document.disk_state != update.expected_disk
                || !Arc::ptr_eq(&document.saved_text, &update.expected_saved)
            {
                retired.push(update);
                continue;
            }
            if matches!(update.outcome, ReconcileResult::Reloaded(_))
                && document.buffer.version() != update.expected_version
            {
                document.disk_conflict = true;
                outcomes.push(ReconcileResult::Conflict(update.path.clone()));
                retired.push(update);
                continue;
            }
            let old = document.clone();
            if matches!(update.outcome, ReconcileResult::Reloaded(_)) {
                document
                    .buffer
                    .install_prepared(update.replacement.buffer.clone());
                document.saved_text = update.replacement.saved_text.clone();
                document.clamp_views();
            }
            document.disk_state = update.replacement.disk_state.clone();
            document.disk_conflict = update.replacement.disk_conflict;
            if matches!(update.outcome, ReconcileResult::Missing(_))
                && document.buffer.version() != update.expected_version
            {
                document.disk_conflict = true;
            }
            document.missing_on_disk = update.replacement.missing_on_disk;
            outcomes.push(update.outcome.clone());
            // Keep old shared text alive until the reclaimer frees it.
            self.retired.push(Retired::Document(old));
            retired.push(update);
        }
        self.retired
            .extend(retired.into_iter().map(Retired::Update));
        outcomes
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
pub(crate) fn replace_buffer_from_disk<D: Clone, K: Clone + Ord>(
    document: &mut OpenDocument<D, K>,
    contents: String,
) {
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
mod tests;
