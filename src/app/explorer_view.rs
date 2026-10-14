//! Explorer view composition: projects the Explorer panel out of the root
//! state, supplies per-frame inputs, and runs the view's semantic commands.
//! Cross-feature effects (editor drags, terminals, clipboard, drawer chrome)
//! stay here so the view never sees the root state.

use std::path::Path;
use std::sync::Arc;

use lgui::application::ApplicationContext;
use lgui::core::UiFocusHandle;
use lgui::prelude::{Element, State, UiRect};
use lgui::services::ServicesContextExt;

#[cfg(test)]
use crate::app::terminal_binding::TerminalTabs;
use crate::features::explorer::{ExplorerContextTarget, ExplorerCreateKind, ExplorerTargetKind};
use crate::features::git::{GitStoreSnapshot, PathDecoration};
use crate::state::{AppState, FileDragSource, FileDragState};
use crate::ui::components::state_access::StateAccess;
use crate::ui::drawer::{self, DrawerMode};
use crate::ui::explorer::commands::{
    Command, Commands, Decoration, EditInput, Frame, GitTone, MenuAction,
};
use crate::ui::explorer::state::ExplorerPanelState;

/// Services the command handler needs besides the root state.
#[derive(Clone)]
pub(crate) struct Services {
    pub application: ApplicationContext,
    pub editor_focus: UiFocusHandle,
    pub terminal_tabs: crate::app::terminal_binding::TerminalStore,
}

fn projection(state: &State<AppState>) -> StateAccess<ExplorerPanelState> {
    StateAccess::project(state.clone(), |app| &app.explorer, |app| &mut app.explorer)
}

fn commands(state: &State<AppState>, services: &Services) -> Commands {
    let state = state.clone();
    let services = services.clone();
    Commands::new(move |command| run(&state, &services, command))
}

/// The Explorer/Search drawer header, shared with the Search view.
pub(crate) fn drawer_header(rect: UiRect, state: &State<AppState>) -> Element {
    use crate::input::action::Action;
    let mode = state.get().drawer.mode;
    let select_state = state.clone();
    let collapse_state = state.clone();
    drawer::header(
        rect,
        mode,
        Arc::new(move |mode| {
            select_state.update(|app| {
                app.apply(match mode {
                    DrawerMode::Files => Action::OpenExplorer,
                    DrawerMode::Search => Action::OpenSearch,
                })
            })
        }),
        Arc::new(move || {
            collapse_state.update(|app| {
                app.drawer.visible = false;
                app.explorer.ui.hovered = false;
                app.explorer.ui.hovered_path = None;
                app.search.blur();
            })
        }),
    )
}

/// Source-control decoration of a tree row, from the current Git snapshot.
fn decoration(git: &GitStoreSnapshot, path: &Path) -> Decoration {
    Decoration {
        tone: match git.decoration_for_absolute_path(path) {
            Some(PathDecoration::Conflict | PathDecoration::Deleted) => Some(GitTone::Deleted),
            Some(PathDecoration::Modified) => Some(GitTone::Modified),
            Some(PathDecoration::Created) => Some(GitTone::Added),
            Some(PathDecoration::Ignored) => Some(GitTone::Ignored),
            None => None,
        },
        indicator: git
            .repository_for_path(path)
            .and_then(|repository| repository.state_for_absolute_path(path))
            .map(|state| state.display_kind().indicator())
            .unwrap_or_default(),
    }
}

pub(crate) fn render(
    rect: UiRect,
    state: &State<AppState>,
    git_store: &State<GitStoreSnapshot>,
    services: &Services,
    input: EditInput,
) -> Element {
    let app = state.get();
    let git = git_store.get();
    let decorate = |path: &Path| decoration(&git, path);
    let frame = Frame {
        active_path: app.workspace.active_path(),
        resizing: app.drawer.resizing,
        resizable: app.application_layout.is_none(),
        decorate: &decorate,
    };
    let header = drawer_header(
        UiRect::new(
            rect.left,
            rect.top,
            rect.right,
            rect.top + crate::theme::TABS_H,
        ),
        state,
    );
    crate::ui::explorer::view::render(
        rect,
        projection(state),
        &frame,
        commands(state, services),
        header,
        input,
    )
}

pub(crate) fn render_menu(
    viewport: UiRect,
    position: (f32, f32),
    state: &State<AppState>,
    services: &Services,
) -> Element {
    let app = state.get();
    crate::ui::explorer::context_menu::render(
        viewport,
        position,
        projection(state),
        app.workspace.active_path(),
        commands(state, services),
    )
}

fn run(state: &State<AppState>, services: &Services, command: Command) {
    use super::explorer;
    match command {
        Command::ToggleDirectory(key) => {
            state.update(move |app| explorer::toggle_directory(app, &key));
        }
        Command::OpenMenu { point, target } => state.update(move |app| {
            app.editor.tab_context_menu = None;
            app.explorer.ui.menu = Some(point);
            app.explorer.ui.menu_target = target;
            app.explorer.ui.menu_hover = None;
        }),
        Command::PressFile { path, point } => state.update(move |app| {
            let clicks = app.editor.tree_click_count(&path, point.0, point.1);
            app.editor.file_drag = Some(FileDragState {
                path,
                source: FileDragSource::Explorer { clicks },
                origin: point,
                point,
                active: false,
                drop: None,
            });
        }),
        Command::OpenPinned(path) => {
            state.update(move |app| match app.workspace.file_id_for_path(&path) {
                Some(id) => {
                    app.workspace.set_active(id);
                    app.workspace.promote_active_preview();
                }
                None => {
                    explorer::open_tree_file(app, path, 2);
                }
            });
            services.editor_focus.focus();
        }
        Command::ChooseFolder => {
            explorer::choose_folder(state);
        }
        Command::CloneRepository => state.update(|app| {
            app.git.ui.clone_visible = true;
            app.git.ui.clone_error = None;
        }),
        Command::SubmitEdit => {
            if state.get().explorer.model.create_request().is_some() {
                explorer::finish_create(state);
            } else {
                explorer::finish_rename(state);
            }
        }
        Command::CancelEdit => explorer::cancel_edit(state),
        Command::ResizeStart => state.update(|app| app.drawer.resizing = true),
        Command::Resize(width) => state.update(move |app| {
            if app.drawer.resizing {
                app.drawer.width = width;
            }
        }),
        Command::ResizeEnd => state.update(|app| app.drawer.resizing = false),
        Command::Menu { action, target } => run_menu(state, services, action, target),
    }
}

fn run_menu(
    state: &State<AppState>,
    services: &Services,
    action: MenuAction,
    target: Option<ExplorerContextTarget>,
) {
    use super::explorer;
    if action == MenuAction::AddFolder {
        explorer::choose_folder_to_add(state);
        return;
    }
    let Some(target) = target else {
        return;
    };
    match action {
        MenuAction::NewFile => explorer::begin_create(state, target.path, ExplorerCreateKind::File),
        MenuAction::NewFolder => {
            explorer::begin_create(state, target.path, ExplorerCreateKind::Folder)
        }
        MenuAction::RefreshExplorer => state.update(|app| {
            explorer::refresh(app);
            app.workspace.reconcile_disk();
        }),
        MenuAction::OpenTerminal => {
            let cwd = match target.kind {
                ExplorerTargetKind::File => target.path.parent().map(Path::to_path_buf),
                ExplorerTargetKind::Directory => Some(target.path),
            };
            if let Some(cwd) = cwd {
                let shell = state.get().effective_shell_at(Some(&cwd));
                services.terminal_tabs.update(move |tabs| {
                    tabs.add_at(shell, Some(cwd));
                });
                state.update(|app| {
                    app.show_terminal = true;
                    app.terminal_shell_menu = false;
                });
            }
        }
        MenuAction::Cut | MenuAction::Copy => {
            explorer::copy_item(state, &target.path, action == MenuAction::Cut)
        }
        MenuAction::Rename => explorer::begin_rename(state, target.path, target.kind),
        MenuAction::Delete => explorer::delete_item(state, target.path),
        MenuAction::RemoveFolder => explorer::remove_root(state, target.path),
        MenuAction::RevealInFileExplorer => explorer::reveal_item(state, &target),
        MenuAction::CopyPath | MenuAction::CopyRelativePath => explorer::copy_path(
            state,
            services.application.clipboard().as_ref(),
            &target.path,
            action == MenuAction::CopyRelativePath,
        ),
        MenuAction::AddFolder => unreachable!("handled before the target check"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use lgui::application::AppView;
    use lgui::core::{UiId, UiScale};
    use lgui::prelude::panel;
    use lgui::session::UiSession;

    use super::*;
    use crate::state::TabContextMenuState;
    use crate::workspace_actions::close_test_directory;

    struct Harness {
        state: State<AppState>,
        services: Services,
        session: UiSession,
        view: AppView,
    }

    const VIEWPORT: UiRect = UiRect::new(0.0, 0.0, 800.0, 600.0);

    /// A live root state plus the services the Explorer commands use. Each
    /// frame renders the real drawer and, when open, its context menu.
    fn harness(app: AppState) -> Harness {
        let initial = Arc::new(Mutex::new(Some(app)));
        let exposed = Arc::new(Mutex::new(None));
        let output = exposed.clone();
        let context = ApplicationContext::empty(Default::default());
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state_with(|| initial.lock().unwrap().take().unwrap());
            let git = cx.state(GitStoreSnapshot::default());
            let tabs = crate::app::terminal_binding::test_store(
                cx,
                Some(state.clone()),
                TerminalTabs::new,
            );
            let id = UiId::new("explorer-edit-input");
            let services = Services {
                application: context.clone(),
                editor_focus: cx.focus_handle(UiId::new("editor")),
                terminal_tabs: tabs,
            };
            let drawer = render(
                UiRect::new(500.0, 0.0, 800.0, 600.0),
                &state,
                &git,
                &services,
                EditInput {
                    id: id.clone(),
                    focus: cx.focus_handle(id),
                },
            );
            let mut root = panel(VIEWPORT, Default::default()).child(drawer);
            if let Some(position) = state.get().explorer.ui.menu {
                root = root.child(render_menu(VIEWPORT, position, &state, &services));
            }
            *output.lock().unwrap() = Some((state, services));
            root
        });
        let mut session = UiSession::new();
        session.render_view(&view, VIEWPORT, UiScale::ONE);
        let (state, services) = exposed.lock().unwrap().take().unwrap();
        Harness {
            state,
            services,
            session,
            view,
        }
    }

    impl Harness {
        fn run(&self, command: Command) {
            run(&self.state, &self.services, command);
        }

        fn render(&mut self) {
            self.session.render_view(&self.view, VIEWPORT, UiScale::ONE);
        }
    }

    fn workspace(name: &str, files: &[&str]) -> (PathBuf, AppState) {
        let root = close_test_directory(name);
        for file in files {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
        let mut app = AppState::new();
        crate::app::explorer::add_folder(&mut app, root);
        let root = app.explorer.model.roots()[0].clone();
        (root, app)
    }

    fn target(path: PathBuf, kind: ExplorerTargetKind) -> Option<ExplorerContextTarget> {
        Some(ExplorerContextTarget { path, kind })
    }

    #[test]
    fn opening_the_explorer_menu_replaces_the_tab_menu() {
        let (root, mut app) = workspace("explorer-view-menu", &["a.rs"]);
        let id = app.workspace.open_path(root.join("a.rs"), String::new());
        app.editor.tab_context_menu = Some(TabContextMenuState {
            position: (1.0, 1.0),
            pane: app.workspace.active_pane(),
            target: id,
            hovered: None,
        });
        let mut h = harness(app);
        let menu_target = target(root.join("a.rs"), ExplorerTargetKind::File);

        h.run(Command::OpenMenu {
            point: (600.0, 40.0),
            target: menu_target.clone(),
        });
        h.render();

        let app = h.state.get();
        assert!(app.editor.tab_context_menu.is_none());
        assert_eq!(app.explorer.ui.menu, Some((600.0, 40.0)));
        assert_eq!(app.explorer.ui.menu_target, menu_target);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pressing_a_file_twice_starts_a_double_click_drag() {
        let (root, app) = workspace("explorer-view-press", &["a.rs"]);
        let h = harness(app);
        let path = root.join("a.rs");

        for _ in 0..2 {
            h.run(Command::PressFile {
                path: path.clone(),
                point: (520.0, 50.0),
            });
        }

        let drag = h.state.get().editor.file_drag.clone().unwrap();
        assert_eq!(drag.path, path);
        assert!(matches!(
            drag.source,
            FileDragSource::Explorer { clicks: 2 }
        ));
        assert!(!drag.active);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn submit_goes_to_whichever_edit_row_is_open() {
        let (root, app) = workspace("explorer-view-submit", &["old.rs"]);
        let h = harness(app);

        h.run(Command::Menu {
            action: MenuAction::Rename,
            target: target(root.join("old.rs"), ExplorerTargetKind::File),
        });
        h.state
            .update(|app| app.explorer.ui.create_input.set_text("new.rs".to_owned()));
        h.run(Command::SubmitEdit);
        assert!(root.join("new.rs").is_file());

        h.run(Command::Menu {
            action: MenuAction::NewFolder,
            target: target(root.clone(), ExplorerTargetKind::Directory),
        });
        h.state
            .update(|app| app.explorer.ui.create_input.set_text("docs".to_owned()));
        h.run(Command::SubmitEdit);
        assert!(root.join("docs").is_dir());
        assert!(!h.state.get().explorer.model.editing());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn drawer_width_changes_only_during_a_resize_drag() {
        let (root, app) = workspace("explorer-view-resize", &[]);
        let h = harness(app);
        let before = h.state.get().drawer.width;

        h.run(Command::Resize(400.0));
        assert_eq!(h.state.get().drawer.width, before);
        h.run(Command::ResizeStart);
        h.run(Command::Resize(400.0));
        h.run(Command::ResizeEnd);
        h.run(Command::Resize(300.0));

        let app = h.state.get();
        assert_eq!(app.drawer.width, 400.0);
        assert!(!app.drawer.resizing);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn menu_commands_need_a_target_except_add_folder() {
        let (root, app) = workspace("explorer-view-targets", &[]);
        let h = harness(app);

        h.run(Command::Menu {
            action: MenuAction::RemoveFolder,
            target: None,
        });
        assert_eq!(
            h.state.get().explorer.model.roots(),
            std::slice::from_ref(&root)
        );

        h.run(Command::Menu {
            action: MenuAction::RemoveFolder,
            target: target(root.clone(), ExplorerTargetKind::Directory),
        });
        assert!(h.state.get().explorer.model.roots().is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn toggling_and_collapsing_render_from_the_projection() {
        let (root, app) = workspace("explorer-view-toggle", &["src/main.rs"]);
        let mut h = harness(app);
        let src = crate::features::explorer::tree::key(&root.join("src"));

        h.run(Command::ToggleDirectory(src.clone()));
        h.render();
        assert!(h.state.get().explorer.model.is_expanded(&src));
        h.state.update(|app| app.explorer.ui.hovered = true);
        h.run(Command::OpenMenu {
            point: (600.0, 40.0),
            target: None,
        });
        h.render();

        h.run(Command::ToggleDirectory(src.clone()));
        h.render();
        assert!(!h.state.get().explorer.model.is_expanded(&src));
        let _ = fs::remove_dir_all(root);
    }
}
