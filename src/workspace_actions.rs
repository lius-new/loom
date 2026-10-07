//! Shared workspace commands used by the welcome screen and editor chrome.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread;

use lgui::dialogs::{FileDialogOptions, system_file_dialogs};
use lgui::prelude::State;

use crate::file_tree::read_directory;
use crate::model::document::FileId;
use crate::state::{
    AppState, CloseContinuation, CloseRequest, ExplorerCreateKind, ExplorerCreateRequest,
    ExplorerRenameRequest, ExplorerTargetKind,
};
use crate::workspace_persistence::{self, MAX_RECENT_FOLDERS};

const MAX_EDITABLE_FILE_BYTES: u64 = 4 * 1024 * 1024;
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
            state.update(move |app| {
                app.workspace.mark_saved(id);
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

pub fn request_close_tab(state: &State<AppState>, id: FileId) {
    state.update(move |app| request_close_tab_in(app, id));
}

pub(crate) fn request_close_tab_in(app: &mut AppState, id: FileId) {
    request_close_tabs_in(app, vec![id]);
}

pub fn tab_close_targets(app: &AppState, target: FileId, scope: TabCloseScope) -> Vec<FileId> {
    let open = app.workspace.open_files();
    let Some(target_index) = open.iter().position(|id| *id == target) else {
        return Vec::new();
    };
    match scope {
        TabCloseScope::Tab => vec![target],
        TabCloseScope::Others => open.iter().copied().filter(|id| *id != target).collect(),
        TabCloseScope::Right => open.iter().copied().skip(target_index + 1).collect(),
        TabCloseScope::Saved => open
            .iter()
            .copied()
            .filter(|id| !app.workspace.is_dirty(*id))
            .collect(),
        TabCloseScope::All => open.to_vec(),
    }
}

pub fn request_close_tabs(state: &State<AppState>, target: FileId, scope: TabCloseScope) {
    let targets = tab_close_targets(&state.get(), target, scope);
    state.update(move |app| request_close_tabs_in(app, targets));
}

fn request_close_tabs_in(app: &mut AppState, targets: Vec<FileId>) {
    let mut existing = Vec::with_capacity(targets.len());
    for id in targets {
        if app.workspace.meta(id).is_some() && !existing.contains(&id) {
            existing.push(id);
        }
    }
    if existing.is_empty() {
        return;
    }
    app.tab_context_menu = None;
    app.tab_drag = None;
    app.editor.menu = None;
    if existing.iter().any(|id| app.workspace.is_dirty(*id)) {
        app.close_request = Some(CloseRequest {
            targets: existing,
            continuation: CloseContinuation::CloseTabs,
        });
    } else {
        for id in existing {
            app.workspace.close(id);
        }
    }
}

pub fn cancel_close_request(state: &State<AppState>) {
    state.update(cancel_close_request_in);
}

fn cancel_close_request_in(app: &mut AppState) {
    app.close_request = None;
}

pub fn discard_close_request(state: &State<AppState>) -> Option<CloseContinuation> {
    let continuation = state.get().close_request.as_ref()?.continuation;
    state.update(|app| {
        let _ = discard_close_request_in(app);
    });
    Some(continuation)
}

fn discard_close_request_in(app: &mut AppState) -> Option<CloseContinuation> {
    let request = app.close_request.take()?;
    for id in request.targets {
        app.workspace.close(id);
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
    let Some(request) = app.close_request.clone() else {
        return Ok(None);
    };
    for id in &request.targets {
        if !app.workspace.is_dirty(*id) {
            continue;
        }
        let Some((id, path, contents)) = app.workspace.save_snapshot(*id) else {
            continue;
        };
        fs::write(&path, contents.as_bytes())
            .map_err(|error| format!("Could not save {}: {error}", path.display()))?;
        app.workspace.mark_saved(id);
    }
    for id in request.targets {
        app.workspace.close(id);
    }
    app.close_request = None;
    Ok(Some(request.continuation))
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
    for entry in entries {
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
    app.workspace.active_editor_mut().unwrap().move_end();
    app.workspace.active_editor_mut().unwrap().insert(change);
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

    match read_text_file(&path) {
        Ok(contents) => {
            state.update(move |app| {
                app.workspace.open_path(path, contents);
                app.toast = None;
                app.welcome_hover = None;
                app.workspace_home_hover = None;
            });
            true
        }
        Err(message) => {
            state.update(move |app| app.show_error(message));
            false
        }
    }
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
fn move_path_to_trash(path: &Path) -> Result<bool, String> {
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
fn move_path_to_trash(path: &Path) -> Result<bool, String> {
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
fn move_path_to_trash(path: &Path) -> Result<bool, String> {
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

fn apply_deleted_path_state(app: &mut AppState, path: &Path) {
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

fn add_folder(app: &mut AppState, path: PathBuf) {
    let path = normalized_folder(path);
    add_folder_state(app, path);
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
    if open_launch_paths_with(app, paths, read_text_file) {
        persist_workspace(app);
    }
}

/// Apply launch paths to the state; returns whether the workspace folders
/// changed and need to be persisted.
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

/// Reopen the persisted file tabs from disk, preserving their order and the
/// active tab. Files that are no longer readable are skipped independently.
pub(crate) fn hydrate_file_tabs(
    app: &mut AppState,
    open_files: Vec<PathBuf>,
    active_file: Option<PathBuf>,
) {
    hydrate_file_tabs_with(app, open_files, active_file, read_text_file);
}

fn hydrate_file_tabs_with(
    app: &mut AppState,
    open_files: Vec<PathBuf>,
    active_file: Option<PathBuf>,
    mut read: impl FnMut(&Path) -> Result<String, String>,
) {
    let mut first_restored = None;
    let mut active_restored = None;
    for path in open_files {
        let Ok(contents) = read(&path) else {
            continue;
        };
        let should_activate = active_file.as_deref() == Some(path.as_path());
        let id = app.workspace.open_path(path, contents);
        first_restored.get_or_insert(id);
        if should_activate {
            active_restored = Some(id);
        }
    }

    if let Some(id) = active_restored.or(first_restored) {
        app.workspace.set_active(id);
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

fn read_text_file(path: &Path) -> Result<String, String> {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let metadata = fs::metadata(path).map_err(|error| format!("Could not open {name}: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("Could not open {name}: not a regular file"));
    }
    if metadata.len() > MAX_EDITABLE_FILE_BYTES {
        return Err(format!("Could not open {name}: file is larger than 4 MiB"));
    }
    fs::read_to_string(path).map_err(|error| format!("Could not open {name} as UTF-8: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_tab_closes_without_confirmation() {
        let mut app = AppState::new();
        let id = app
            .workspace
            .open_path(PathBuf::from("workspace/clean.rs"), String::new());

        request_close_tab_in(&mut app, id);

        assert!(app.workspace.meta(id).is_none());
        assert!(app.close_request.is_none());
    }

    #[test]
    fn dirty_tab_waits_for_confirmation() {
        let mut app = AppState::new();
        let id = app
            .workspace
            .open_path(PathBuf::from("workspace/dirty.rs"), String::new());
        app.workspace.active_editor_mut().unwrap().insert("changed");

        request_close_tab_in(&mut app, id);

        assert!(app.workspace.meta(id).is_some());
        assert_eq!(
            app.close_request,
            Some(CloseRequest {
                targets: vec![id],
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
        assert!(app.workspace.open_files().is_empty());
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
        app.workspace.active_editor_mut().unwrap().insert("changed");

        app.apply(Action::CloseActiveItem);
        assert!(app.workspace.meta(dirty).is_some());
        assert_eq!(
            app.close_request
                .as_ref()
                .map(|request| request.targets.clone()),
            Some(vec![dirty])
        );

        // A pending request is never replaced by another keyboard close.
        app.workspace.set_active(clean);
        app.apply(Action::CloseActiveItem);
        assert!(app.workspace.meta(clean).is_some());
        assert_eq!(
            app.close_request
                .as_ref()
                .map(|request| request.targets.clone()),
            Some(vec![dirty])
        );

        app.close_request = None;
        app.apply(Action::CloseActiveItem);
        assert!(app.workspace.meta(clean).is_none());
        assert!(app.close_request.is_none());
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
        app.workspace.active_editor_mut().unwrap().insert("dirty");
        let third = app
            .workspace
            .open_path(PathBuf::from("third.rs"), String::new());
        let fourth = app
            .workspace
            .open_path(PathBuf::from("fourth.rs"), String::new());

        assert_eq!(
            tab_close_targets(&app, second, TabCloseScope::Tab),
            vec![second]
        );
        assert_eq!(
            tab_close_targets(&app, second, TabCloseScope::Others),
            vec![first, third, fourth]
        );
        assert_eq!(
            tab_close_targets(&app, second, TabCloseScope::Right),
            vec![third, fourth]
        );
        assert_eq!(
            tab_close_targets(&app, second, TabCloseScope::Saved),
            vec![first, third, fourth]
        );
        assert_eq!(
            tab_close_targets(&app, second, TabCloseScope::All),
            vec![first, second, third, fourth]
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
        app.workspace.active_editor_mut().unwrap().insert("dirty");

        request_close_tabs_in(&mut app, vec![clean, dirty]);

        assert!(app.workspace.meta(clean).is_some());
        assert!(app.workspace.meta(dirty).is_some());
        assert_eq!(
            app.close_request,
            Some(CloseRequest {
                targets: vec![clean, dirty],
                continuation: CloseContinuation::CloseTabs,
            })
        );
    }

    #[test]
    fn saving_multiple_dirty_tabs_closes_them_after_every_write_succeeds() {
        let root = close_test_directory("save-all");
        let mut app = AppState::new();
        let first_path = root.join("first.txt");
        let second_path = root.join("second.txt");
        let first = dirty_test_file(&mut app, first_path.clone(), "first", " updated");
        let second = dirty_test_file(&mut app, second_path.clone(), "second", " updated");
        app.close_request = Some(CloseRequest {
            targets: vec![first, second],
            continuation: CloseContinuation::ExitApplication,
        });

        let result = save_close_request_in(&mut app).unwrap();

        assert_eq!(result, Some(CloseContinuation::ExitApplication));
        assert!(app.workspace.meta(first).is_none());
        assert!(app.workspace.meta(second).is_none());
        assert!(app.close_request.is_none());
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
        app.workspace.active_editor_mut().unwrap().move_end();
        app.workspace
            .active_editor_mut()
            .unwrap()
            .insert(" updated");
        app.close_request = Some(CloseRequest {
            targets: vec![first, second],
            continuation: CloseContinuation::CloseTabs,
        });

        let error = save_close_request_in(&mut app).unwrap_err();

        assert!(error.contains("Could not save"));
        assert!(app.workspace.meta(first).is_some());
        assert!(app.workspace.meta(second).is_some());
        assert!(!app.workspace.is_dirty(first));
        assert!(app.workspace.is_dirty(second));
        assert!(app.close_request.is_some());
        assert_eq!(fs::read_to_string(first_path).unwrap(), "first updated");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn discarding_multiple_dirty_tabs_closes_every_target() {
        let mut app = AppState::new();
        let first = app
            .workspace
            .open_path(PathBuf::from("first.txt"), String::new());
        app.workspace.active_editor_mut().unwrap().insert("first");
        let second = app
            .workspace
            .open_path(PathBuf::from("second.txt"), String::new());
        app.workspace.active_editor_mut().unwrap().insert("second");
        app.close_request = Some(CloseRequest {
            targets: vec![first, second],
            continuation: CloseContinuation::CloseTabs,
        });

        let result = discard_close_request_in(&mut app);

        assert_eq!(result, Some(CloseContinuation::CloseTabs));
        assert!(app.workspace.meta(first).is_none());
        assert!(app.workspace.meta(second).is_none());
        assert!(app.close_request.is_none());
    }

    #[test]
    fn cancelling_batch_close_preserves_dirty_tabs() {
        let mut app = AppState::new();
        let id = app
            .workspace
            .open_path(PathBuf::from("dirty.txt"), String::new());
        app.workspace.active_editor_mut().unwrap().insert("changed");
        app.close_request = Some(CloseRequest {
            targets: vec![id],
            continuation: CloseContinuation::ExitApplication,
        });

        cancel_close_request_in(&mut app);

        assert!(app.workspace.meta(id).is_some());
        assert!(app.workspace.is_dirty(id));
        assert!(app.close_request.is_none());
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
            .insert(removed.to_string_lossy().into_owned(), Vec::new());
        app.dir_entries
            .insert(child.to_string_lossy().into_owned(), Vec::new());
        app.dir_entries
            .insert(kept.to_string_lossy().into_owned(), Vec::new());
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

    #[test]
    fn restores_file_tabs_in_order_and_reactivates_the_saved_file() {
        let mut app = AppState::new();
        let first = PathBuf::from("first.rs");
        let missing = PathBuf::from("missing.rs");
        let second = PathBuf::from("second.rs");

        hydrate_file_tabs_with(
            &mut app,
            vec![first.clone(), missing.clone(), second.clone()],
            Some(first.clone()),
            |path| {
                if path == missing {
                    Err("missing".to_owned())
                } else {
                    Ok(format!("// {}", path.display()))
                }
            },
        );

        assert_eq!(app.workspace.open_paths(), vec![first.clone(), second]);
        assert_eq!(app.workspace.active_path(), Some(first.as_path()));
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
            .insert(old_path.to_string_lossy().into_owned(), Vec::new());
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
