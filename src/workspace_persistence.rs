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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
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
    #[serde(default)]
    pub git_tree_view: bool,
    #[serde(default)]
    pub git_split_diff: bool,
    #[serde(default)]
    pub git_inline_blame: bool,
    #[serde(default)]
    pub default_shell: ShellKind,
    #[serde(default = "default_true")]
    pub terminal_cursor_blink: bool,
}

impl Default for WorkspaceSession {
    fn default() -> Self {
        Self {
            open_folders: Vec::new(),
            recent_folders: Vec::new(),
            window: None,
            open_files: Vec::new(),
            active_file: None,
            terminal_tabs: Vec::new(),
            active_terminal: None,
            show_terminal: false,
            terminal_height: None,
            source_control_open: false,
            git_tree_view: false,
            git_split_diff: false,
            git_inline_blame: false,
            default_shell: ShellKind::default(),
            terminal_cursor_blink: true,
        }
    }
}

fn default_true() -> bool {
    true
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

pub fn save_git_ui(
    source_control_open: bool,
    git_tree_view: bool,
    git_split_diff: bool,
    git_inline_blame: bool,
) -> io::Result<()> {
    update(|session| {
        session.source_control_open = source_control_open;
        session.git_tree_view = git_tree_view;
        session.git_split_diff = git_split_diff;
        session.git_inline_blame = git_inline_blame;
    })
}

pub fn save_terminal_preferences(
    default_shell: ShellKind,
    terminal_cursor_blink: bool,
) -> io::Result<()> {
    update(|session| {
        session.default_shell = default_shell;
        session.terminal_cursor_blink = terminal_cursor_blink;
    })
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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_vec_pretty(session).map_err(io::Error::other)?;
    fs::write(path, contents)
}

fn persistence_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn session_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("Loom").join("workspace.json"))
    }

    #[cfg(target_os = "macos")]
    {
        env::var_os("HOME").map(PathBuf::from).map(|path| {
            path.join("Library")
                .join("Application Support")
                .join("Loom")
                .join("workspace.json")
        })
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(path).join("loom").join("workspace.json"));
        }
        env::var_os("HOME")
            .map(PathBuf::from)
            .map(|path| path.join(".config").join("loom").join("workspace.json"))
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
            git_tree_view: true,
            git_split_diff: true,
            git_inline_blame: true,
            default_shell: ShellKind::Bash,
            terminal_cursor_blink: false,
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
