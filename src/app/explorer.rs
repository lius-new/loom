//! Explorer commands: open, add and remove workspace folders, inline create
//! and rename, trash, reveal, and opening files from the drawers. Each
//! command keeps Workspace and Explorer changes in one root-state update;
//! disk access goes through the Explorer filesystem adapter.

use std::path::{Path, PathBuf};

use lgui::dialogs::{FileDialogOptions, system_file_dialogs};
use lgui::prelude::State;

use crate::features::explorer::model::Reveal;
use crate::features::explorer::ports::FileOpsPort;
use crate::features::explorer::{ExplorerContextTarget, ExplorerCreateKind, ExplorerTargetKind};
use crate::features::explorer::{edit, roots};
use crate::infrastructure::explorer_filesystem::ExplorerFilesystem;
use crate::model::document::FileId;
use crate::state::{AppState, TabDrop};
use crate::workspace_actions::read_text_file;
use crate::workspace_persistence;

const TREE_ROW_H: f32 = 20.0;

/// Open a file clicked in the Explorer: a single click previews it, a double
/// click keeps it open. Returns whether the editor should take focus.
pub(crate) fn open_tree_file(app: &mut AppState, path: PathBuf, clicks: u8) -> bool {
    if app.workspace.file_id_for_path(&path).is_some() {
        crate::file_loading::cancel(app);
        return open_tree_file_with(app, path, clicks, |_| Ok(String::new()));
    } else if crate::file_loading::request(
        app,
        path.clone(),
        if clicks > 1 {
            crate::model::workspace::OpenMode::Permanent
        } else {
            crate::model::workspace::OpenMode::Preview
        },
        None,
    ) {
        return true;
    }
    open_tree_file_with(app, path, clicks, read_text_file)
}

fn open_tree_file_with(
    app: &mut AppState,
    path: PathBuf,
    clicks: u8,
    read: impl FnOnce(&Path) -> Result<String, String>,
) -> bool {
    if let Some(id) = app.workspace.file_id_for_path(&path)
        && app.workspace.active_items().contains(&id)
    {
        app.workspace.set_active(id);
        if clicks > 1 {
            app.workspace.promote_active_preview();
        }
        return true;
    }
    match read(&path) {
        Ok(contents) => {
            if clicks > 1 {
                app.workspace.open_path(path, contents);
            } else {
                app.workspace.preview_path(path, contents);
            }
            app.toast = None;
            true
        }
        Err(message) => {
            app.show_error(message);
            false
        }
    }
}

/// Release a pressed Explorer file: open it like a click when it was not
/// dragged, otherwise open it where it was dropped. Returns whether the
/// editor should take focus.
pub(crate) fn finish_file_drag(app: &mut AppState) -> bool {
    if app
        .runtime()
        .is_some_and(|runtime| runtime.file_loader().is_some())
    {
        if let Some(drag) = app.editor.file_drag.take() {
            let crate::state::FileDragSource::Explorer { clicks } = drag.source else {
                return false;
            };
            if !drag.active {
                return open_tree_file(app, drag.path, clicks);
            }
            if let Some(drop) = drag.drop {
                return crate::file_loading::request(
                    app,
                    drag.path,
                    crate::model::workspace::OpenMode::Permanent,
                    Some(drop),
                );
            }
        }
        return false;
    }
    finish_file_drag_with(app, read_text_file)
}

fn finish_file_drag_with(
    app: &mut AppState,
    read: impl FnOnce(&Path) -> Result<String, String>,
) -> bool {
    let Some(drag) = app.editor.file_drag.take() else {
        return false;
    };
    let crate::state::FileDragSource::Explorer { clicks } = drag.source else {
        return false;
    };
    if !drag.active {
        return open_tree_file_with(app, drag.path, clicks, read);
    }
    let Some(drop) = drag.drop else {
        return false;
    };
    let contents = match read(&drag.path) {
        Ok(contents) => contents,
        Err(message) => {
            app.show_error(message);
            return false;
        }
    };
    let workspace = &mut app.workspace;
    match drop {
        TabDrop::Insert { pane, index } => {
            workspace.activate_pane(pane);
            let id = workspace.open_path(drag.path, contents);
            // `index` is an insertion point; `move_tab` takes a final index.
            if let Some(position) = workspace
                .pane(pane)
                .and_then(|pane| pane.items().iter().position(|item| *item == id))
            {
                let target = if position < index { index - 1 } else { index };
                workspace.move_tab(pane, id, target);
            }
        }
        TabDrop::Pane { pane, split: None } => {
            workspace.activate_pane(pane);
            workspace.open_path(drag.path, contents);
        }
        TabDrop::Pane {
            pane,
            split: Some(direction),
        } => {
            let Some(new) = workspace.split_empty(pane, direction) else {
                return false;
            };
            workspace.activate_pane(new);
            workspace.open_path(drag.path, contents);
        }
    }
    app.toast = None;
    true
}

/// Dispatch a drawer-file release without doing Git reads on the UI thread.
pub(crate) fn finish_drawer_file_drag(state: &State<AppState>) -> bool {
    let mut changes = None;
    let mut focus = false;
    state.try_update(|app| {
        let Some(drag) = &app.editor.file_drag else {
            return false;
        };
        if matches!(drag.source, crate::state::FileDragSource::Changes { .. }) {
            changes = app.editor.file_drag.take();
        } else {
            focus = finish_file_drag(app);
        }
        true
    });
    if let Some(drag) = changes {
        let crate::state::FileDragSource::Changes { request, untracked } = drag.source else {
            unreachable!("only changes drags are dispatched here");
        };
        if !drag.active {
            crate::git_actions::open_changes(state, request);
            return true;
        }
        if let Some(drop) = drag.drop {
            return crate::git_actions::open_diff_at(state, request, untracked, drop);
        }
    }
    focus
}

/// Expand or collapse an Explorer directory row, reading it on first expansion.
pub(crate) fn toggle_directory(app: &mut AppState, key: &str) {
    app.explorer
        .model
        .toggle_directory(&ExplorerFilesystem, key);
}

/// Reread every loaded Explorer listing (deferred while an edit row is open).
pub(crate) fn refresh(app: &mut AppState) -> bool {
    app.explorer.model.refresh_all(&ExplorerFilesystem)
}

pub fn reveal_file(state: &State<AppState>, id: FileId) {
    state.update(move |app| {
        let Some(path) = app.workspace.meta(id).map(|meta| meta.path.clone()) else {
            return;
        };
        let Reveal::Revealed { row } = app.explorer.model.reveal(&ExplorerFilesystem, &path) else {
            app.show_toast("File is outside the workspace.");
            return;
        };
        app.workspace.set_active(id);
        app.drawer.visible = true;
        if let Some(index) = row {
            app.explorer.ui.scroll = index.saturating_sub(1) as f32 * TREE_ROW_H;
        }
        app.explorer.ui.scroll_x = 0.0;
    });
}

pub fn choose_folder(state: &State<AppState>) -> bool {
    let mut options = FileDialogOptions::new().title("Open Folder");
    if let Some(directory) = state.get().explorer.model.roots().first().cloned() {
        options = options.directory(directory);
    }
    let Some(path) = system_file_dialogs().pick_folder(&options) else {
        return false;
    };

    state.update(move |app| load_folder(app, path));
    true
}

/// Add another root folder to the current file tree without replacing the
/// folders that are already open.
pub fn choose_folder_to_add(state: &State<AppState>) -> bool {
    let mut options = FileDialogOptions::new().title("Add Folder to Project");
    if let Some(directory) = state.get().explorer.model.roots().first().cloned() {
        options = options.directory(directory);
    }
    let Some(path) = system_file_dialogs().pick_folder(&options) else {
        return false;
    };

    state.update(move |app| add_folder(app, path));
    true
}

pub fn begin_create(state: &State<AppState>, parent: PathBuf, kind: ExplorerCreateKind) {
    state.update(move |app| {
        app.explorer.model.begin_create(parent, kind);
        app.explorer.ui.clear_edit();
    });
}

pub fn cancel_edit(state: &State<AppState>) {
    state.update(|app| {
        app.explorer.model.cancel_edit();
        app.explorer.ui.clear_edit();
    });
}

pub fn copy_item(state: &State<AppState>, path: &Path, cut: bool) {
    let action = if cut { "Cut" } else { "Copied" };
    match ExplorerFilesystem.set_file_clipboard(path, cut) {
        Ok(()) => {
            let name = path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            state.update(move |app| app.show_success(format!("{action} {name}.")));
        }
        Err(error) => state.update(move |app| app.show_error(error)),
    }
}

pub fn begin_rename(state: &State<AppState>, path: PathBuf, kind: ExplorerTargetKind) {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        state.update(|app| app.show_toast("This name cannot be edited as text."));
        return;
    };
    let name = name.to_owned();
    state.update(move |app| {
        app.explorer.model.begin_rename(path, kind);
        app.explorer.ui.create_input.set_text(name);
        app.explorer.ui.create_error = None;
    });
}

pub fn finish_rename(state: &State<AppState>) {
    let snapshot = state.get();
    let Some(request) = snapshot.explorer.model.rename_request().cloned() else {
        return;
    };
    let raw_name = snapshot.explorer.ui.create_input.text().to_owned();
    drop(snapshot);

    let target = match edit::validate_rename_path(&ExplorerFilesystem, &request.path, &raw_name) {
        Ok(target) => target,
        Err(message) => {
            state.update(move |app| {
                app.explorer.ui.create_error = Some(message.clone());
                app.show_error(message);
            });
            return;
        }
    };
    if target == request.path {
        cancel_edit(state);
        return;
    }
    if let Err(error) = ExplorerFilesystem.rename(&request.path, &target) {
        let message = format!("Could not rename {}: {error}", request.path.display());
        state.update(move |app| {
            app.explorer.ui.create_error = Some(message.clone());
            app.show_error(message);
        });
        return;
    }

    state.update(move |app| apply_renamed_path_state(app, &request.path, &target));
}

fn apply_renamed_path_state(app: &mut AppState, old_path: &Path, new_path: &Path) {
    app.workspace.move_open_paths_under(old_path, new_path);
    let roots_changed = app
        .explorer
        .model
        .apply_rename(&ExplorerFilesystem, old_path, new_path);
    app.explorer.ui.hovered_path = None;
    app.explorer.ui.clear_edit();
    app.toast = None;
    if roots_changed {
        persist_workspace(app);
    }
}

pub fn delete_item(state: &State<AppState>, path: PathBuf) {
    if state.get().workspace.has_dirty_paths_under(&path) {
        state.update(|app| {
            app.show_toast("Save or close modified files before deleting this item.")
        });
        return;
    }
    match move_path_to_trash(&path) {
        Ok(false) => {}
        Ok(true) => state.update(move |app| apply_deleted_path_state(app, &path)),
        Err(error) => state.update(move |app| app.show_error(error)),
    }
}

/// Git worktree removal shares the Explorer's platform trash operation.
pub(crate) fn move_path_to_trash(path: &Path) -> Result<bool, String> {
    ExplorerFilesystem.move_to_trash(path)
}

/// Show a file or folder in the platform file manager.
pub(crate) fn reveal_item(state: &State<AppState>, target: &ExplorerContextTarget) {
    if let Err(error) = ExplorerFilesystem.reveal(&target.path, target.kind) {
        state.update(move |app| {
            app.show_error(format!("Could not reveal item: {error}"));
        });
    }
}

pub(crate) fn apply_deleted_path_state(app: &mut AppState, path: &Path) {
    app.workspace.close_paths_under(path);
    let folders_changed = app.explorer.model.apply_delete(&ExplorerFilesystem, path);
    app.explorer.ui.hovered_path = None;
    app.explorer.ui.clear_edit();
    app.show_success("Moved item to Trash.");
    if folders_changed {
        persist_workspace(app);
    }
}

/// Remove a root from the current workspace without touching it on disk.
pub fn remove_root(state: &State<AppState>, root: PathBuf) {
    let display = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());
    state.update(move |app| {
        if remove_folder_state(app, &root) {
            persist_workspace(app);
            app.show_success(format!("Removed {display} from workspace."));
        }
    });
}

fn remove_folder_state(app: &mut AppState, root: &Path) -> bool {
    let Some(edit_cancelled) = app.explorer.model.remove_root(root) else {
        return false;
    };
    if app
        .explorer
        .ui
        .hovered_path
        .as_deref()
        .is_some_and(|path| Path::new(path).starts_with(root))
    {
        app.explorer.ui.hovered_path = None;
    }
    if edit_cancelled {
        app.explorer.ui.clear_edit();
    }
    app.explorer.ui.reset_scroll();
    true
}

pub fn finish_create(state: &State<AppState>) {
    let snapshot = state.get();
    let Some(request) = snapshot.explorer.model.create_request().cloned() else {
        return;
    };
    let raw_name = snapshot.explorer.ui.create_input.text().to_owned();
    drop(snapshot);

    let force_folder = raw_name.ends_with(['/', '\\']);
    let relative = match edit::validate_create_path(&ExplorerFilesystem, &request.parent, &raw_name)
    {
        Ok(relative) => relative,
        Err(message) => {
            state.update(move |app| {
                app.explorer.ui.create_error = Some(message.clone());
                app.show_error(message);
            });
            return;
        }
    };
    let target = request.parent.join(relative);
    let kind = if force_folder {
        ExplorerCreateKind::Folder
    } else {
        request.kind
    };
    let result = match kind {
        ExplorerCreateKind::File => ExplorerFilesystem.create_file(&target),
        ExplorerCreateKind::Folder => ExplorerFilesystem.create_folder(&target),
    };
    if let Err(error) = result {
        let message = format!("Could not create {}: {error}", target.display());
        state.update(move |app| {
            app.explorer.ui.create_error = Some(message.clone());
            app.show_error(message);
        });
        return;
    }

    state.update(move |app| {
        app.explorer
            .model
            .complete_create(&ExplorerFilesystem, &request.parent, &target, kind);
        if kind == ExplorerCreateKind::File {
            app.workspace.open_path(target, String::new());
        }
        app.explorer.ui.clear_edit();
        app.toast = None;
    });
}

/// Reopen a folder selected from the welcome screen's recent-workspace list.
pub fn open_recent_folder(state: &State<AppState>, path: PathBuf) -> bool {
    if !path.is_dir() {
        let display = path.display().to_string();
        state.update(move |app| {
            app.explorer.model.forget_recent(&path);
            persist_workspace(app);
            app.show_error(format!(
                "Workspace folder is no longer available: {display}"
            ));
        });
        return false;
    }

    state.update(move |app| load_folder(app, path));
    true
}

pub(crate) fn load_folder(app: &mut AppState, path: PathBuf) {
    let path = normalized_folder(path);
    replace_folder_state(app, path);
    persist_workspace(app);
}

fn replace_folder_state(app: &mut AppState, path: PathBuf) {
    app.explorer.model.open_root(&ExplorerFilesystem, path);
    app.explorer.ui.reset_scroll();
    app.drawer.visible = true;
    app.welcome_hover = None;
    app.toast = None;
}

pub(crate) fn add_folder(app: &mut AppState, path: PathBuf) {
    let path = normalized_folder(path);
    add_folder_state(app, path);
    persist_workspace(app);
}

/// Forget a worktree after Git successfully removed it from disk.
pub(crate) fn forget_worktree(app: &mut AppState, root: &Path) {
    remove_folder_state(app, root);
    app.explorer.model.forget_recent(root);
    if app
        .git_pinned_repository
        .as_ref()
        .is_some_and(|(path, _)| path == root)
    {
        app.git_pinned_repository = None;
    }
    persist_workspace(app);
}

fn add_folder_state(app: &mut AppState, path: PathBuf) -> bool {
    let was_empty = app.explorer.model.roots().is_empty();
    if !app.explorer.model.add_root(&ExplorerFilesystem, path) {
        return false;
    }
    if was_empty {
        app.explorer.ui.reset_scroll();
    }
    app.drawer.visible = true;
    app.welcome_hover = None;
    app.toast = None;
    true
}

/// Open paths given on the command line, after the previous session has been
/// restored. The first folder replaces the workspace like Open Folder and any
/// further folders are added to it; files open as tabs, the last one active.
/// Unreadable paths are reported once in a toast.
pub(crate) fn open_launch_paths(app: &mut AppState, paths: Vec<PathBuf>) {
    // Launch files use the same metadata-first restoration as session tabs.
    // Reading them here would still block the very first frame.
    let mut replaced_workspace = false;
    for path in paths {
        let path = std::path::absolute(&path).unwrap_or(path);
        if path.is_dir() {
            let path = normalized_folder(path);
            if replaced_workspace {
                add_folder_state(app, path);
            } else {
                replace_folder_state(app, path);
                replaced_workspace = true;
            }
        } else {
            if app
                .runtime()
                .is_some_and(|runtime| runtime.file_loader().is_some())
            {
                if let Some(id) = app.workspace.file_id_for_path(&path) {
                    app.workspace.set_active(id);
                } else {
                    crate::file_loading::request(
                        app,
                        path,
                        crate::model::workspace::OpenMode::Permanent,
                        None,
                    );
                }
            } else {
                app.workspace.open_deferred(path);
            }
        }
    }
    if replaced_workspace {
        persist_workspace(app);
    }
}

/// Apply launch paths to the state; returns whether the workspace folders
/// changed and need to be persisted.
#[cfg(test)]
fn open_launch_paths_with(
    app: &mut AppState,
    paths: Vec<PathBuf>,
    mut read: impl FnMut(&Path) -> Result<String, String>,
) -> bool {
    let mut replaced_workspace = false;
    let mut errors = Vec::new();
    for path in paths {
        // Relative to the launching shell's directory. `absolute` keeps the
        // plain spelling (no `\\?\` prefix), so session tabs still match.
        let path = std::path::absolute(&path).unwrap_or(path);
        if path.is_dir() {
            let path = normalized_folder(path);
            if replaced_workspace {
                add_folder_state(app, path);
            } else {
                replace_folder_state(app, path);
                replaced_workspace = true;
            }
            continue;
        }
        match read(&path) {
            Ok(contents) => {
                app.workspace.open_path(path, contents);
            }
            Err(message) => errors.push(message),
        }
    }
    if let Some(first) = errors.first() {
        let message = match errors.len() {
            1 => first.clone(),
            count => format!("{first} (and {} more)", count - 1),
        };
        app.show_error(message);
    }
    replaced_workspace
}

/// Populate directory caches for folders restored from the previous session.
pub(crate) fn hydrate_workspace_folders(app: &mut AppState) {
    app.explorer.model.hydrate(&ExplorerFilesystem);
}

fn normalized_folder(path: PathBuf) -> PathBuf {
    roots::normalized(&ExplorerFilesystem, path)
}

fn persist_workspace(app: &mut AppState) {
    if let Err(error) =
        workspace_persistence::save(app.explorer.model.roots(), app.explorer.model.recents())
    {
        app.show_error(format!("Could not remember workspace folders: {error}"));
    }
}

#[cfg(test)]
#[path = "explorer_tests.rs"]
mod tests;

/// Copy `target` to the clipboard, absolute or relative to its workspace root.
pub(crate) fn copy_path(
    state: &State<AppState>,
    clipboard: &dyn lgui::services::Clipboard,
    target: &Path,
    relative: bool,
) {
    let value = if relative {
        let snapshot = state.get();
        let Some(root) = snapshot.explorer.model.root_for_path(target) else {
            state.update(|app| app.show_toast("Path is outside the workspace."));
            return;
        };
        let relative = target.strip_prefix(root).unwrap_or(target);
        if relative.as_os_str().is_empty() {
            ".".to_owned()
        } else {
            crate::ui::display_path(relative)
        }
    } else {
        crate::ui::display_path(target)
    };
    let label = if relative { "relative path" } else { "path" };
    match clipboard.write_text(&value) {
        Ok(()) => state.update(move |app| app.show_success(format!("Copied {label}."))),
        Err(error) => state.update(move |app| app.show_error(format!("Clipboard: {error}"))),
    }
}
