//! Persistent file-tree session and recent-workspace history.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::features::terminal::ShellKind;

use crate::infrastructure::session_filesystem as filesystem;
pub(crate) use filesystem::session_path;
#[cfg(test)]
pub(crate) use filesystem::write_json;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct WindowGeometry {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub maximized: bool,
}

pub use crate::domain::workspace_session::EditorLayout;
#[cfg(test)]
pub use crate::domain::workspace_session::{SavedAxis, SavedChild, SavedPaneNode};
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct WorkspaceSession {
    #[serde(default)]
    pub open_folders: Vec<PathBuf>,
    #[serde(default)]
    pub recent_folders: Vec<PathBuf>,
    #[serde(default)]
    pub window: Option<WindowGeometry>,
    /// Editor panes and their restorable file tabs.
    #[serde(default)]
    pub editor_layout: Option<EditorLayout>,
    #[serde(default)]
    pub terminal_tabs: Vec<ShellKind>,
    #[serde(default)]
    pub terminal_groups: Vec<usize>,
    #[serde(default)]
    pub terminal_widths: Vec<f32>,
    #[serde(default)]
    pub terminal_purposes: Vec<Option<String>>,
    #[serde(default)]
    pub terminal_directories: Vec<Option<PathBuf>>,
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
    session_path()
        .and_then(|path| filesystem::read_json(&path))
        .unwrap_or_default()
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

pub fn save_editor_layout(layout: Option<&EditorLayout>) -> io::Result<()> {
    update(|session| session.editor_layout = layout.cloned())
}

pub fn save_terminal_state(
    terminal_tabs: &[ShellKind],
    terminal_groups: &[usize],
    terminal_widths: &[f32],
    terminal_purposes: &[Option<String>],
    terminal_directories: &[Option<PathBuf>],
    active_terminal: Option<usize>,
    show_terminal: bool,
    terminal_height: f32,
) -> io::Result<()> {
    update(|session| {
        session.terminal_tabs = terminal_tabs.to_vec();
        session.terminal_groups = terminal_groups.to_vec();
        session.terminal_widths = terminal_widths.to_vec();
        session.terminal_purposes = terminal_purposes.to_vec();
        session.terminal_directories = terminal_directories.to_vec();
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
    filesystem::write_session(path, session)
}

fn persistence_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

use super::state::AppState;
use super::workspace::Workspace;
use crate::domain::identity::FileId;
use crate::domain::workspace_session::{self as editor_session, PendingTab};
use lgui::prelude::State;

pub(crate) fn saved_editor_layout(workspace: &Workspace) -> Option<EditorLayout> {
    saved_layout_with_pending(workspace, None)
}

pub(crate) fn saved_app_editor_layout(app: &AppState) -> Option<EditorLayout> {
    saved_layout_with_pending(&app.workspace, app.editor.pending_file.as_ref())
}

pub(crate) fn save_editor_session(state: &State<AppState>) -> bool {
    let layout = saved_app_editor_layout(&state.get());
    let result = save_editor_layout(layout.as_ref());
    if let Err(error) = result {
        state.update(|app| app.show_error(format!("Could not remember file tabs: {error}")));
        return false;
    }
    true
}

pub(crate) fn hydrate_editor_layout(app: &mut AppState, layout: Option<EditorLayout>) {
    restore_editor_layout_with(app, layout, |workspace, path| {
        Some(workspace.open_deferred(path))
    });
}

#[cfg(test)]
pub(crate) fn hydrate_editor_layout_with(
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
    let shape = editor_session::restored_shape(layout.root, &mut contents);
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
fn saved_layout_with_pending(
    workspace: &Workspace,
    pending: Option<&crate::file_loading::PendingFile>,
) -> Option<EditorLayout> {
    let pending = pending.map(|value| PendingTab {
        pane: value.pane,
        path: value.path.clone(),
        insert_before: match value.drop {
            Some(crate::app::state::TabDrop::Insert { index, .. }) => Some(index),
            _ => None,
        },
    });
    editor_session::capture(&workspace.core, pending.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, fs};

    #[test]
    fn session_replacement_preserves_other_fields_and_is_readable_after_each_save() {
        let root = env::temp_dir().join(format!("loom-session-atomic-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("workspace.json");
        let mut session = WorkspaceSession {
            recent_folders: vec!["remember-me".into()],
            show_terminal: true,
            ..Default::default()
        };
        save_to(&path, &session).unwrap();
        session.editor_layout = Some(EditorLayout {
            root: SavedPaneNode::Pane {
                items: vec!["single-click.json".into()],
                active: Some("single-click.json".into()),
            },
            active_pane: 0,
        });
        save_to(&path, &session).unwrap();
        let decoded: WorkspaceSession = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(decoded, session);
        assert_eq!(
            fs::read_dir(&root).unwrap().count(),
            1,
            "no temporary save should remain"
        );
        fs::remove_dir_all(root).unwrap();
    }

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
            editor_layout: Some(EditorLayout {
                root: SavedPaneNode::Split {
                    axis: SavedAxis::Horizontal,
                    children: vec![
                        SavedChild {
                            size: 0.4,
                            node: SavedPaneNode::Pane {
                                items: vec![PathBuf::from("one/main.rs")],
                                active: Some(PathBuf::from("one/main.rs")),
                            },
                        },
                        SavedChild {
                            size: 0.6,
                            node: SavedPaneNode::Pane {
                                items: vec![PathBuf::from("two/lib.rs")],
                                active: None,
                            },
                        },
                    ],
                },
                active_pane: 1,
            }),
            terminal_tabs: vec![
                ShellKind::PowerShell,
                ShellKind::PowerShell,
                ShellKind::Bash,
            ],
            terminal_groups: vec![0, 0, 1],
            terminal_widths: vec![1.5, 0.5, 1.0],
            terminal_purposes: vec![Some("Testing".into()), None, Some("Development".into())],
            terminal_directories: vec![
                Some(PathBuf::from("one")),
                None,
                Some(PathBuf::from("two")),
            ],
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
