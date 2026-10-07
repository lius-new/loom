//! Persistent file-tree session and recent-workspace history.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::terminal_session::ShellKind;

pub const MAX_RECENT_FOLDERS: usize = 8;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct WindowGeometry {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub maximized: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct WorkspaceSession {
    #[serde(default)]
    pub open_folders: Vec<PathBuf>,
    #[serde(default)]
    pub recent_folders: Vec<PathBuf>,
    #[serde(default)]
    pub window: Option<WindowGeometry>,
    #[serde(default)]
    pub open_files: Vec<PathBuf>,
    #[serde(default)]
    pub active_file: Option<PathBuf>,
    #[serde(default)]
    pub terminal_tabs: Vec<ShellKind>,
    #[serde(default)]
    pub active_terminal: Option<usize>,
    #[serde(default)]
    pub show_terminal: bool,
    #[serde(default)]
    pub terminal_height: Option<f32>,
    #[serde(default)]
    pub source_control_open: bool,
}

pub fn load() -> WorkspaceSession {
    let _guard = persistence_lock()
        .lock()
        .expect("persistence lock poisoned");
    load_unlocked()
}

fn load_unlocked() -> WorkspaceSession {
    let Some(path) = session_path() else {
        return WorkspaceSession::default();
    };
    let Ok(contents) = fs::read_to_string(path) else {
        return WorkspaceSession::default();
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

pub fn save(open_folders: &[PathBuf], recent_folders: &[PathBuf]) -> io::Result<()> {
    update(|session| {
        session.open_folders = open_folders.to_vec();
        session.recent_folders = recent_folders.to_vec();
    })
}

pub fn save_window_geometry(geometry: WindowGeometry) -> io::Result<()> {
    update(|session| session.window = Some(geometry))
}

pub fn save_file_tabs(open_files: &[PathBuf], active_file: Option<&Path>) -> io::Result<()> {
    update(|session| {
        session.open_files = open_files.to_vec();
        session.active_file = active_file.map(Path::to_path_buf);
    })
}

pub fn save_terminal_state(
    terminal_tabs: &[ShellKind],
    active_terminal: Option<usize>,
    show_terminal: bool,
    terminal_height: f32,
) -> io::Result<()> {
    update(|session| {
        session.terminal_tabs = terminal_tabs.to_vec();
        session.active_terminal = active_terminal;
        session.show_terminal = show_terminal;
        session.terminal_height = Some(terminal_height);
    })
}

pub fn save_source_control_open(source_control_open: bool) -> io::Result<()> {
    update(|session| session.source_control_open = source_control_open)
}

fn update(change: impl FnOnce(&mut WorkspaceSession)) -> io::Result<()> {
    let _guard = persistence_lock()
        .lock()
        .expect("persistence lock poisoned");
    let path = session_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "could not determine the user configuration directory",
        )
    })?;
    let mut session = load_unlocked();
    change(&mut session);
    save_to(&path, &session)
}

fn save_to(path: &Path, session: &WorkspaceSession) -> io::Result<()> {
    write_json(path, session)
}

/// Pretty-print `value` to `path`, creating the parent directory as needed.
pub(crate) fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    fs::write(path, contents)
}

fn persistence_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) fn session_path() -> Option<PathBuf> {
    config_path("workspace.json")
}

/// Path of `file_name` inside Loom's per-user configuration directory.
pub(crate) fn config_path(file_name: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("Loom").join(file_name))
    }

    #[cfg(target_os = "macos")]
    {
        env::var_os("HOME").map(PathBuf::from).map(|path| {
            path.join("Library")
                .join("Application Support")
                .join("Loom")
                .join(file_name)
        })
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(path).join("loom").join(file_name));
        }
        env::var_os("HOME")
            .map(PathBuf::from)
            .map(|path| path.join(".config").join("loom").join(file_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_session_round_trips_as_json() {
        let session = WorkspaceSession {
            open_folders: vec![PathBuf::from("one"), PathBuf::from("two")],
            recent_folders: vec![PathBuf::from("two"), PathBuf::from("one")],
            window: Some(WindowGeometry {
                x: Some(120),
                y: Some(80),
                width: 900.0,
                height: 600.0,
                maximized: false,
            }),
            open_files: vec![PathBuf::from("one/main.rs"), PathBuf::from("two/lib.rs")],
            active_file: Some(PathBuf::from("one/main.rs")),
            terminal_tabs: vec![ShellKind::PowerShell, ShellKind::Bash],
            active_terminal: Some(1),
            show_terminal: true,
            terminal_height: Some(320.0),
            source_control_open: true,
        };

        let json = serde_json::to_string(&session).unwrap();
        let decoded: WorkspaceSession = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, session);
    }

    #[test]
    fn missing_fields_from_older_sessions_use_empty_lists() {
        let decoded: WorkspaceSession = serde_json::from_str("{}").unwrap();

        assert_eq!(decoded, WorkspaceSession::default());
    }
}
