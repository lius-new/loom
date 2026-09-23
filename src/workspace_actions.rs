//! Shared workspace commands used by the welcome screen and editor chrome.

use std::fs;
use std::path::{Path, PathBuf};

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
