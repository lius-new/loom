//! Shared workspace commands used by the welcome screen and editor chrome.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;

use lgui::dialogs::{FileDialogOptions, system_file_dialogs};
use lgui::prelude::State;

use crate::state::{AppState, DirEntry};
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
}
