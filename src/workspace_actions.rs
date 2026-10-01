//! Shared workspace commands used by the welcome screen and editor chrome.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread;

use lgui::dialogs::{FileDialogOptions, system_file_dialogs};
use lgui::prelude::State;

use crate::state::{
    AppState, DirEntry, ExplorerCreateKind, ExplorerCreateRequest, ExplorerRenameRequest,
};
use crate::workspace_persistence::{self, MAX_RECENT_FOLDERS};

const MAX_EDITABLE_FILE_BYTES: u64 = 4 * 1024 * 1024;

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
            });
            true
        }
        Err(message) => {
            state.update(move |app| app.show_toast(message));
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

pub fn copy_directory(state: &State<AppState>, path: &Path, cut: bool) {
    let action = if cut { "Cut" } else { "Copied" };
    match set_directory_clipboard(path, cut) {
        Ok(()) => {
            let name = path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            state.update(move |app| app.show_toast(format!("{action} {name}.")));
        }
        Err(error) => state.update(move |app| app.show_toast(error)),
    }
}

#[cfg(target_os = "windows")]
fn set_directory_clipboard(path: &Path, cut: bool) -> Result<(), String> {
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
fn set_directory_clipboard(_path: &Path, _cut: bool) -> Result<(), String> {
    Err("Copying folders is not supported on this platform yet.".to_owned())
}

pub fn begin_explorer_rename(state: &State<AppState>, path: PathBuf) {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        state.update(|app| app.show_toast("This folder name cannot be edited as text."));
        return;
    };
    let name = name.to_owned();
    state.update(move |app| {
        app.explorer_create = None;
        app.explorer_rename = Some(ExplorerRenameRequest { path });
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
                app.show_toast(message);
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
            app.show_toast(message);
        });
        return;
    }

    state.update(move |app| apply_renamed_directory_state(app, &request.path, &target));
}

fn validate_rename_path(path: &Path, raw_name: &str) -> Result<PathBuf, String> {
    let name = raw_name.trim_matches('\t');
    if name.is_empty() || name.chars().all(char::is_whitespace) {
        return Err("A folder name must be provided.".to_owned());
    }
    if name.contains(['/', '\\']) || matches!(name, "." | "..") || invalid_file_name_segment(name) {
        return Err(format!("The name '{name}' is not valid."));
    }
    let parent = path
        .parent()
        .ok_or_else(|| "The workspace root cannot be renamed.".to_owned())?;
    let target = parent.join(name);
    if target == path {
        return Ok(target);
    }
    if target.exists() {
        return Err(format!("{} already exists.", target.display()));
    }
    Ok(target)
}

fn apply_renamed_directory_state(app: &mut AppState, old_path: &Path, new_path: &Path) {
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
        && app.workspace_folders.iter().any(|root| parent.starts_with(root))
    {
        app.dir_entries
            .insert(parent.to_string_lossy().into_owned(), read_entries(parent));
    }
    if was_expanded || app.workspace_folders.iter().any(|root| root == new_path) {
        let key = new_path.to_string_lossy().into_owned();
        app.dir_entries.insert(key.clone(), read_entries(new_path));
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

pub fn delete_directory(state: &State<AppState>, path: PathBuf) {
    if state.get().workspace.has_dirty_paths_under(&path) {
        state.update(|app| app.show_toast("Save or close modified files in this folder first."));
        return;
    }
    match move_directory_to_trash(&path) {
        Ok(false) => {}
        Ok(true) => state.update(move |app| apply_deleted_directory_state(app, &path)),
        Err(error) => state.update(move |app| app.show_toast(error)),
    }
}

#[cfg(target_os = "windows")]
fn move_directory_to_trash(path: &Path) -> Result<bool, String> {
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
            "Could not move the folder to the Recycle Bin (shell error {result})."
        ));
    }
    Ok(operation.fAnyOperationsAborted == 0)
}

#[cfg(target_os = "macos")]
fn move_directory_to_trash(path: &Path) -> Result<bool, String> {
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
        Err("Could not move the folder to Trash.".to_owned())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn move_directory_to_trash(path: &Path) -> Result<bool, String> {
    let status = std::process::Command::new("gio")
        .args(["trash", "--"])
        .arg(path)
        .status()
        .map_err(|error| format!("Could not open Trash: {error}"))?;
    if status.success() {
        Ok(true)
    } else {
        Err("Could not move the folder to Trash.".to_owned())
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

fn apply_deleted_directory_state(app: &mut AppState, path: &Path) {
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
        && app.workspace_folders.iter().any(|root| parent.starts_with(root))
    {
        app.dir_entries
            .insert(parent.to_string_lossy().into_owned(), read_entries(parent));
    }
    app.tree_hovered_path = None;
    app.explorer_create = None;
    app.explorer_rename = None;
    app.explorer_create_input.clear();
    app.explorer_create_error = None;
    app.show_toast("Moved folder to Trash.");
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
            app.show_toast(format!("Removed {display} from workspace."));
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
                app.show_toast(message);
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
            app.show_toast(message);
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
        .insert(key.clone(), read_entries(&directory));
    app.expanded.insert(key);
    if let Ok(relative) = refresh_until.strip_prefix(root) {
        for component in relative.components() {
            directory.push(component.as_os_str());
            let key = directory.to_string_lossy().into_owned();
            app.dir_entries
                .insert(key.clone(), read_entries(&directory));
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
            app.show_toast(format!(
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
    let entries = read_entries(&path);
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
    let entries = read_entries(&path);
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

/// Populate directory caches for folders restored from the previous session.
pub(crate) fn hydrate_workspace_folders(app: &mut AppState) {
    app.workspace_folders = existing_unique(std::mem::take(&mut app.workspace_folders));
    app.recent_folders = existing_unique(std::mem::take(&mut app.recent_folders));
    app.recent_folders.truncate(MAX_RECENT_FOLDERS);

    for path in &app.workspace_folders {
        let key = path.to_string_lossy().into_owned();
        app.dir_entries.insert(key.clone(), read_entries(path));
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
        app.show_toast(format!("Could not remember workspace folders: {error}"));
    }
}

fn read_entries(dir: &Path) -> Vec<DirEntry> {
    let mut entries = match fs::read_dir(dir) {
        Ok(iter) => iter
            .flatten()
            .map(|entry| DirEntry {
                is_dir: entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false),
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path(),
            })
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    entries
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
        assert!(app.dir_entries.contains_key(&kept.to_string_lossy().into_owned()));
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

        apply_renamed_directory_state(&mut app, &old_path, &new_path);

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
