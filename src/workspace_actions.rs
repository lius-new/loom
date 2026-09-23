//! Shared workspace commands used by the welcome screen and editor chrome.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

use lgui::dialogs::{FileDialogOptions, system_file_dialogs};
use lgui::prelude::State;

use crate::state::{AppState, DirEntry};

const MAX_EDITABLE_FILE_BYTES: u64 = 4 * 1024 * 1024;

pub fn choose_file(state: &State<AppState>) -> bool {
    let mut options = FileDialogOptions::new().title("Open File");
    if let Some(directory) = state.get().open_dir {
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
    if let Some(directory) = state.get().open_dir {
        options = options.directory(directory);
    }
    let Some(path) = system_file_dialogs().pick_folder(&options) else {
        return false;
    };

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
    if let Some(directory) = state.get().open_dir {
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
        .name("leditor-git-clone".to_owned())
        .spawn(move || {
            let result = Command::new("git")
                .arg("clone")
                .arg("--")
                .arg(&url)
                .arg(&target)
                .output();

            clone_state.update(move |app| {
                app.cloning_repository = false;
                match result {
                    Ok(output) if output.status.success() => {
                        load_folder(app, target);
                        app.show_clone_dialog = false;
                        app.clone_repository_url.clear();
                        app.clone_repository_error = None;
                        app.clone_input_focused = false;
                        app.toast = None;
                    }
                    Ok(output) => {
                        let detail = String::from_utf8_lossy(&output.stderr);
                        app.clone_repository_error = Some(first_error_line(&detail));
                    }
                    Err(error) => {
                        app.clone_repository_error = Some(format!("Could not run git: {error}"));
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
    let key = path.to_string_lossy().into_owned();
    let entries = read_entries(&path);
    app.open_dir = Some(path);
    app.dir_entries.clear();
    app.dir_entries.insert(key.clone(), entries);
    app.expanded.clear();
    app.expanded.insert(key);
    app.tree_scroll = 0.0;
    app.tree_scroll_x = 0.0;
    app.show_drawer = true;
    app.welcome_hover = None;
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
}
