//! Persistent file-tree session and recent-workspace history.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MAX_RECENT_FOLDERS: usize = 8;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct WorkspaceSession {
    #[serde(default)]
    pub open_folders: Vec<PathBuf>,
    #[serde(default)]
    pub recent_folders: Vec<PathBuf>,
}

pub fn load() -> WorkspaceSession {
    let Some(path) = session_path() else {
        return WorkspaceSession::default();
    };
    let Ok(contents) = fs::read_to_string(path) else {
        return WorkspaceSession::default();
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

pub fn save(open_folders: &[PathBuf], recent_folders: &[PathBuf]) -> io::Result<()> {
    let path = session_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "could not determine the user configuration directory",
        )
    })?;
    save_to(&path, open_folders, recent_folders)
}

fn save_to(path: &Path, open_folders: &[PathBuf], recent_folders: &[PathBuf]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let session = WorkspaceSession {
        open_folders: open_folders.to_vec(),
        recent_folders: recent_folders.to_vec(),
    };
    let contents = serde_json::to_vec_pretty(&session).map_err(io::Error::other)?;
    fs::write(path, contents)
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
