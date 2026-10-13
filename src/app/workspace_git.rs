//! Git payload adaptation stays outside the pure workspace rules.
use super::workspace::*;
use crate::domain::document::{DiskState, FileMeta};
use crate::domain::identity::{FileId, PaneId};
use crate::git::{DiffContent, DiffRequest, DiffTarget, GitResult, PollTarget};
use crate::model::diff_document::DiffDocument;
use std::path::Path;
use std::sync::Arc;
type OpenDocument = crate::domain::workspace::OpenDocument<DiffDocument, DiffRequest>;
impl Workspace {
    pub fn open_diff(&mut self, diff: DiffDocument) -> FileId {
        self.open_prepared_diff(Arc::new(diff))
    }

    pub fn open_prepared_file_changes(&mut self, diff: Arc<DiffDocument>) -> FileId {
        if let Some((&id, document)) = self.core.documents.iter_mut().find(|(_, document)| {
            document.diff.as_ref().is_some_and(|current| {
                current.repository_root == diff.repository_root
                    && current.sections.is_none()
                    && current.revision.is_none()
            })
        }) {
            let changed_file = document
                .diff
                .as_ref()
                .is_none_or(|current| !current.same_view(&diff));
            document.meta.name = diff.title();
            document.meta.path = diff.absolute_path();
            document.focused_diff = None;
            if changed_file {
                for view in document.views.values_mut() {
                    view.scroll_x = 0.0;
                    view.scroll_y = 0.0;
                }
            }
            crate::background_ui::retire(document.diff.replace(diff));
            self.reveal(id);
            return id;
        }
        self.open_prepared_diff(diff)
    }

    pub fn open_prepared_commit_changes(&mut self, diff: Arc<DiffDocument>) -> FileId {
        if let Some((&id, document)) = self.core.documents.iter_mut().find(|(_, document)| {
            document.diff.as_ref().is_some_and(|current| {
                current.repository_root == diff.repository_root
                    && current.sections.is_some()
                    && current.revision.is_some()
            })
        }) {
            let changed_commit = document
                .diff
                .as_ref()
                .is_none_or(|current| current.revision != diff.revision);
            document.meta.name = diff.title();
            document.meta.path = diff.absolute_path();
            if changed_commit {
                document.collapsed_diffs.clear();
                document.focused_diff = None;
                for view in document.views.values_mut() {
                    view.scroll_x = 0.0;
                    view.scroll_y = 0.0;
                }
            }
            crate::background_ui::retire(document.diff.replace(diff));
            self.reveal(id);
            return id;
        }
        self.open_prepared_diff(diff)
    }

    pub fn commit_changes_id(&self, repository_root: &Path, revision: &str) -> Option<FileId> {
        self.core.documents.iter().find_map(|(&id, document)| {
            document
                .diff
                .as_ref()
                .is_some_and(|diff| {
                    diff.repository_root == repository_root
                        && diff.sections.is_some()
                        && diff.revision.as_deref() == Some(revision)
                })
                .then_some(id)
        })
    }

    pub fn open_prepared_diff(&mut self, diff: Arc<DiffDocument>) -> FileId {
        if let Some((&id, document)) = self.core.documents.iter_mut().find(|(_, document)| {
            document
                .diff
                .as_ref()
                .is_some_and(|current| current.same_view(&diff))
        }) {
            document.meta.name = diff.title();
            document.meta.path = diff.absolute_path();
            crate::background_ui::retire(document.diff.replace(diff));
            self.reveal(id);
            return id;
        }

        self.insert_prepared_diff(self.core.active_pane, diff)
    }

    pub fn open_prepared_diff_in(
        &mut self,
        pane: PaneId,
        diff: Arc<DiffDocument>,
    ) -> Option<FileId> {
        let existing = self
            .core
            .panes
            .get(&pane)?
            .items
            .iter()
            .copied()
            .find(|id| {
                self.core.documents[id]
                    .diff
                    .as_ref()
                    .is_some_and(|current| current.same_view(&diff))
            });
        self.activate_pane(pane);
        if let Some(id) = existing {
            let document = self.core.documents.get_mut(&id).unwrap();
            document.meta.name = diff.title();
            document.meta.path = diff.absolute_path();
            crate::background_ui::retire(document.diff.replace(diff));
            self.set_active(id);
            Some(id)
        } else {
            Some(self.insert_prepared_diff(pane, diff))
        }
    }

    fn insert_prepared_diff(&mut self, pane: PaneId, diff: Arc<DiffDocument>) -> FileId {
        let id = self.allocate_file_id();
        let absolute_path = diff.absolute_path();
        let mut meta = FileMeta::from_path(absolute_path.clone());
        meta.name = diff.title();
        let mut document = OpenDocument::new(
            meta,
            String::new(),
            DiskState {
                modified_time: None,
                size: 0,
                content_hash: 0,
            },
        );
        document.diff = Some(diff);
        self.core.documents.insert(id, document);
        self.insert_item(pane, id, usize::MAX, None);
        self.core.panes.get_mut(&pane).unwrap().active = Some(id);
        id
    }

    pub fn git_poll_target(&self) -> PollTarget {
        let mut diffs = self
            .documents
            .values()
            // A commit's diff never changes, so it is not refreshed.
            .filter_map(|document| {
                document
                    .diff
                    .as_ref()
                    .filter(|diff| diff.revision.is_none())
            })
            .map(|diff| DiffRequest {
                repository_root: diff.repository_root.clone(),
                path: diff.path.clone(),
                target: diff.target,
            })
            .collect::<Vec<_>>();
        diffs.sort();
        diffs.dedup();
        PollTarget {
            active_path: self.active_path().map(Path::to_path_buf),
            diffs,
            pinned_repository: None,
        }
    }

    pub fn apply_git_diffs(
        &mut self,
        diffs: &std::collections::BTreeMap<DiffRequest, GitResult<DiffContent>>,
    ) -> bool {
        let mut changed = false;
        for document in self.core.documents.values_mut() {
            let Some(current) = &document.diff else {
                continue;
            };
            let request = DiffRequest {
                repository_root: current.repository_root.clone(),
                path: current.path.clone(),
                target: current.target,
            };
            let Some(Ok(content)) = diffs.get(&request) else {
                continue;
            };
            let next = DiffDocument::from_content(request, content.clone());
            if **current != next {
                document.diff = Some(Arc::new(next));
                changed = true;
            }
        }
        changed
    }

    pub fn apply_prepared_git_diffs(
        &mut self,
        diffs: &std::collections::BTreeMap<DiffRequest, Arc<DiffDocument>>,
        split: bool,
    ) -> Vec<Arc<DiffDocument>> {
        let mut retired = Vec::new();
        for document in self.core.documents.values_mut() {
            let Some(current) = &document.diff else {
                continue;
            };
            let request = DiffRequest {
                repository_root: current.repository_root.clone(),
                path: current.path.clone(),
                target: current.target,
            };
            if let Some(next) = diffs.get(&request)
                && !Arc::ptr_eq(current, next)
            {
                if let (Some(old_sections), Some(new_sections)) =
                    (&current.sections, &next.sections)
                {
                    let old_layout = current.changes_layout(split, &document.collapsed_diffs);
                    let new_layout = next.changes_layout(split, &document.collapsed_diffs);
                    for view in document.views.values_mut() {
                        let row = view.scroll_y / crate::model::diff_document::row_height();
                        if let Some(old) = old_layout
                            .sections
                            .iter()
                            .rfind(|item| item.top as f32 <= row)
                        {
                            let request = old_sections[old.index].request();
                            if let Some(new) = new_layout
                                .sections
                                .iter()
                                .find(|item| new_sections[item.index].request() == request)
                            {
                                view.scroll_y = (new.top as f32 + row - old.top as f32).max(0.0)
                                    * crate::model::diff_document::row_height();
                            }
                        }
                    }
                }
                retired.push(document.diff.replace(next.clone()).unwrap());
            }
        }
        retired
    }

    pub fn focus_diff_section(&mut self, request: &DiffRequest, split: bool) {
        let pane = self.core.active_pane;
        let Some(id) = self.active() else { return };
        let Some(document) = self.core.documents.get_mut(&id) else {
            return;
        };
        let Some(diff) = &document.diff else { return };
        let Some(sections) = &diff.sections else {
            return;
        };
        document.focused_diff = Some(request.clone());
        document.collapsed_diffs.remove(request);
        let layout = diff.changes_layout(split, &document.collapsed_diffs);
        if let Some(section) = layout
            .sections
            .iter()
            .find(|item| sections[item.index].request() == *request)
            && let Some(view) = document.views.get_mut(&pane)
        {
            view.scroll_y = section.top as f32 * crate::model::diff_document::row_height();
        }
    }

    pub fn active_diff_focus(&self) -> Option<&DiffRequest> {
        let document = self.core.documents.get(&self.active()?)?;
        document.diff.as_ref()?.sections.as_ref()?;
        document.focused_diff.as_ref()
    }

    pub fn diff_id_for(
        &self,
        repository_root: &Path,
        path: &Path,
        target: DiffTarget,
    ) -> Option<FileId> {
        self.core.documents.iter().find_map(|(&id, document)| {
            document
                .diff
                .as_ref()
                .is_some_and(|diff| diff.matches(repository_root, path, target))
                .then_some(id)
        })
    }
}
