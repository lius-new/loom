//! Shared workspace commands used by the welcome screen and editor chrome.

use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::thread;

use lgui::dialogs::{FileDialogOptions, system_file_dialogs};
use lgui::prelude::State;

use crate::file_tree::read_directory;
use crate::model::document::FileId;
use crate::model::pane_layout::{Axis, Child, PaneId, PaneNode};
use crate::model::workspace::Workspace;
use crate::state::{
    AppState, CloseContinuation, CloseRequest, ExplorerCreateKind, ExplorerCreateRequest,
    ExplorerRenameRequest, ExplorerTargetKind, TabDrop,
};
use crate::workspace_persistence::{self, MAX_RECENT_FOLDERS};
use crate::workspace_persistence::{EditorLayout, SavedAxis, SavedChild, SavedPaneNode};

const MAX_EDITABLE_FILE_MIB: u64 = 64;
const MAX_EDITABLE_FILE_BYTES: u64 = MAX_EDITABLE_FILE_MIB * 1024 * 1024;
const TREE_ROW_H: f32 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabCloseScope {
    Tab,
    Others,
    Right,
    Saved,
    All,
}

pub fn save_active_document(state: &State<AppState>) -> bool {
    let Some(id) = state.get().workspace.active() else {
        return false;
    };
    save_document(state, id)
}

pub fn save_document(state: &State<AppState>, id: FileId) -> bool {
    let Some((id, path, contents)) = state.get().workspace.save_snapshot(id) else {
        return false;
    };

    match fs::write(&path, contents.as_bytes()) {
        Ok(()) => {
            let project_settings = crate::project_settings::is_settings_file(&path);
            state.update(move |app| {
                app.workspace.mark_saved(id);
                if project_settings {
                    app.refresh_project_settings();
                }
            });
            true
        }
        Err(error) => {
            state.update(move |app| {
                app.show_error(format!("Could not save {}: {error}", path.display()));
            });
            false
        }
    }
}

pub fn request_close_tab(state: &State<AppState>, pane: PaneId, id: FileId) {
    state.update(move |app| request_close_tab_in(app, pane, id));
}

pub(crate) fn request_close_tab_in(app: &mut AppState, pane: PaneId, id: FileId) {
    request_close_tabs_in(app, vec![(pane, id)]);
}

/// Close every tab of a pane (asking about dirty files), which then closes
/// the pane itself; an already empty pane closes directly.
pub(crate) fn request_close_pane_in(app: &mut AppState, pane: PaneId) {
    let Some(items) = app.workspace.pane(pane).map(|p| p.items().to_vec()) else {
        return;
    };
    if items.is_empty() {
        app.workspace.close_pane(pane);
    } else {
        request_close_tabs_in(app, items.into_iter().map(|id| (pane, id)).collect());
    }
}

pub fn tab_close_targets(
    app: &AppState,
    pane: PaneId,
    target: FileId,
    scope: TabCloseScope,
) -> Vec<(PaneId, FileId)> {
    let Some(open) = app.workspace.pane(pane).map(|pane| pane.items()) else {
        return Vec::new();
    };
    let Some(target_index) = open.iter().position(|id| *id == target) else {
        return Vec::new();
    };
    let ids: Vec<FileId> = match scope {
        TabCloseScope::Tab => vec![target],
        TabCloseScope::Others => open.iter().copied().filter(|id| *id != target).collect(),
        TabCloseScope::Right => open.iter().copied().skip(target_index + 1).collect(),
        TabCloseScope::Saved => open
            .iter()
            .copied()
            .filter(|id| !app.workspace.is_dirty(*id))
            .collect(),
        TabCloseScope::All => open.to_vec(),
    };
    ids.into_iter().map(|id| (pane, id)).collect()
}

pub fn request_close_tabs(
    state: &State<AppState>,
    pane: PaneId,
    target: FileId,
    scope: TabCloseScope,
) {
    let targets = tab_close_targets(&state.get(), pane, target, scope);
    state.update(move |app| request_close_tabs_in(app, targets));
}

fn request_close_tabs_in(app: &mut AppState, targets: Vec<(PaneId, FileId)>) {
    let mut existing = Vec::with_capacity(targets.len());
    for (pane, id) in targets {
        if app.workspace.pane(pane).is_some_and(|p| p.contains(id))
            && !existing.contains(&(pane, id))
        {
            existing.push((pane, id));
        }
    }
    if existing.is_empty() {
        return;
    }
    app.editor.tab_context_menu = None;
    app.editor.tab_drag = None;
    app.editor.menu = None;
    // A dirty document only needs a decision when its last tab closes.
    if app
        .workspace
        .released_by(&existing)
        .into_iter()
        .any(|id| app.workspace.is_dirty(id))
    {
        app.editor.close_request = Some(CloseRequest {
            targets: existing,
            continuation: CloseContinuation::CloseTabs,
        });
    } else {
        for (pane, id) in existing {
            app.workspace.close_item(pane, id);
        }
    }
}

pub fn cancel_close_request(state: &State<AppState>) {
    state.update(cancel_close_request_in);
}

fn cancel_close_request_in(app: &mut AppState) {
    app.editor.close_request = None;
}

pub fn discard_close_request(state: &State<AppState>) -> Option<CloseContinuation> {
    let continuation = state.get().editor.close_request.as_ref()?.continuation;
    state.update(|app| {
        let _ = discard_close_request_in(app);
    });
    Some(continuation)
}

fn discard_close_request_in(app: &mut AppState) -> Option<CloseContinuation> {
    let request = app.editor.close_request.take()?;
    if request.continuation == CloseContinuation::CloseTabs {
        for (pane, id) in request.targets {
            app.workspace.close_item(pane, id);
        }
    }
    Some(request.continuation)
}

pub fn save_close_request(state: &State<AppState>) -> Option<CloseContinuation> {
    let mut next = state.get();
    match save_close_request_in(&mut next) {
        Ok(continuation) => {
            state.set(next);
            continuation
        }
        Err(error) => {
            next.show_error(error);
            state.set(next);
            None
        }
    }
}

fn save_close_request_in(app: &mut AppState) -> Result<Option<CloseContinuation>, String> {
    let Some(request) = app.editor.close_request.clone() else {
        return Ok(None);
    };
    for id in app.workspace.released_by(&request.targets) {
        if !app.workspace.is_dirty(id) {
            continue;
        }
        let Some((id, path, contents)) = app.workspace.save_snapshot(id) else {
            continue;
        };
        fs::write(&path, contents.as_bytes())
            .map_err(|error| format!("Could not save {}: {error}", path.display()))?;
        app.workspace.mark_saved(id);
    }
    if request.continuation == CloseContinuation::CloseTabs {
        for (pane, id) in request.targets {
            app.workspace.close_item(pane, id);
        }
    }
    app.editor.close_request = None;
    Ok(Some(request.continuation))
}

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

pub fn reveal_file_in_tree(state: &State<AppState>, id: FileId) {
    state.update(move |app| {
        let Some(path) = app.workspace.meta(id).map(|meta| meta.path.clone()) else {
            return;
        };
        let Some(root) = app
            .workspace_folders
            .iter()
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.components().count())
            .cloned()
        else {
            app.show_toast("File is outside the workspace.");
            return;
        };

        app.workspace.set_active(id);
        app.show_drawer = true;
        let mut directories = Vec::new();
        let mut current = path.parent();
        while let Some(directory) = current {
            if !directory.starts_with(&root) {
                break;
            }
            directories.push(directory.to_path_buf());
            if directory == root {
                break;
            }
            current = directory.parent();
        }
        directories.reverse();
        for directory in directories {
            let key = directory.to_string_lossy().into_owned();
            app.dir_entries
                .insert(key.clone(), read_directory(&directory));
            app.expanded.insert(key);
        }
        if let Some(index) = visible_tree_row_index(app, &path) {
            app.tree_scroll = index.saturating_sub(1) as f32 * TREE_ROW_H;
        }
        app.tree_scroll_x = 0.0;
    });
}

fn visible_tree_row_index(app: &AppState, target: &Path) -> Option<usize> {
    let mut index = 0usize;
    for root in &app.workspace_folders {
        if root == target {
            return Some(index);
        }
        index += 1;
        let key = root.to_string_lossy();
        if app.expanded.contains(key.as_ref())
            && let Some(found) = visible_descendant_index(app, key.as_ref(), target, &mut index)
        {
            return Some(found);
        }
    }
    None
}

fn visible_descendant_index(
    app: &AppState,
    directory_key: &str,
    target: &Path,
    index: &mut usize,
) -> Option<usize> {
    let entries = app.dir_entries.get(directory_key)?;
    for entry in entries.iter() {
        if entry.path == target {
            return Some(*index);
        }
        *index += 1;
        let key = entry.path.to_string_lossy();
        if entry.is_dir
            && app.expanded.contains(key.as_ref())
            && let Some(found) = visible_descendant_index(app, key.as_ref(), target, index)
        {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
fn dirty_test_file(app: &mut AppState, path: PathBuf, original: &str, change: &str) -> FileId {
    fs::write(&path, original).unwrap();
    let id = app.workspace.open_path(path, original.to_owned());
    app.workspace
        .active_editor_mut()
        .unwrap()
        .select_to(usize::MAX, false);
    app.workspace
        .active_editor_mut()
        .unwrap()
        .replace_selection(change, crate::model::text::EditKind::Typing)
        .unwrap();
    id
}

#[cfg(test)]
fn close_test_directory(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "loom-close-{name}-{}-{}",
        std::process::id(),
        crate::model::document::content_hash(name)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

pub fn choose_file(state: &State<AppState>) -> bool {
    let mut options = FileDialogOptions::new().title("Open File");
    if let Some(directory) = state.get().workspace_folders.first().cloned() {
        options = options.directory(directory);
    }
    let Some(path) = system_file_dialogs().pick_file(&options) else {
        return false;
    };

    let mut opened = false;
    state.update(|app| {
        opened = open_tree_file(app, path, 2);
        app.welcome_hover = None;
        app.workspace_home_hover = None;
    });
    opened
}

pub fn choose_folder(state: &State<AppState>) -> bool {
    let mut options = FileDialogOptions::new().title("Open Folder");
    if let Some(directory) = state.get().workspace_folders.first().cloned() {
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
    if let Some(directory) = state.get().workspace_folders.first().cloned() {
        options = options.directory(directory);
    }
    let Some(path) = system_file_dialogs().pick_folder(&options) else {
        return false;
    };

    state.update(move |app| add_folder(app, path));
    true
}

pub fn begin_explorer_create(state: &State<AppState>, parent: PathBuf, kind: ExplorerCreateKind) {
    state.update(move |app| {
        let key = parent.to_string_lossy().into_owned();
        app.expanded.insert(key);
        app.explorer_rename = None;
        app.explorer_create = Some(ExplorerCreateRequest { kind, parent });
        app.explorer_create_input.clear();
        app.explorer_create_error = None;
    });
}

pub fn cancel_explorer_create(state: &State<AppState>) {
    state.update(|app| {
        app.explorer_create = None;
        app.explorer_rename = None;
        app.explorer_create_input.clear();
        app.explorer_create_error = None;
    });
}

pub fn copy_explorer_item(state: &State<AppState>, path: &Path, cut: bool) {
    let action = if cut { "Cut" } else { "Copied" };
    match set_file_clipboard(path, cut) {
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

#[cfg(target_os = "windows")]
fn set_file_clipboard(path: &Path, cut: bool) -> Result<(), String> {
    let path = windows_shell_path(path);
    let _clipboard = clipboard_win::Clipboard::new_attempts(10)
        .map_err(|error| format!("Could not open the clipboard: {error}"))?;
    clipboard_win::empty().map_err(|error| format!("Could not clear the clipboard: {error}"))?;
    clipboard_win::raw::set_file_list(&[path.as_str()])
        .map_err(|error| format!("Could not copy the folder: {error}"))?;

    let format = clipboard_win::register_format("Preferred DropEffect")
        .ok_or_else(|| "Could not register the file clipboard format.".to_owned())?;
    let effect = if cut { 2_u32 } else { 1_u32 };
    clipboard_win::raw::set_without_clear(format.get(), &effect.to_le_bytes())
        .map_err(|error| format!("Could not set the clipboard operation: {error}"))
}

#[cfg(not(target_os = "windows"))]
fn set_file_clipboard(_path: &Path, _cut: bool) -> Result<(), String> {
    Err("Copying file-system items is not supported on this platform yet.".to_owned())
}

pub fn begin_explorer_rename(state: &State<AppState>, path: PathBuf, kind: ExplorerTargetKind) {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        state.update(|app| app.show_toast("This name cannot be edited as text."));
        return;
    };
    let name = name.to_owned();
    state.update(move |app| {
        app.explorer_create = None;
        app.explorer_rename = Some(ExplorerRenameRequest { path, kind });
        app.explorer_create_input.set_text(name);
        app.explorer_create_error = None;
    });
}

pub fn finish_explorer_rename(state: &State<AppState>) {
    let snapshot = state.get();
    let Some(request) = snapshot.explorer_rename.clone() else {
        return;
    };
    let raw_name = snapshot.explorer_create_input.text().to_owned();
    drop(snapshot);

    let target = match validate_rename_path(&request.path, &raw_name) {
        Ok(target) => target,
        Err(message) => {
            state.update(move |app| {
                app.explorer_create_error = Some(message.clone());
                app.show_error(message);
            });
            return;
        }
    };
    if target == request.path {
        cancel_explorer_create(state);
        return;
    }
    if let Err(error) = fs::rename(&request.path, &target) {
        let message = format!("Could not rename {}: {error}", request.path.display());
        state.update(move |app| {
            app.explorer_create_error = Some(message.clone());
            app.show_error(message);
        });
        return;
    }

    state.update(move |app| apply_renamed_path_state(app, &request.path, &target));
}

fn validate_rename_path(path: &Path, raw_name: &str) -> Result<PathBuf, String> {
    let name = raw_name.trim_matches('\t');
    if name.is_empty() || name.chars().all(char::is_whitespace) {
        return Err("A name must be provided.".to_owned());
    }
    if name.contains(['/', '\\']) || matches!(name, "." | "..") || invalid_file_name_segment(name) {
        return Err(format!("The name '{name}' is not valid."));
    }
    let parent = path
        .parent()
        .ok_or_else(|| "This item cannot be renamed.".to_owned())?;
    let target = parent.join(name);
    if target == path {
        return Ok(target);
    }
    if target.exists() {
        return Err(format!("{} already exists.", target.display()));
    }
    Ok(target)
}

fn apply_renamed_path_state(app: &mut AppState, old_path: &Path, new_path: &Path) {
    let was_expanded = app.expanded.iter().any(|path| Path::new(path) == old_path);
    app.workspace.move_open_paths_under(old_path, new_path);

    let mut roots_changed = false;
    for root in &mut app.workspace_folders {
        if let Ok(relative) = root.strip_prefix(old_path) {
            *root = new_path.join(relative);
            roots_changed = true;
        }
    }
    for recent in &mut app.recent_folders {
        if let Ok(relative) = recent.strip_prefix(old_path) {
            *recent = new_path.join(relative);
        }
    }
    app.dir_entries
        .retain(|path, _| !Path::new(path).starts_with(old_path));
    app.expanded
        .retain(|path| !Path::new(path).starts_with(old_path));
    if let Some(parent) = new_path.parent()
        && app
            .workspace_folders
            .iter()
            .any(|root| parent.starts_with(root))
    {
        app.dir_entries.insert(
            parent.to_string_lossy().into_owned(),
            read_directory(parent),
        );
    }
    if was_expanded || app.workspace_folders.iter().any(|root| root == new_path) {
        let key = new_path.to_string_lossy().into_owned();
        app.dir_entries
            .insert(key.clone(), read_directory(new_path));
        app.expanded.insert(key);
    }
    app.tree_hovered_path = None;
    app.explorer_rename = None;
    app.explorer_create_input.clear();
    app.explorer_create_error = None;
    app.toast = None;
    if roots_changed {
        persist_workspace(app);
    }
}

pub fn delete_explorer_item(state: &State<AppState>, path: PathBuf) {
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

#[cfg(target_os = "windows")]
pub(crate) fn move_path_to_trash(path: &Path) -> Result<bool, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_WANTNUKEWARNING, SHFILEOPSTRUCTW,
        SHFileOperationW,
    };

    let path = windows_shell_path(path);
    let mut from = std::ffi::OsStr::new(&path)
        .encode_wide()
        .collect::<Vec<_>>();
    from.extend([0, 0]);
    let mut operation = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_WANTNUKEWARNING) as u16,
        ..Default::default()
    };
    // FOF_ALLOWUNDO asks the shell to use the Recycle Bin; WANTNUKEWARNING
    // preserves a shell warning if recycling is unavailable.
    let result = unsafe { SHFileOperationW(&mut operation) };
    if result != 0 {
        return Err(format!(
            "Could not move the item to the Recycle Bin (shell error {result})."
        ));
    }
    Ok(operation.fAnyOperationsAborted == 0)
}

#[cfg(target_os = "macos")]
pub(crate) fn move_path_to_trash(path: &Path) -> Result<bool, String> {
    let status = std::process::Command::new("osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "tell application \"Finder\" to delete POSIX file (item 1 of argv)",
            "-e",
            "end run",
            "--",
        ])
        .arg(path)
        .status()
        .map_err(|error| format!("Could not open Trash: {error}"))?;
    if status.success() {
        Ok(true)
    } else {
        Err("Could not move the item to Trash.".to_owned())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn move_path_to_trash(path: &Path) -> Result<bool, String> {
    let status = std::process::Command::new("gio")
        .args(["trash", "--"])
        .arg(path)
        .status()
        .map_err(|error| format!("Could not open Trash: {error}"))?;
    if status.success() {
        Ok(true)
    } else {
        Err("Could not move the item to Trash.".to_owned())
    }
}

#[cfg(target_os = "windows")]
fn windows_shell_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(local) = value.strip_prefix(r"\\?\") {
        local.to_owned()
    } else {
        value.into_owned()
    }
}

pub(crate) fn apply_deleted_path_state(app: &mut AppState, path: &Path) {
    app.workspace.close_paths_under(path);
    let original_root_count = app.workspace_folders.len();
    let original_recent_count = app.recent_folders.len();
    app.workspace_folders.retain(|root| !root.starts_with(path));
    app.recent_folders
        .retain(|recent| !recent.starts_with(path));
    app.dir_entries
        .retain(|cached, _| !Path::new(cached).starts_with(path));
    app.expanded
        .retain(|expanded| !Path::new(expanded).starts_with(path));
    if let Some(parent) = path.parent()
        && app
            .workspace_folders
            .iter()
            .any(|root| parent.starts_with(root))
    {
        app.dir_entries.insert(
            parent.to_string_lossy().into_owned(),
            read_directory(parent),
        );
    }
    app.tree_hovered_path = None;
    app.explorer_create = None;
    app.explorer_rename = None;
    app.explorer_create_input.clear();
    app.explorer_create_error = None;
    app.show_success("Moved item to Trash.");
    if app.workspace_folders.len() != original_root_count
        || app.recent_folders.len() != original_recent_count
    {
        persist_workspace(app);
    }
}

/// Remove a root from the current workspace without touching it on disk.
pub fn remove_folder_from_workspace(state: &State<AppState>, root: PathBuf) {
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
    let original_len = app.workspace_folders.len();
    app.workspace_folders.retain(|folder| folder != root);
    if app.workspace_folders.len() == original_len {
        return false;
    }

    app.dir_entries
        .retain(|path, _| !Path::new(path).starts_with(root));
    app.expanded
        .retain(|path| !Path::new(path).starts_with(root));
    if app
        .tree_hovered_path
        .as_deref()
        .is_some_and(|path| Path::new(path).starts_with(root))
    {
        app.tree_hovered_path = None;
    }
    if app
        .explorer_create
        .as_ref()
        .is_some_and(|request| request.parent.starts_with(root))
    {
        app.explorer_create = None;
        app.explorer_create_input.clear();
        app.explorer_create_error = None;
    }
    if app
        .explorer_rename
        .as_ref()
        .is_some_and(|request| request.path.starts_with(root))
    {
        app.explorer_rename = None;
        app.explorer_create_input.clear();
        app.explorer_create_error = None;
    }
    app.tree_scroll = 0.0;
    app.tree_scroll_x = 0.0;
    true
}

pub fn finish_explorer_create(state: &State<AppState>) {
    let snapshot = state.get();
    let Some(request) = snapshot.explorer_create.clone() else {
        return;
    };
    let raw_name = snapshot.explorer_create_input.text().to_owned();
    drop(snapshot);

    let force_folder = raw_name.ends_with(['/', '\\']);
    let relative = match validate_create_path(&request.parent, &raw_name) {
        Ok(relative) => relative,
        Err(message) => {
            state.update(move |app| {
                app.explorer_create_error = Some(message.clone());
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
        ExplorerCreateKind::File => target
            .parent()
            .map(fs::create_dir_all)
            .transpose()
            .and_then(|_| {
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                    .map(|_| ())
            }),
        ExplorerCreateKind::Folder => fs::create_dir_all(&target),
    };
    if let Err(error) = result {
        let message = format!("Could not create {}: {error}", target.display());
        state.update(move |app| {
            app.explorer_create_error = Some(message.clone());
            app.show_error(message);
        });
        return;
    }

    state.update(move |app| {
        refresh_created_path(app, &request.parent, &target, kind);
        if kind == ExplorerCreateKind::File {
            app.workspace.open_path(target, String::new());
        }
        app.explorer_create = None;
        app.explorer_create_input.clear();
        app.explorer_create_error = None;
        app.toast = None;
    });
}

fn validate_create_path(parent: &Path, raw_name: &str) -> Result<PathBuf, String> {
    let name = raw_name.trim_matches('\t').trim_end_matches(['/', '\\']);
    if name.is_empty() || name.chars().all(char::is_whitespace) {
        return Err("A file or folder name must be provided.".to_owned());
    }
    if name.starts_with(['/', '\\']) || Path::new(name).is_absolute() {
        return Err("A file or folder name cannot start with a slash or drive prefix.".to_owned());
    }

    let mut relative = PathBuf::new();
    for segment in name
        .split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
    {
        if matches!(segment, "." | "..") {
            return Err("Relative path segments '.' and '..' are not allowed.".to_owned());
        }
        if segment.trim() != segment {
            return Err("Leading or trailing whitespace is not allowed in a name.".to_owned());
        }
        if invalid_file_name_segment(segment) {
            return Err(format!("The name '{segment}' is not valid."));
        }
        relative.push(segment);
    }
    if relative.as_os_str().is_empty() {
        return Err("A file or folder name must be provided.".to_owned());
    }
    let target = parent.join(&relative);
    if target.exists() {
        return Err(format!("{} already exists.", target.display()));
    }
    Ok(relative)
}

fn invalid_file_name_segment(segment: &str) -> bool {
    if segment.len() > 255 || segment.chars().any(|character| character == '\0') {
        return true;
    }
    #[cfg(target_os = "windows")]
    {
        if segment
            .chars()
            .any(|character| character.is_control() || r#"<>:"|?*"#.contains(character))
            || segment.ends_with(['.', ' '])
        {
            return true;
        }
        let device_name = segment
            .split('.')
            .next()
            .unwrap_or(segment)
            .to_ascii_uppercase();
        if matches!(
            device_name.as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        ) {
            return true;
        }
    }
    false
}

fn refresh_created_path(app: &mut AppState, root: &Path, target: &Path, kind: ExplorerCreateKind) {
    let refresh_until = if kind == ExplorerCreateKind::Folder {
        target
    } else {
        target.parent().unwrap_or(root)
    };
    let mut directory = root.to_path_buf();
    let key = directory.to_string_lossy().into_owned();
    app.dir_entries
        .insert(key.clone(), read_directory(&directory));
    app.expanded.insert(key);
    if let Ok(relative) = refresh_until.strip_prefix(root) {
        for component in relative.components() {
            directory.push(component.as_os_str());
            let key = directory.to_string_lossy().into_owned();
            app.dir_entries
                .insert(key.clone(), read_directory(&directory));
            app.expanded.insert(key);
        }
    }
}

/// Reopen a folder selected from the welcome screen's recent-workspace list.
pub fn open_recent_folder(state: &State<AppState>, path: PathBuf) -> bool {
    if !path.is_dir() {
        let display = path.display().to_string();
        state.update(move |app| {
            app.recent_folders.retain(|recent| recent != &path);
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

/// Ask for a destination and clone the URL currently entered in the clone
/// dialog. Git runs off the UI thread; a successful clone becomes the active
/// workspace automatically.
pub fn clone_repository(state: &State<AppState>) -> bool {
    let url = state.get().clone_repository_url.trim().to_owned();
    if url.is_empty() {
        state.update(|app| {
            app.clone_repository_error = Some("Enter a repository URL.".to_owned());
        });
        return false;
    }
    let Some(directory_name) = repository_directory_name(&url) else {
        state.update(|app| {
            app.clone_repository_error = Some("The repository URL has no project name.".to_owned());
        });
        return false;
    };

    let mut options = FileDialogOptions::new().title("Select Repository Location");
    if let Some(directory) = state.get().workspace_folders.first().cloned() {
        options = options.directory(directory);
    }
    let Some(parent) = system_file_dialogs().pick_folder(&options) else {
        return false;
    };
    let target = parent.join(directory_name);
    if target.exists() {
        let message = format!("{} already exists.", target.display());
        state.update(move |app| app.clone_repository_error = Some(message));
        return false;
    }

    state.update(|app| {
        app.cloning_repository = true;
        app.clone_repository_error = None;
    });
    let clone_state = state.clone();
    thread::Builder::new()
        .name("loom-git-clone".to_owned())
        .spawn(move || {
            let result = crate::git::service().and_then(|service| {
                service.clone_repository(
                    &url,
                    &target,
                    &crate::git::command::CancellationToken::default(),
                )
            });

            clone_state.update(move |app| {
                app.cloning_repository = false;
                match result {
                    Ok(()) => {
                        app.toast = None;
                        load_folder(app, target);
                        app.show_clone_dialog = false;
                        app.clone_repository_url.clear();
                        app.clone_repository_error = None;
                        app.clone_input_focused = false;
                    }
                    Err(error) => {
                        app.clone_repository_error = Some(error.user_message());
                    }
                }
            });
        })
        .map(|_| true)
        .unwrap_or_else(|error| {
            state.update(move |app| {
                app.cloning_repository = false;
                app.clone_repository_error = Some(format!("Could not start git: {error}"));
            });
            false
        })
}

fn repository_directory_name(url: &str) -> Option<String> {
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    let trimmed = without_query.trim_end_matches(['/', '\\']);
    let name = trimmed
        .rsplit(['/', '\\', ':'])
        .next()
        .unwrap_or(trimmed)
        .strip_suffix(".git")
        .unwrap_or_else(|| trimmed.rsplit(['/', '\\', ':']).next().unwrap_or(trimmed))
        .trim();
    if name.is_empty() || matches!(name, "." | "..") {
        None
    } else {
        Some(name.to_owned())
    }
}

fn first_error_line(stderr: &str) -> String {
    stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("Git clone failed.")
        .to_owned()
}

fn load_folder(app: &mut AppState, path: PathBuf) {
    let path = normalized_folder(path);
    replace_folder_state(app, path);
    persist_workspace(app);
}

fn replace_folder_state(app: &mut AppState, path: PathBuf) {
    let key = path.to_string_lossy().into_owned();
    let entries = read_directory(&path);
    app.workspace_folders.clear();
    app.workspace_folders.push(path.clone());
    app.dir_entries.clear();
    app.dir_entries.insert(key.clone(), entries);
    app.expanded.clear();
    app.expanded.insert(key);
    app.tree_scroll = 0.0;
    app.tree_scroll_x = 0.0;
    app.show_drawer = true;
    app.welcome_hover = None;
    app.toast = None;
    remember_folder(app, path);
}

pub(crate) fn add_folder(app: &mut AppState, path: PathBuf) {
    let path = normalized_folder(path);
    add_folder_state(app, path);
    persist_workspace(app);
}

/// Forget a worktree after Git successfully removed it from disk.
pub(crate) fn forget_worktree(app: &mut AppState, root: &Path) {
    remove_folder_state(app, root);
    app.recent_folders.retain(|path| path != root);
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
    if app.workspace_folders.contains(&path) {
        remember_folder(app, path);
        return false;
    }

    let key = path.to_string_lossy().into_owned();
    let entries = read_directory(&path);
    let was_empty = app.workspace_folders.is_empty();
    app.workspace_folders.push(path.clone());
    app.dir_entries.insert(key.clone(), entries);
    app.expanded.insert(key);
    if was_empty {
        app.tree_scroll = 0.0;
        app.tree_scroll_x = 0.0;
    }
    app.show_drawer = true;
    app.welcome_hover = None;
    app.toast = None;
    remember_folder(app, path);
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
    app.workspace_folders = existing_unique(std::mem::take(&mut app.workspace_folders));
    app.recent_folders = existing_unique(std::mem::take(&mut app.recent_folders));
    app.recent_folders.truncate(MAX_RECENT_FOLDERS);

    for path in &app.workspace_folders {
        let key = path.to_string_lossy().into_owned();
        app.dir_entries.insert(key.clone(), read_directory(path));
        app.expanded.insert(key);
    }

    include_workspace_folders_in_recents(app);
}

/// All open file tabs, including the single-click preview. Diffs and built-in
/// pages are filtered out; panes left empty by that filter are dropped.
pub(crate) fn saved_editor_layout(workspace: &Workspace) -> Option<EditorLayout> {
    saved_layout_with_pending(workspace, None)
}

pub(crate) fn saved_app_editor_layout(app: &AppState) -> Option<EditorLayout> {
    saved_layout_with_pending(&app.workspace, app.editor.pending_file.as_ref())
}

/// Take the current snapshot at the exit boundary, rather than waiting for a
/// later render effect. A failed write keeps the window open with its buffers.
pub(crate) fn save_editor_session(state: &State<AppState>) -> bool {
    let layout = saved_app_editor_layout(&state.get());
    let result = workspace_persistence::save_editor_layout(layout.as_ref());
    if let Err(error) = result {
        state.update(|app| app.show_error(format!("Could not remember file tabs: {error}")));
        return false;
    }
    true
}

fn saved_layout_with_pending(
    workspace: &Workspace,
    pending: Option<&crate::file_loading::PendingFile>,
) -> Option<EditorLayout> {
    let mut leaf_index = 0;
    let mut active_pane = None;
    let root = save_pane_node(
        workspace,
        workspace.layout(),
        pending,
        &mut leaf_index,
        &mut active_pane,
    )?;
    Some(EditorLayout {
        root,
        active_pane: active_pane.unwrap_or(0),
    })
}

fn save_pane_node(
    workspace: &Workspace,
    node: &PaneNode,
    pending: Option<&crate::file_loading::PendingFile>,
    leaf_index: &mut usize,
    active_pane: &mut Option<usize>,
) -> Option<SavedPaneNode> {
    match node {
        PaneNode::Leaf(id) => {
            let pane = workspace.pane(*id)?;
            let restorable = |file: FileId| workspace.is_file(file);
            let path = |file: FileId| workspace.meta(file).map(|meta| meta.path.clone());
            let mut items = pane
                .items()
                .iter()
                .copied()
                .filter(|file| restorable(*file))
                .filter_map(path)
                .collect::<Vec<_>>();
            let pending = pending.filter(|p| p.pane == *id);
            if let Some(pending) = pending
                && !items.contains(&pending.path)
            {
                let index = if let Some(TabDrop::Insert { index, .. }) = pending.drop {
                    pane.items()
                        .iter()
                        .take(index)
                        .filter(|file| workspace.is_file(**file))
                        .count()
                } else {
                    items.len()
                };
                items.insert(index.min(items.len()), pending.path.clone());
            }
            if items.is_empty() {
                return None;
            }
            if pending.is_some() || (active_pane.is_none() && *id == workspace.active_pane()) {
                *active_pane = Some(*leaf_index);
            }
            *leaf_index += 1;
            let active = pending.map(|p| p.path.clone()).or_else(|| {
                pane.active()
                    .filter(|file| restorable(*file))
                    .and_then(path)
            });
            Some(SavedPaneNode::Pane { items, active })
        }
        PaneNode::Split { axis, children } => {
            let mut saved = children
                .iter()
                .filter_map(|child| {
                    save_pane_node(workspace, &child.node, pending, leaf_index, active_pane).map(
                        |node| SavedChild {
                            size: child.size,
                            node,
                        },
                    )
                })
                .collect::<Vec<_>>();
            match saved.len() {
                0 => None,
                1 => saved.pop().map(|child| child.node),
                _ => {
                    let total: f32 = saved.iter().map(|child| child.size).sum();
                    for child in &mut saved {
                        child.size /= total.max(f32::EPSILON);
                    }
                    Some(SavedPaneNode::Split {
                        axis: match axis {
                            Axis::Horizontal => SavedAxis::Horizontal,
                            Axis::Vertical => SavedAxis::Vertical,
                        },
                        children: saved,
                    })
                }
            }
        }
    }
}

pub(crate) fn hydrate_editor_layout(app: &mut AppState, layout: Option<EditorLayout>) {
    restore_editor_layout_with(app, layout, |workspace, path| {
        Some(workspace.open_deferred(path))
    });
}

/// Rebuild the saved panes and reopen their files. Files that can no longer
/// be read are skipped, and a pane left without tabs is dropped.
#[cfg(test)]
fn hydrate_editor_layout_with(
    app: &mut AppState,
    layout: Option<EditorLayout>,
    mut read: impl FnMut(&Path) -> Result<String, String>,
) {
    restore_editor_layout_with(app, layout, |workspace, path| {
        read(&path).ok().map(|text| workspace.open_path(path, text))
    });
}

fn restore_editor_layout_with(
    app: &mut AppState,
    layout: Option<EditorLayout>,
    mut open: impl FnMut(&mut Workspace, PathBuf) -> Option<FileId>,
) {
    let Some(layout) = layout else {
        return;
    };
    let mut contents = Vec::new();
    let shape = restored_shape(layout.root, &mut contents);
    let panes = app.workspace.restore_layout(shape);
    for (&pane, (items, active)) in panes.iter().zip(contents) {
        app.workspace.activate_pane(pane);
        let mut first = None;
        let mut active_id = None;
        for path in items {
            let is_active = active.as_ref() == Some(&path);
            let Some(id) = open(&mut app.workspace, path) else {
                continue;
            };
            first.get_or_insert(id);
            if is_active {
                active_id = Some(id);
            }
        }
        if let Some(id) = active_id.or(first) {
            app.workspace.activate(pane, id);
        }
    }
    for &pane in &panes {
        if app.workspace.pane(pane).is_some_and(|pane| pane.is_empty()) {
            app.workspace.close_pane(pane);
        }
    }
    let focus = panes
        .get(layout.active_pane)
        .copied()
        .filter(|pane| app.workspace.pane(*pane).is_some())
        .unwrap_or_else(|| app.workspace.pane_ids()[0]);
    app.workspace.activate_pane(focus);
}

/// The tree shape of a saved layout with leaves numbered in reading order;
/// each leaf's tabs are pushed onto `contents` in the same order.
fn restored_shape(
    node: SavedPaneNode,
    contents: &mut Vec<(Vec<PathBuf>, Option<PathBuf>)>,
) -> PaneNode {
    match node {
        SavedPaneNode::Pane { items, active } => {
            let leaf = PaneId::new(contents.len() as u64);
            contents.push((items, active));
            PaneNode::Leaf(leaf)
        }
        SavedPaneNode::Split { axis, children } if !children.is_empty() => PaneNode::Split {
            axis: match axis {
                SavedAxis::Horizontal => Axis::Horizontal,
                SavedAxis::Vertical => Axis::Vertical,
            },
            children: children
                .into_iter()
                .map(|child| Child {
                    size: child.size,
                    node: restored_shape(child.node, contents),
                })
                .collect(),
        },
        SavedPaneNode::Split { .. } => {
            let leaf = PaneId::new(contents.len() as u64);
            contents.push((Vec::new(), None));
            PaneNode::Leaf(leaf)
        }
    }
}

fn existing_unique(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut unique = Vec::new();
    for path in paths {
        if !path.is_dir() {
            continue;
        }
        let path = normalized_folder(path);
        if !unique.contains(&path) {
            unique.push(path);
        }
    }
    unique
}

fn normalized_folder(path: PathBuf) -> PathBuf {
    fs::canonicalize(&path).unwrap_or(path)
}

fn remember_folder(app: &mut AppState, path: PathBuf) {
    app.recent_folders.retain(|recent| recent != &path);
    app.recent_folders.insert(0, path);
    app.recent_folders.truncate(MAX_RECENT_FOLDERS);
}

fn include_workspace_folders_in_recents(app: &mut AppState) {
    for path in &app.workspace_folders {
        if !app.recent_folders.contains(path) {
            app.recent_folders.push(path.clone());
        }
    }
    app.recent_folders.truncate(MAX_RECENT_FOLDERS);
}

fn persist_workspace(app: &mut AppState) {
    if let Err(error) = workspace_persistence::save(&app.workspace_folders, &app.recent_folders) {
        app.show_error(format!("Could not remember workspace folders: {error}"));
    }
}

pub(crate) fn read_text_file(path: &Path) -> Result<String, String> {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let file = fs::File::open(path).map_err(|error| format!("Could not open {name}: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Could not open {name}: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("Could not open {name}: not a regular file"));
    }
    let too_large =
        || format!("Could not open {name}: file is larger than {MAX_EDITABLE_FILE_MIB} MiB");
    if metadata.len() > MAX_EDITABLE_FILE_BYTES {
        return Err(too_large());
    }
    // Bound the read as well: another process can grow the file after metadata.
    let mut bytes = Vec::new();
    file.take(MAX_EDITABLE_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not open {name}: {error}"))?;
    if bytes.len() as u64 > MAX_EDITABLE_FILE_BYTES {
        return Err(too_large());
    }
    String::from_utf8(bytes).map_err(|error| format!("Could not open {name} as UTF-8: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::text::EditKind;

    #[test]
    fn text_file_above_four_mib_opens_in_explorer() {
        let root = close_test_directory("large-text-file");
        let path = root.join("seed.database.json");
        let contents = format!("[\n{}null\n]\n", "\"中文😀\",\n".repeat(400_000));
        assert!(contents.len() > 4 * 1024 * 1024);
        fs::write(&path, &contents).unwrap();
        assert_eq!(read_text_file(&path).unwrap(), contents);

        let mut app = AppState::new();
        assert!(open_tree_file(&mut app, path.clone(), 2));
        assert_eq!(app.workspace.active_path(), Some(path.as_path()));
        let id = app.workspace.active().unwrap();
        assert_eq!(app.workspace.save_snapshot(id).unwrap().2, contents);
        assert!(app.toast.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn text_file_size_boundary_is_inclusive() {
        let root = close_test_directory("text-file-size-boundary");
        let path = root.join("large.txt");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_EDITABLE_FILE_BYTES).unwrap();
        drop(file);
        assert_eq!(
            read_text_file(&path).unwrap().len() as u64,
            MAX_EDITABLE_FILE_BYTES
        );
        let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(MAX_EDITABLE_FILE_BYTES + 1).unwrap();
        drop(file);
        assert_eq!(
            read_text_file(&path).unwrap_err(),
            "Could not open large.txt: file is larger than 64 MiB"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn text_file_rejects_invalid_utf8_and_directories() {
        let root = close_test_directory("invalid-text-file");
        let path = root.join("invalid.txt");
        fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(read_text_file(&path).unwrap_err().contains("as UTF-8"));
        assert!(read_text_file(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    fn tabs(app: &AppState, ids: &[FileId]) -> Vec<(PaneId, FileId)> {
        let pane = app.workspace.active_pane();
        ids.iter().map(|id| (pane, *id)).collect()
    }

    fn close_tab(app: &mut AppState, id: FileId) {
        let pane = app.workspace.active_pane();
        request_close_tab_in(app, pane, id);
    }

    #[test]
    fn clean_tab_closes_without_confirmation() {
        let mut app = AppState::new();
        let id = app
            .workspace
            .open_path(PathBuf::from("workspace/clean.rs"), String::new());

        close_tab(&mut app, id);

        assert!(app.workspace.meta(id).is_none());
        assert!(app.editor.close_request.is_none());
    }

    #[test]
    fn closing_one_of_two_tabs_of_a_dirty_document_needs_no_confirmation() {
        let mut app = AppState::new();
        let left = app.workspace.active_pane();
        let id = app
            .workspace
            .open_path(PathBuf::from("workspace/shared.rs"), String::new());
        let right = app
            .workspace
            .split(left, crate::model::pane_layout::Direction::Right)
            .unwrap();
        app.workspace
            .active_editor_mut()
            .unwrap()
            .replace_selection("changed", EditKind::Typing)
            .unwrap();

        request_close_tab_in(&mut app, right, id);
        assert!(app.editor.close_request.is_none());
        assert!(app.workspace.is_dirty(id));

        request_close_tab_in(&mut app, left, id);
        assert_eq!(
            app.editor.close_request.map(|request| request.targets),
            Some(vec![(left, id)])
        );
    }

    #[test]
    fn dirty_tab_waits_for_confirmation() {
        let mut app = AppState::new();
        let id = app
            .workspace
            .open_path(PathBuf::from("workspace/dirty.rs"), String::new());
        app.workspace
            .active_editor_mut()
            .unwrap()
            .replace_selection("changed", EditKind::Typing)
            .unwrap();

        close_tab(&mut app, id);

        assert!(app.workspace.meta(id).is_some());
        assert_eq!(
            app.editor.close_request,
            Some(CloseRequest {
                targets: tabs(&app, &[id]),
                continuation: CloseContinuation::CloseTabs,
            })
        );
    }

    #[test]
    fn launch_paths_open_the_first_folder_add_others_and_open_files() {
        let root = close_test_directory("launch-paths");
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let file = first.join("main.rs");
        let mut app = AppState::new();
        app.workspace_folders
            .push(PathBuf::from("previous-workspace"));

        let changed = open_launch_paths_with(
            &mut app,
            vec![first.clone(), file.clone(), second.clone()],
            |_| Ok("fn main() {}".to_owned()),
        );

        assert!(changed);
        assert_eq!(
            app.workspace_folders,
            vec![normalized_folder(first), normalized_folder(second)]
        );
        let active = app.workspace.active().unwrap();
        assert_eq!(app.workspace.meta(active).unwrap().path, file);
        assert!(app.toast.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unreadable_launch_files_report_one_toast_and_keep_the_workspace() {
        let mut app = AppState::new();
        app.workspace_folders
            .push(PathBuf::from("previous-workspace"));

        let changed = open_launch_paths_with(
            &mut app,
            vec![PathBuf::from("missing-a.rs"), PathBuf::from("missing-b.rs")],
            |path| Err(format!("Could not open {}", path.display())),
        );

        assert!(!changed);
        assert_eq!(
            app.workspace_folders,
            vec![PathBuf::from("previous-workspace")]
        );
        assert!(app.workspace.active_items().is_empty());
        assert!(
            app.toast
                .as_ref()
                .unwrap()
                .message
                .ends_with("(and 1 more)")
        );
    }

    #[test]
    fn keyboard_close_uses_the_guarded_request_for_the_active_tab() {
        use crate::input::action::Action;

        let mut app = AppState::new();
        let clean = app
            .workspace
            .open_path(PathBuf::from("workspace/clean.rs"), String::new());
        let dirty = app
            .workspace
            .open_path(PathBuf::from("workspace/dirty.rs"), String::new());
        app.workspace
            .active_editor_mut()
            .unwrap()
            .replace_selection("changed", EditKind::Typing)
            .unwrap();

        app.apply(Action::CloseActiveItem);
        assert!(app.workspace.meta(dirty).is_some());
        assert_eq!(
            app.editor
                .close_request
                .as_ref()
                .map(|request| request.targets.clone()),
            Some(tabs(&app, &[dirty]))
        );

        // A pending request is never replaced by another keyboard close.
        app.workspace.set_active(clean);
        app.apply(Action::CloseActiveItem);
        assert!(app.workspace.meta(clean).is_some());
        assert_eq!(
            app.editor
                .close_request
                .as_ref()
                .map(|request| request.targets.clone()),
            Some(tabs(&app, &[dirty]))
        );

        app.editor.close_request = None;
        app.apply(Action::CloseActiveItem);
        assert!(app.workspace.meta(clean).is_none());
        assert!(app.editor.close_request.is_none());
    }

    #[test]
    fn tab_close_scopes_follow_visible_order_and_dirty_state() {
        let mut app = AppState::new();
        let first = app
            .workspace
            .open_path(PathBuf::from("first.rs"), String::new());
        let second = app
            .workspace
            .open_path(PathBuf::from("second.rs"), String::new());
        app.workspace
            .active_editor_mut()
            .unwrap()
            .replace_selection("dirty", EditKind::Typing)
            .unwrap();
        let third = app
            .workspace
            .open_path(PathBuf::from("third.rs"), String::new());
        let fourth = app
            .workspace
            .open_path(PathBuf::from("fourth.rs"), String::new());

        assert_eq!(
            tab_close_targets(
                &app,
                app.workspace.active_pane(),
                second,
                TabCloseScope::Tab
            ),
            tabs(&app, &[second])
        );
        assert_eq!(
            tab_close_targets(
                &app,
                app.workspace.active_pane(),
                second,
                TabCloseScope::Others
            ),
            tabs(&app, &[first, third, fourth])
        );
        assert_eq!(
            tab_close_targets(
                &app,
                app.workspace.active_pane(),
                second,
                TabCloseScope::Right
            ),
            tabs(&app, &[third, fourth])
        );
        assert_eq!(
            tab_close_targets(
                &app,
                app.workspace.active_pane(),
                second,
                TabCloseScope::Saved
            ),
            tabs(&app, &[first, third, fourth])
        );
        assert_eq!(
            tab_close_targets(
                &app,
                app.workspace.active_pane(),
                second,
                TabCloseScope::All
            ),
            tabs(&app, &[first, second, third, fourth])
        );
    }

    #[test]
    fn mixed_batch_waits_for_dirty_review_before_closing_clean_tabs() {
        let mut app = AppState::new();
        let clean = app
            .workspace
            .open_path(PathBuf::from("clean.rs"), String::new());
        let dirty = app
            .workspace
            .open_path(PathBuf::from("dirty.rs"), String::new());
        app.workspace
            .active_editor_mut()
            .unwrap()
            .replace_selection("dirty", EditKind::Typing)
            .unwrap();

        let targets = tabs(&app, &[clean, dirty]);
        request_close_tabs_in(&mut app, targets);

        assert!(app.workspace.meta(clean).is_some());
        assert!(app.workspace.meta(dirty).is_some());
        assert_eq!(
            app.editor.close_request,
            Some(CloseRequest {
                targets: tabs(&app, &[clean, dirty]),
                continuation: CloseContinuation::CloseTabs,
            })
        );
    }

    #[test]
    fn saving_for_application_exit_preserves_tabs_for_the_final_snapshot() {
        let root = close_test_directory("save-all");
        let mut app = AppState::new();
        let first_path = root.join("first.txt");
        let second_path = root.join("second.txt");
        let first = dirty_test_file(&mut app, first_path.clone(), "first", " updated");
        let second = dirty_test_file(&mut app, second_path.clone(), "second", " updated");
        app.editor.close_request = Some(CloseRequest {
            targets: tabs(&app, &[first, second]),
            continuation: CloseContinuation::ExitApplication,
        });

        let result = save_close_request_in(&mut app).unwrap();

        assert_eq!(result, Some(CloseContinuation::ExitApplication));
        assert!(app.workspace.meta(first).is_some());
        assert!(app.workspace.meta(second).is_some());
        assert!(!app.workspace.is_dirty(first));
        assert!(!app.workspace.is_dirty(second));
        let saved = saved_app_editor_layout(&app).unwrap();
        assert_eq!(
            saved.root,
            SavedPaneNode::Pane {
                items: vec![first_path.clone(), second_path.clone()],
                active: Some(second_path.clone())
            }
        );
        assert!(app.editor.close_request.is_none());
        assert_eq!(fs::read_to_string(first_path).unwrap(), "first updated");
        assert_eq!(fs::read_to_string(second_path).unwrap(), "second updated");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_batch_save_keeps_every_tab_open() {
        let root = close_test_directory("partial-failure");
        let mut app = AppState::new();
        let first_path = root.join("first.txt");
        let first = dirty_test_file(&mut app, first_path.clone(), "first", " updated");
        let missing_path = root.join("missing-parent").join("second.txt");
        let second = app.workspace.open_path(missing_path, "second".to_owned());
        app.workspace
            .active_editor_mut()
            .unwrap()
            .select_to(usize::MAX, false);
        app.workspace
            .active_editor_mut()
            .unwrap()
            .replace_selection(" updated", EditKind::Typing)
            .unwrap();
        app.editor.close_request = Some(CloseRequest {
            targets: tabs(&app, &[first, second]),
            continuation: CloseContinuation::CloseTabs,
        });

        let error = save_close_request_in(&mut app).unwrap_err();

        assert!(error.contains("Could not save"));
        assert!(app.workspace.meta(first).is_some());
        assert!(app.workspace.meta(second).is_some());
        assert!(!app.workspace.is_dirty(first));
        assert!(app.workspace.is_dirty(second));
        assert!(app.editor.close_request.is_some());
        assert_eq!(fs::read_to_string(first_path).unwrap(), "first updated");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn discarding_multiple_dirty_tabs_closes_every_target() {
        let mut app = AppState::new();
        let first = app
            .workspace
            .open_path(PathBuf::from("first.txt"), String::new());
        app.workspace
            .active_editor_mut()
            .unwrap()
            .replace_selection("first", EditKind::Typing)
            .unwrap();
        let second = app
            .workspace
            .open_path(PathBuf::from("second.txt"), String::new());
        app.workspace
            .active_editor_mut()
            .unwrap()
            .replace_selection("second", EditKind::Typing)
            .unwrap();
        app.editor.close_request = Some(CloseRequest {
            targets: tabs(&app, &[first, second]),
            continuation: CloseContinuation::CloseTabs,
        });

        let result = discard_close_request_in(&mut app);

        assert_eq!(result, Some(CloseContinuation::CloseTabs));
        assert!(app.workspace.meta(first).is_none());
        assert!(app.workspace.meta(second).is_none());
        assert!(app.editor.close_request.is_none());
    }

    #[test]
    fn cancelling_batch_close_preserves_dirty_tabs() {
        let mut app = AppState::new();
        let id = app
            .workspace
            .open_path(PathBuf::from("dirty.txt"), String::new());
        app.workspace
            .active_editor_mut()
            .unwrap()
            .replace_selection("changed", EditKind::Typing)
            .unwrap();
        app.editor.close_request = Some(CloseRequest {
            targets: tabs(&app, &[id]),
            continuation: CloseContinuation::ExitApplication,
        });

        cancel_close_request_in(&mut app);

        assert!(app.workspace.meta(id).is_some());
        assert!(app.workspace.is_dirty(id));
        assert!(app.editor.close_request.is_none());
    }

    #[test]
    fn derives_clone_directory_from_common_repository_urls() {
        assert_eq!(
            repository_directory_name("https://github.com/owner/project.git"),
            Some("project".to_owned())
        );
        assert_eq!(
            repository_directory_name("git@github.com:owner/project.git"),
            Some("project".to_owned())
        );
        assert_eq!(
            repository_directory_name("https://example.test/owner/project/"),
            Some("project".to_owned())
        );
    }

    #[test]
    fn adding_a_folder_preserves_existing_roots_and_ignores_duplicates() {
        let mut app = AppState::new();
        let first = PathBuf::from("first-workspace");
        let second = PathBuf::from("second-workspace");

        replace_folder_state(&mut app, first.clone());
        assert!(add_folder_state(&mut app, second.clone()));
        assert!(!add_folder_state(&mut app, first.clone()));

        assert_eq!(app.workspace_folders, vec![first.clone(), second.clone()]);
        assert!(app.expanded.contains(&first.to_string_lossy().into_owned()));
        assert!(
            app.expanded
                .contains(&second.to_string_lossy().into_owned())
        );
        assert_eq!(app.recent_folders, vec![first.clone(), second]);
    }

    #[test]
    fn opening_a_folder_replaces_the_current_project_roots() {
        let mut app = AppState::new();
        let first = PathBuf::from("first-workspace");
        let second = PathBuf::from("second-workspace");

        add_folder_state(&mut app, first);
        replace_folder_state(&mut app, second.clone());

        assert_eq!(app.workspace_folders, vec![second]);
    }

    #[test]
    fn removing_a_workspace_folder_keeps_disk_documents_and_clears_tree_state() {
        let mut app = AppState::new();
        let removed = PathBuf::from("first-workspace");
        let child = removed.join("src");
        let kept = PathBuf::from("second-workspace");
        let open_file = child.join("main.rs");
        app.workspace_folders = vec![removed.clone(), kept.clone()];
        app.recent_folders = vec![removed.clone(), kept.clone()];
        app.dir_entries
            .insert(removed.to_string_lossy().into_owned(), Vec::new().into());
        app.dir_entries
            .insert(child.to_string_lossy().into_owned(), Vec::new().into());
        app.dir_entries
            .insert(kept.to_string_lossy().into_owned(), Vec::new().into());
        app.expanded.insert(removed.to_string_lossy().into_owned());
        app.expanded.insert(child.to_string_lossy().into_owned());
        app.expanded.insert(kept.to_string_lossy().into_owned());
        app.tree_hovered_path = Some(open_file.to_string_lossy().into_owned());
        app.explorer_create = Some(ExplorerCreateRequest {
            kind: ExplorerCreateKind::File,
            parent: child.clone(),
        });
        app.workspace.open_path(open_file.clone(), String::new());

        assert!(remove_folder_state(&mut app, &removed));

        assert_eq!(app.workspace_folders, vec![kept.clone()]);
        assert_eq!(app.recent_folders, vec![removed.clone(), kept.clone()]);
        assert!(
            app.dir_entries
                .keys()
                .all(|path| !Path::new(path).starts_with(&removed))
        );
        assert!(
            app.expanded
                .iter()
                .all(|path| !Path::new(path).starts_with(&removed))
        );
        assert!(
            app.dir_entries
                .contains_key(&kept.to_string_lossy().into_owned())
        );
        assert!(app.expanded.contains(&kept.to_string_lossy().into_owned()));
        assert!(app.tree_hovered_path.is_none());
        assert!(app.explorer_create.is_none());
        assert_eq!(app.workspace.active_path(), Some(open_file.as_path()));
    }

    #[test]
    fn recent_folders_are_mru_ordered_and_bounded() {
        let mut app = AppState::new();

        for index in 0..MAX_RECENT_FOLDERS + 2 {
            remember_folder(&mut app, PathBuf::from(format!("workspace-{index}")));
        }
        remember_folder(&mut app, PathBuf::from("workspace-4"));

        assert_eq!(app.recent_folders.len(), MAX_RECENT_FOLDERS);
        assert_eq!(app.recent_folders[0], PathBuf::from("workspace-4"));
        assert_eq!(app.recent_folders[1], PathBuf::from("workspace-9"));
    }

    #[test]
    fn restored_roots_do_not_reorder_existing_recent_history() {
        let mut app = AppState::new();
        let first = PathBuf::from("first-workspace");
        let second = PathBuf::from("second-workspace");
        let older = PathBuf::from("older-workspace");
        app.workspace_folders = vec![first.clone(), second.clone()];
        app.recent_folders = vec![second.clone(), older.clone(), first.clone()];

        include_workspace_folders_in_recents(&mut app);

        assert_eq!(app.recent_folders, vec![second, older, first]);
    }

    fn press_file(app: &mut AppState, path: &str, clicks: u8, drop: Option<TabDrop>) {
        app.editor.file_drag = Some(crate::state::FileDragState {
            path: PathBuf::from(path),
            source: crate::state::FileDragSource::Explorer { clicks },
            origin: (0.0, 0.0),
            point: (0.0, 0.0),
            active: drop.is_some(),
            drop,
        });
    }

    fn read_ok(path: &Path) -> Result<String, String> {
        Ok(format!("// {}", path.display()))
    }

    #[test]
    fn releasing_an_undragged_file_opens_it_like_a_click() {
        let mut app = AppState::new();
        press_file(&mut app, "a.rs", 1, None);
        assert!(finish_file_drag_with(&mut app, read_ok));
        assert!(app.editor.file_drag.is_none());
        let id = app.workspace.active().unwrap();
        assert_eq!(app.workspace.preview(), Some(id));

        press_file(&mut app, "a.rs", 2, None);
        finish_file_drag_with(&mut app, read_ok);
        assert_eq!(app.workspace.preview(), None);
        assert_eq!(app.workspace.active_items(), &[id]);
    }

    #[test]
    fn dropping_a_file_on_a_strip_inserts_it_at_the_marker() {
        let mut app = AppState::new();
        let pane = app.workspace.active_pane();
        let first = app
            .workspace
            .open_path(PathBuf::from("first.rs"), String::new());
        let second = app
            .workspace
            .open_path(PathBuf::from("second.rs"), String::new());
        press_file(
            &mut app,
            "new.rs",
            1,
            Some(TabDrop::Insert { pane, index: 1 }),
        );

        assert!(finish_file_drag_with(&mut app, read_ok));

        let new = app.workspace.active().unwrap();
        assert_eq!(app.workspace.active_items(), &[first, new, second]);
        // A dragged file opens as a permanent tab.
        assert_eq!(app.workspace.preview(), None);
    }

    #[test]
    fn dropping_a_file_near_a_pane_edge_opens_it_in_a_new_split() {
        let mut app = AppState::new();
        let pane = app.workspace.active_pane();
        let first = app
            .workspace
            .open_path(PathBuf::from("first.rs"), String::new());
        press_file(
            &mut app,
            "new.rs",
            1,
            Some(TabDrop::Pane {
                pane,
                split: Some(crate::model::pane_layout::Direction::Down),
            }),
        );

        assert!(finish_file_drag_with(&mut app, read_ok));

        let panes = app.workspace.pane_ids();
        assert_eq!(panes.len(), 2);
        assert_eq!(app.workspace.pane(panes[0]).unwrap().items(), &[first]);
        assert_eq!(app.workspace.active_pane(), panes[1]);
        assert_eq!(app.workspace.active_path(), Some(Path::new("new.rs")));
    }

    #[test]
    fn dropping_a_file_on_another_pane_opens_it_there_and_failures_change_nothing() {
        let mut app = AppState::new();
        let left = app.workspace.active_pane();
        app.workspace
            .open_path(PathBuf::from("first.rs"), String::new());
        let right = app
            .workspace
            .split(left, crate::model::pane_layout::Direction::Right)
            .unwrap();
        app.workspace.activate_pane(left);
        let drop = Some(TabDrop::Pane {
            pane: right,
            split: None,
        });

        press_file(&mut app, "gone.rs", 1, drop);
        assert!(!finish_file_drag_with(&mut app, |_| Err("gone".to_owned())));
        assert_eq!(app.workspace.pane_count(), 2);
        assert_eq!(app.workspace.active_pane(), left);

        press_file(&mut app, "new.rs", 1, drop);
        assert!(finish_file_drag_with(&mut app, read_ok));
        assert_eq!(app.workspace.active_pane(), right);
        assert_eq!(app.workspace.active_path(), Some(Path::new("new.rs")));
    }

    fn read_all_but(missing: &'static str) -> impl FnMut(&Path) -> Result<String, String> {
        move |path| {
            if path == Path::new(missing) {
                Err("missing".to_owned())
            } else {
                Ok(format!("// {}", path.display()))
            }
        }
    }

    #[test]
    fn restores_file_tabs_in_order_and_reactivates_the_saved_file() {
        let mut app = AppState::new();
        let first = PathBuf::from("first.rs");
        let second = PathBuf::from("second.rs");
        let layout = EditorLayout {
            root: SavedPaneNode::Pane {
                items: vec![first.clone(), PathBuf::from("missing.rs"), second.clone()],
                active: Some(first.clone()),
            },
            active_pane: 0,
        };

        hydrate_editor_layout_with(&mut app, Some(layout), read_all_but("missing.rs"));

        assert_eq!(app.workspace.open_paths(), vec![first.clone(), second]);
        assert_eq!(app.workspace.active_path(), Some(first.as_path()));
    }

    #[test]
    fn unclosed_single_click_preview_restores_in_order_and_closed_tabs_stay_closed() {
        let mut app = AppState::new();
        app.workspace.open_path("first.rs".into(), String::new());
        let closed = app.workspace.open_path("closed.rs".into(), String::new());
        app.workspace.close(closed);
        app.workspace
            .preview_path("preview.json".into(), String::new());
        let saved = saved_app_editor_layout(&app).unwrap();
        assert_eq!(
            saved.root,
            SavedPaneNode::Pane {
                items: vec!["first.rs".into(), "preview.json".into()],
                active: Some("preview.json".into())
            }
        );
        let serialized = serde_json::to_string(&saved).unwrap();
        let mut restored = AppState::new();
        hydrate_editor_layout_with(
            &mut restored,
            Some(serde_json::from_str(&serialized).unwrap()),
            read_all_but(""),
        );
        assert_eq!(
            restored.workspace.active_path(),
            Some(Path::new("preview.json"))
        );
        assert!(
            restored
                .workspace
                .file_id_for_path(Path::new("closed.rs"))
                .is_none()
        );
        assert_eq!(saved_app_editor_layout(&restored), Some(saved));
    }

    #[test]
    fn dont_save_on_application_exit_keeps_tabs_and_reopens_disk_contents() {
        let root = close_test_directory("discard-on-exit");
        let file = root.join("dirty.txt");
        let mut app = AppState::new();
        let id = dirty_test_file(&mut app, file.clone(), "disk", " modified");
        app.editor.close_request = Some(CloseRequest {
            targets: tabs(&app, &[id]),
            continuation: CloseContinuation::ExitApplication,
        });
        assert_eq!(
            discard_close_request_in(&mut app),
            Some(CloseContinuation::ExitApplication)
        );
        assert!(app.workspace.meta(id).is_some());
        let layout = saved_app_editor_layout(&app);
        let mut restored = AppState::new();
        hydrate_editor_layout_with(&mut restored, layout, read_text_file);
        assert_eq!(restored.workspace.active_path(), Some(file.as_path()));
        assert_eq!(restored.workspace.active_editor().unwrap().text(), "disk");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn split_layout_round_trips_through_the_session_format() {
        let mut app = AppState::new();
        let left = app.workspace.active_pane();
        app.workspace.open_path(PathBuf::from("a.rs"), "a".into());
        app.workspace.open_path(PathBuf::from("b.rs"), "b".into());
        let right = app
            .workspace
            .split(left, crate::model::pane_layout::Direction::Right)
            .unwrap();
        app.workspace
            .preview_path(PathBuf::from("preview.rs"), String::new());
        app.workspace.open_settings();
        app.workspace
            .split(right, crate::model::pane_layout::Direction::Down)
            .unwrap();
        app.workspace.open_path(PathBuf::from("c.rs"), "c".into());
        app.workspace.activate_pane(right);

        let saved = saved_editor_layout(&app.workspace).unwrap();
        // The settings page is excluded; the open preview is restored too.
        assert_eq!(
            saved.root,
            SavedPaneNode::Split {
                axis: SavedAxis::Horizontal,
                children: vec![
                    SavedChild {
                        size: 0.5,
                        node: SavedPaneNode::Pane {
                            items: vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")],
                            active: Some(PathBuf::from("b.rs")),
                        },
                    },
                    SavedChild {
                        size: 0.5,
                        node: SavedPaneNode::Split {
                            axis: SavedAxis::Vertical,
                            children: vec![
                                SavedChild {
                                    size: 0.5,
                                    node: SavedPaneNode::Pane {
                                        items: vec![
                                            PathBuf::from("b.rs"),
                                            PathBuf::from("preview.rs")
                                        ],
                                        active: None,
                                    },
                                },
                                SavedChild {
                                    size: 0.5,
                                    node: SavedPaneNode::Pane {
                                        items: vec![PathBuf::from("c.rs")],
                                        active: Some(PathBuf::from("c.rs")),
                                    },
                                },
                            ],
                        },
                    },
                ],
            }
        );
        assert_eq!(saved.active_pane, 1);

        let mut restored = AppState::new();
        hydrate_editor_layout_with(&mut restored, Some(saved.clone()), read_all_but(""));
        assert_eq!(restored.workspace.pane_count(), 3);
        let resaved = saved_editor_layout(&restored.workspace).unwrap();
        assert_eq!(resaved.active_pane, 1);
        assert_eq!(
            restored.workspace.open_paths(),
            vec![
                PathBuf::from("a.rs"),
                PathBuf::from("b.rs"),
                PathBuf::from("preview.rs"),
                PathBuf::from("c.rs")
            ]
        );
        // Without a saved active tab, the restored pane activates its first tab.
        let SavedPaneNode::Split { children, .. } = resaved.root else {
            panic!("expected a split")
        };
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn panes_whose_files_are_gone_are_dropped_on_restore() {
        let mut app = AppState::new();
        let layout = EditorLayout {
            root: SavedPaneNode::Split {
                axis: SavedAxis::Horizontal,
                children: vec![
                    SavedChild {
                        size: 0.3,
                        node: SavedPaneNode::Pane {
                            items: vec![PathBuf::from("gone.rs")],
                            active: None,
                        },
                    },
                    SavedChild {
                        size: 0.7,
                        node: SavedPaneNode::Pane {
                            items: vec![PathBuf::from("kept.rs")],
                            active: None,
                        },
                    },
                ],
            },
            active_pane: 0,
        };

        hydrate_editor_layout_with(&mut app, Some(layout), read_all_but("gone.rs"));

        assert_eq!(app.workspace.pane_count(), 1);
        assert_eq!(app.workspace.open_paths(), vec![PathBuf::from("kept.rs")]);
        assert_eq!(app.workspace.active_path(), Some(Path::new("kept.rs")));
    }

    #[test]
    fn create_path_validation_accepts_nested_relative_names() {
        let parent = PathBuf::from("workspace");

        assert_eq!(
            validate_create_path(&parent, "src/components/button.rs").unwrap(),
            PathBuf::from("src").join("components").join("button.rs")
        );
        assert_eq!(
            validate_create_path(&parent, "assets\\icons\\").unwrap(),
            PathBuf::from("assets").join("icons")
        );
    }

    #[test]
    fn create_path_validation_rejects_empty_absolute_and_parent_names() {
        let parent = PathBuf::from("workspace");

        assert!(validate_create_path(&parent, "   ").is_err());
        assert!(validate_create_path(&parent, "/absolute").is_err());
        assert!(validate_create_path(&parent, "../outside").is_err());
        assert!(validate_create_path(&parent, "src/../outside").is_err());
    }

    #[test]
    fn rename_validation_accepts_one_folder_name_and_rejects_paths() {
        let path = PathBuf::from("__loom_missing__/old-name");

        assert_eq!(
            validate_rename_path(&path, "new-name").unwrap(),
            PathBuf::from("__loom_missing__/new-name")
        );
        assert!(validate_rename_path(&path, "nested/name").is_err());
        assert!(validate_rename_path(&path, "..").is_err());
        assert!(validate_rename_path(&path, "   ").is_err());
    }

    #[test]
    fn renamed_directory_retargets_tabs_and_rebuilds_tree_state() {
        let mut app = AppState::new();
        let root = PathBuf::from("workspace");
        let old_path = root.join("src");
        let new_path = root.join("source");
        let old_file = old_path.join("main.rs");
        let new_file = new_path.join("main.rs");
        app.workspace_folders = vec![root];
        app.dir_entries
            .insert(old_path.to_string_lossy().into_owned(), Vec::new().into());
        app.expanded.insert(old_path.to_string_lossy().into_owned());
        app.workspace.open_path(old_file, String::new());

        apply_renamed_path_state(&mut app, &old_path, &new_path);

        assert_eq!(app.workspace.active_path(), Some(new_file.as_path()));
        assert!(
            app.dir_entries
                .keys()
                .all(|path| !Path::new(path).starts_with(&old_path))
        );
        assert!(
            app.expanded
                .contains(&new_path.to_string_lossy().into_owned())
        );
        assert!(app.explorer_rename.is_none());
    }

    #[test]
    fn renamed_file_retargets_its_open_editor_tab() {
        let mut app = AppState::new();
        let root = PathBuf::from("workspace");
        let old_path = root.join("before.rs");
        let new_path = root.join("after.rs");
        app.workspace_folders = vec![root];
        app.workspace.open_path(old_path.clone(), String::new());

        apply_renamed_path_state(&mut app, &old_path, &new_path);

        assert_eq!(app.workspace.active_path(), Some(new_path.as_path()));
        assert_eq!(app.workspace.file_id_for_path(&old_path), None);
    }

    #[test]
    fn deleted_file_closes_its_open_editor_tab() {
        let mut app = AppState::new();
        let root = PathBuf::from("workspace");
        let path = root.join("deleted.rs");
        app.workspace_folders = vec![root];
        app.workspace.open_path(path.clone(), String::new());

        apply_deleted_path_state(&mut app, &path);

        assert_eq!(app.workspace.file_id_for_path(&path), None);
        assert_eq!(app.workspace.active_path(), None);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn create_path_validation_rejects_windows_device_and_invalid_names() {
        let parent = PathBuf::from("workspace");

        assert!(validate_create_path(&parent, "CON.txt").is_err());
        assert!(validate_create_path(&parent, "bad?.txt").is_err());
        assert!(validate_create_path(&parent, "trailing.").is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn shell_paths_hide_windows_verbatim_prefixes() {
        assert_eq!(
            windows_shell_path(Path::new(r"\\?\D:\Documents\project")),
            r"D:\Documents\project"
        );
        assert_eq!(
            windows_shell_path(Path::new(r"\\?\UNC\server\share\project")),
            r"\\server\share\project"
        );
    }
}
