//! Open disk-backed documents and the active editor buffer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::git::DiffTarget;
use crate::model::buffer::TextBuffer;
use crate::model::diff_document::DiffDocument;
use crate::model::document::{DiskState, FileId, FileMeta};

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

#[derive(Clone)]
struct OpenDocument {
    meta: FileMeta,
    buffer: TextBuffer,
    saved_text: String,
    diff: Option<DiffDocument>,
    /// A built-in page (Settings, Keymap) rather than a file.
    page: Option<AppPage>,
    scroll_x: f32,
    scroll_y: f32,
    disk_state: DiskState,
    disk_conflict: bool,
    missing_on_disk: bool,
}

impl OpenDocument {
    /// Whether this tab edits a file on disk (not a diff or the settings page).
    fn is_text(&self) -> bool {
        self.diff.is_none() && self.page.is_none()
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
    open: Vec<FileId>,
    active: Option<FileId>,
    /// The one replaceable editor tab in this workspace.
    preview: Option<FileId>,
    documents: HashMap<FileId, OpenDocument>,
    paths: HashMap<PathBuf, FileId>,
    next_file_id: u64,
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            open: Vec::new(),
            active: None,
            preview: None,
            documents: HashMap::new(),
            paths: HashMap::new(),
            next_file_id: 1,
        }
    }

    pub fn open_files(&self) -> &[FileId] {
        &self.open
    }

    pub fn open_paths(&self) -> Vec<PathBuf> {
        self.open
            .iter()
            .filter_map(|id| self.documents.get(id))
            .filter(|document| document.is_text())
            .map(|document| document.meta.path.clone())
            .collect()
    }

    /// Disk-backed tabs worth restoring in a later session. Preview tabs are
    /// browsing state, so they intentionally do not survive a restart.
    pub fn persistent_open_paths(&self) -> Vec<PathBuf> {
        self.open
            .iter()
            .filter(|id| self.preview != Some(**id))
            .filter_map(|id| self.documents.get(id))
            .filter(|document| document.is_text())
            .map(|document| document.meta.path.clone())
            .collect()
    }

    /// The active path is persisted only when that same tab is restorable.
    pub fn persistent_active_path(&self) -> Option<&Path> {
        let id = self.active?;
        if self.preview == Some(id) || !self.is_file(id) {
            return None;
        }
        self.meta(id).map(|meta| meta.path.as_path())
    }

    /// Labels for open tabs. Duplicate file names receive the shortest parent
    /// path suffix that distinguishes them from the other open documents.
    pub fn tab_labels(&self) -> HashMap<FileId, String> {
        let mut groups: HashMap<String, Vec<(FileId, &Path)>> = HashMap::new();
        for id in &self.open {
            let Some(document) = self.documents.get(id) else {
                continue;
            };
            groups
                .entry(document.meta.name.clone())
                .or_default()
                .push((*id, document.meta.path.as_path()));
        }

        let mut labels = HashMap::with_capacity(self.open.len());
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

    pub fn active(&self) -> Option<FileId> {
        self.active
    }

    pub fn preview(&self) -> Option<FileId> {
        self.preview
    }

    pub fn is_preview(&self, id: FileId) -> bool {
        self.preview == Some(id)
    }

    pub fn promote_preview(&mut self, id: FileId) -> bool {
        if self.preview == Some(id) {
            self.preview = None;
            true
        } else {
            false
        }
    }

    pub fn promote_active_preview(&mut self) -> bool {
        self.active.is_some_and(|id| self.promote_preview(id))
    }

    pub fn meta(&self, id: FileId) -> Option<&FileMeta> {
        self.documents.get(&id).map(|document| &document.meta)
    }

    pub fn active_meta(&self) -> Option<&FileMeta> {
        self.active.and_then(|id| self.meta(id))
    }

    /// Path of the active file or diff; built-in pages have none.
    pub fn active_path(&self) -> Option<&Path> {
        if self.active_page().is_some() {
            return None;
        }
        self.active_meta().map(|meta| meta.path.as_path())
    }

    pub fn active_diff(&self) -> Option<&DiffDocument> {
        self.active
            .and_then(|id| self.documents.get(&id))
            .and_then(|document| document.diff.as_ref())
    }

    pub fn is_diff(&self, id: FileId) -> bool {
        self.documents
            .get(&id)
            .is_some_and(|document| document.diff.is_some())
    }

    pub fn file_id_for_path(&self, path: &Path) -> Option<FileId> {
        self.paths.get(path).copied()
    }

    pub fn active_buffer(&self) -> Option<&TextBuffer> {
        self.active
            .and_then(|id| self.documents.get(&id))
            .filter(|document| document.is_text())
            .map(|document| &document.buffer)
    }

    pub fn active_buffer_mut(&mut self) -> Option<&mut TextBuffer> {
        self.active
            .and_then(|id| self.documents.get_mut(&id))
            .filter(|document| document.is_text())
            .map(|document| &mut document.buffer)
    }

    pub fn is_dirty(&self, id: FileId) -> bool {
        self.documents.get(&id).is_some_and(|document| {
            document.is_text() && document.buffer.text() != document.saved_text
        })
    }

    pub fn dirty_paths(&self) -> Vec<PathBuf> {
        self.open
            .iter()
            .filter(|id| self.is_dirty(**id))
            .filter_map(|id| self.documents.get(id))
            .map(|document| document.meta.path.clone())
            .collect()
    }

    pub fn dirty_file_ids(&self) -> Vec<FileId> {
        self.open
            .iter()
            .copied()
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

    pub fn active_save_snapshot(&self) -> Option<(FileId, PathBuf, String)> {
        let id = self.active?;
        self.save_snapshot(id)
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

    pub fn active_scroll(&self) -> (f32, f32) {
        self.active
            .and_then(|id| self.documents.get(&id))
            .map_or((0.0, 0.0), |document| {
                (document.scroll_x, document.scroll_y)
            })
    }

    pub fn set_active_scroll(&mut self, x: f32, y: f32) {
        if let Some(document) = self.active.and_then(|id| self.documents.get_mut(&id)) {
            document.scroll_x = x.max(0.0);
            document.scroll_y = y.max(0.0);
        }
    }

    pub fn open_path(&mut self, path: PathBuf, contents: String) -> FileId {
        self.open_path_with_mode(path, contents, OpenMode::Permanent)
    }

    pub fn preview_path(&mut self, path: PathBuf, contents: String) -> FileId {
        self.open_path_with_mode(path, contents, OpenMode::Preview)
    }

    fn open_path_with_mode(&mut self, path: PathBuf, contents: String, mode: OpenMode) -> FileId {
        if let Some(id) = self.file_id_for_path(&path) {
            self.active = Some(id);
            if mode == OpenMode::Permanent {
                self.promote_preview(id);
            }
            return id;
        }

        let replacement_index = if mode == OpenMode::Preview {
            self.preview.and_then(|preview| {
                if self.is_dirty(preview) {
                    // Defensive promotion: UI edit paths promote eagerly, but
                    // no missed path may allow a dirty preview to be replaced.
                    self.preview = None;
                    None
                } else {
                    self.open.iter().position(|id| *id == preview)
                }
            })
        } else {
            None
        };

        if let Some(preview) = self.preview.filter(|_| replacement_index.is_some()) {
            self.close(preview);
        }

        let id = FileId::new(self.next_file_id);
        self.next_file_id += 1;
        let saved_text = contents.clone();
        let disk_state = DiskState::capture(&path, &contents);
        self.documents.insert(
            id,
            OpenDocument {
                meta: FileMeta::from_path(path.clone()),
                buffer: TextBuffer::new(contents),
                saved_text,
                diff: None,
                page: None,
                scroll_x: 0.0,
                scroll_y: 0.0,
                disk_state,
                disk_conflict: false,
                missing_on_disk: false,
            },
        );
        self.paths.insert(path, id);
        if let Some(index) = replacement_index {
            self.open.insert(index.min(self.open.len()), id);
        } else {
            self.open.push(id);
        }
        self.active = Some(id);
        if mode == OpenMode::Preview {
            self.preview = Some(id);
        }
        id
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
            self.active = Some(id);
            return id;
        }

        let id = FileId::new(self.next_file_id);
        self.next_file_id += 1;
        let absolute_path = diff.absolute_path();
        let mut meta = FileMeta::from_path(absolute_path.clone());
        meta.name = diff.title();
        self.documents.insert(
            id,
            OpenDocument {
                meta,
                buffer: TextBuffer::new(String::new()),
                saved_text: String::new(),
                diff: Some(diff),
                page: None,
                scroll_x: 0.0,
                scroll_y: 0.0,
                disk_state: DiskState::capture(&absolute_path, ""),
                disk_conflict: false,
                missing_on_disk: false,
            },
        );
        self.open.push(id);
        self.active = Some(id);
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

    /// Open a built-in page; each page has at most one tab.
    pub fn open_page(&mut self, page: AppPage) -> FileId {
        if let Some(id) = self.page_id(page) {
            self.active = Some(id);
            return id;
        }

        let id = FileId::new(self.next_file_id);
        self.next_file_id += 1;
        let path = PathBuf::from(page.title());
        let mut meta = FileMeta::from_path(path.clone());
        meta.name = page.title().to_owned();
        self.documents.insert(
            id,
            OpenDocument {
                meta,
                buffer: TextBuffer::new(String::new()),
                saved_text: String::new(),
                diff: None,
                page: Some(page),
                scroll_x: 0.0,
                scroll_y: 0.0,
                disk_state: DiskState::capture(&path, ""),
                disk_conflict: false,
                missing_on_disk: false,
            },
        );
        self.open.push(id);
        self.active = Some(id);
        id
    }

    pub fn page_id(&self, page: AppPage) -> Option<FileId> {
        self.open
            .iter()
            .copied()
            .find(|id| self.page(*id) == Some(page))
    }

    /// The built-in page a tab shows, if it is not a file or diff.
    pub fn page(&self, id: FileId) -> Option<AppPage> {
        self.documents.get(&id).and_then(|document| document.page)
    }

    pub fn active_page(&self) -> Option<AppPage> {
        self.active.and_then(|id| self.page(id))
    }

    pub fn settings_id(&self) -> Option<FileId> {
        self.page_id(AppPage::Settings)
    }

    pub fn is_settings(&self, id: FileId) -> bool {
        self.page(id) == Some(AppPage::Settings)
    }

    pub fn active_is_settings(&self) -> bool {
        self.active_page() == Some(AppPage::Settings)
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

    pub fn set_active(&mut self, id: FileId) {
        if self.documents.contains_key(&id) {
            self.active = Some(id);
        }
    }

    pub fn activate_at(&mut self, index: usize) -> bool {
        let Some(id) = self.open.get(index).copied() else {
            return false;
        };
        let changed = self.active != Some(id);
        self.active = Some(id);
        changed
    }

    pub fn activate_last(&mut self) -> bool {
        let Some(id) = self.open.last().copied() else {
            return false;
        };
        let changed = self.active != Some(id);
        self.active = Some(id);
        changed
    }

    /// Move an open tab to a final index while preserving its document and
    /// active state. Returns whether the visible order changed.
    pub fn move_tab(&mut self, id: FileId, target_index: usize) -> bool {
        let Some(source_index) = self.open.iter().position(|&file| file == id) else {
            return false;
        };
        let target_index = target_index.min(self.open.len().saturating_sub(1));
        if source_index == target_index {
            return false;
        }

        let id = self.open.remove(source_index);
        self.open.insert(target_index, id);
        true
    }

    pub fn close(&mut self, id: FileId) {
        if self.preview == Some(id) {
            self.preview = None;
        }
        if let Some(pos) = self.open.iter().position(|&file| file == id) {
            self.open.remove(pos);
            if let Some(document) = self.documents.remove(&id) {
                if document.is_text() {
                    self.paths.remove(&document.meta.path);
                }
            }
            if self.active == Some(id) {
                self.active = self
                    .open
                    .get(pos.min(self.open.len().saturating_sub(1)))
                    .copied();
            }
        }
    }

    pub fn next(&mut self) {
        if let Some(active) = self.active {
            if let Some(pos) = self.open.iter().position(|&file| file == active) {
                self.active = Some(self.open[(pos + 1) % self.open.len()]);
            }
        }
    }

    pub fn prev(&mut self) {
        if let Some(active) = self.active {
            if let Some(pos) = self.open.iter().position(|&file| file == active) {
                self.active = Some(self.open[(pos + self.open.len() - 1) % self.open.len()]);
            }
        }
    }

    /// Reconcile every open document with disk. Clean documents reload
    /// automatically; dirty documents retain their buffer and are marked as a
    /// conflict for explicit user resolution.
    pub fn reconcile_disk(&mut self) -> Vec<ReconcileResult> {
        let ids = self.open.clone();
        ids.into_iter()
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
            .open
            .iter()
            .copied()
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
            replace_buffer_from_disk(&mut document.buffer, contents.clone());
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
        replace_buffer_from_disk(&mut document.buffer, contents.clone());
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

/// External reloads reset edit history but retain the caret/selection as far
/// as the new document length permits. Per-document scroll lives outside the
/// buffer and is therefore preserved as well.
fn replace_buffer_from_disk(buffer: &mut TextBuffer, contents: String) {
    let cursor = buffer.cursor();
    let selection = buffer.selection();
    let mut replacement = TextBuffer::new(contents);
    if let Some(selection) = selection {
        replacement.select_range(selection.start..selection.end);
    } else {
        replacement.set_cursor(cursor);
    }
    *buffer = replacement;
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
        assert!(workspace.active_buffer().is_none());
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
        assert_eq!(workspace.open_files(), &[first, second]);
        assert_eq!(
            workspace.open_paths(),
            vec![PathBuf::from("src/main.rs"), PathBuf::from("README.md")]
        );
        assert_eq!(workspace.active_buffer().unwrap().text(), "fn main() {}");
    }

    #[test]
    fn a_new_preview_replaces_the_clean_preview_at_the_same_index() {
        let mut workspace = Workspace::new();
        let permanent = workspace.open_path(PathBuf::from("permanent.rs"), String::new());
        let first = workspace.preview_path(PathBuf::from("first.rs"), "first".into());
        let trailing = workspace.open_path(PathBuf::from("trailing.rs"), String::new());
        workspace.set_active(first);

        let second = workspace.preview_path(PathBuf::from("second.rs"), "second".into());

        assert_eq!(workspace.open_files(), &[permanent, second, trailing]);
        assert_eq!(workspace.preview(), Some(second));
        assert_eq!(workspace.active(), Some(second));
        assert_eq!(workspace.file_id_for_path(Path::new("first.rs")), None);
    }

    #[test]
    fn editing_a_preview_defensively_preserves_it_before_the_next_preview() {
        let mut workspace = Workspace::new();
        let first = workspace.preview_path(PathBuf::from("first.rs"), String::new());
        workspace.active_buffer_mut().unwrap().insert("changed");

        let second = workspace.preview_path(PathBuf::from("second.rs"), String::new());

        assert_eq!(workspace.open_files(), &[first, second]);
        assert!(!workspace.is_preview(first));
        assert!(workspace.is_preview(second));
        assert!(workspace.is_dirty(first));
    }

    #[test]
    fn permanently_reopening_a_preview_promotes_without_duplication() {
        let mut workspace = Workspace::new();
        let path = PathBuf::from("main.rs");
        let preview = workspace.preview_path(path.clone(), "original".into());

        let reopened = workspace.open_path(path, "ignored".into());

        assert_eq!(reopened, preview);
        assert_eq!(workspace.open_files(), &[preview]);
        assert_eq!(workspace.preview(), None);
        assert_eq!(workspace.active_buffer().unwrap().text(), "original");
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
        assert_eq!(workspace.open_files(), &[permanent, preview]);
    }

    #[test]
    fn preview_tabs_are_excluded_from_session_state() {
        let mut workspace = Workspace::new();
        let permanent_path = PathBuf::from("permanent.rs");
        let preview_path = PathBuf::from("preview.rs");
        let permanent = workspace.open_path(permanent_path.clone(), String::new());
        workspace.preview_path(preview_path, String::new());

        assert_eq!(
            workspace.persistent_open_paths(),
            vec![permanent_path.clone()]
        );
        assert_eq!(workspace.persistent_active_path(), None);

        workspace.set_active(permanent);
        assert_eq!(
            workspace.persistent_active_path(),
            Some(permanent_path.as_path())
        );
    }

    #[test]
    fn closing_a_preview_clears_the_preview_slot() {
        let mut workspace = Workspace::new();
        let preview = workspace.preview_path(PathBuf::from("preview.rs"), String::new());

        workspace.close(preview);

        assert_eq!(workspace.preview(), None);
        assert!(workspace.open_files().is_empty());
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
        assert!(workspace.active_buffer().is_none());
        assert_eq!(workspace.open_files(), &[file, first]);
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
        workspace.active_buffer_mut().unwrap().move_end();
        assert!(!workspace.is_dirty(file));

        workspace.active_buffer_mut().unwrap().insert("!");
        assert!(workspace.is_dirty(file));

        let (snapshot_id, path, contents) = workspace.active_save_snapshot().unwrap();
        assert_eq!(snapshot_id, file);
        assert_eq!(path, PathBuf::from("notes.txt"));
        assert_eq!(contents, "hello!");
        assert!(workspace.mark_saved(file));
        assert!(!workspace.is_dirty(file));

        workspace.active_buffer_mut().unwrap().backspace();
        assert!(workspace.is_dirty(file));
    }

    #[test]
    fn dirty_file_ids_follow_tab_order_and_exclude_clean_files() {
        let mut workspace = Workspace::new();
        let first = workspace.open_path(PathBuf::from("first.rs"), String::new());
        workspace
            .active_buffer_mut()
            .unwrap()
            .insert("first change");
        let clean = workspace.open_path(PathBuf::from("clean.rs"), String::new());
        let second = workspace.open_path(PathBuf::from("second.rs"), String::new());
        workspace
            .active_buffer_mut()
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

        assert!(workspace.move_tab(first, 2));
        assert_eq!(workspace.open_files(), &[second, third, first]);
        assert_eq!(workspace.active(), Some(third));

        assert!(workspace.move_tab(first, 0));
        assert_eq!(workspace.open_files(), &[first, second, third]);
        assert!(!workspace.move_tab(first, 0));
    }

    #[test]
    fn duplicate_tab_names_use_the_shortest_distinguishing_parent_suffix() {
        let mut workspace = Workspace::new();
        let first = workspace.open_path(PathBuf::from("alpha/src/main.rs"), String::new());
        let second = workspace.open_path(PathBuf::from("beta/src/main.rs"), String::new());
        let unique = workspace.open_path(PathBuf::from("beta/src/lib.rs"), String::new());

        let labels = workspace.tab_labels();
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
        workspace.active_buffer_mut().unwrap().move_end();
        workspace.active_buffer_mut().unwrap().insert(" + editor");
        std::fs::write(&path, "disk two with size").unwrap();

        assert_eq!(
            workspace.reconcile_document(id),
            ReconcileResult::Conflict(path.clone())
        );
        assert_eq!(
            workspace.active_buffer().unwrap().text(),
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
        workspace.active_buffer_mut().unwrap().select_range(2..4);
        workspace.set_active_scroll(12.0, 34.0);
        std::fs::write(&path, "after and longer").unwrap();

        assert_eq!(
            workspace.reconcile_document(id),
            ReconcileResult::Reloaded(path.clone())
        );
        assert_eq!(
            workspace.active_buffer().unwrap().text(),
            "after and longer"
        );
        assert_eq!(workspace.active_buffer().unwrap().cursor(), 4);
        assert_eq!(workspace.active_buffer().unwrap().selection(), Some(2..4));
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
        assert_eq!(workspace.active_buffer().unwrap().text(), "contents");
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
        workspace.active_buffer_mut().unwrap().move_end();
        workspace.active_buffer_mut().unwrap().insert(" saved");
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
}
