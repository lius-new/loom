//! Narrow chrome value frames and semantic command ports.
use std::path::PathBuf;
use std::sync::Arc;
#[derive(Clone)]
pub struct HomeFrame {
    pub paths: Vec<PathBuf>,
    pub hover: Option<usize>,
}
pub enum HomeCommand {
    Hover(Option<usize>),
    Focus(bool),
    OpenFile,
    OpenFolder,
    OpenRecent(PathBuf),
    ToggleTerminal,
    OpenSourceControl,
}
#[derive(Clone)]
pub struct HomeBinding {
    read: Arc<dyn Fn() -> HomeFrame + Send + Sync>,
    run: Arc<dyn Fn(HomeCommand) -> bool + Send + Sync>,
}
impl HomeBinding {
    pub fn new(
        read: impl Fn() -> HomeFrame + Send + Sync + 'static,
        run: impl Fn(HomeCommand) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            read: Arc::new(read),
            run: Arc::new(run),
        }
    }
    pub fn frame(&self) -> HomeFrame {
        (self.read)()
    }
    pub fn run(&self, c: HomeCommand) -> bool {
        (self.run)(c)
    }
}
#[derive(Clone)]
pub struct TitleFrame {
    pub settings_open: bool,
    pub maximized: bool,
    pub update: Option<crate::features::update::Indicator>,
}
#[derive(Clone)]
pub struct TitleBinding {
    read: Arc<dyn Fn() -> TitleFrame + Send + Sync>,
    open: Arc<dyn Fn() + Send + Sync>,
    update: Arc<dyn Fn() -> bool + Send + Sync>,
}
impl TitleBinding {
    pub fn new(
        read: impl Fn() -> TitleFrame + Send + Sync + 'static,
        open: impl Fn() + Send + Sync + 'static,
        update: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            read: Arc::new(read),
            open: Arc::new(open),
            update: Arc::new(update),
        }
    }
    pub fn frame(&self) -> TitleFrame {
        (self.read)()
    }
    pub fn open_settings(&self) {
        (self.open)();
    }
    /// Run the update button's action; true asks to close the window so the
    /// installer can run.
    pub fn activate_update(&self) -> bool {
        (self.update)()
    }
}

#[derive(Clone)]
pub struct FileStatus {
    pub lang: crate::domain::document::ResolvedLanguage,
    pub missing: bool,
    pub conflict: bool,
    pub cursor: Option<(usize, usize)>,
    pub is_diff: bool,
    pub is_file: bool,
}
#[derive(Clone)]
pub struct StatusFrame {
    pub terminal_open: bool,
    pub pending: Vec<crate::features::keymap::keystroke::Keystroke>,
    pub file: Option<FileStatus>,
    pub source_control: bool,
    pub diff_loading: bool,
    pub vim: Option<(loom_editor_core::vim::Mode, loom_editor_core::vim::Status)>,
}
pub enum StatusCommand {
    ToggleTerminal,
    ToggleSourceControl,
}
#[derive(Clone)]
pub struct StatusBinding {
    read: Arc<dyn Fn() -> StatusFrame + Send + Sync>,
    run: Arc<dyn Fn(StatusCommand) + Send + Sync>,
}
impl StatusBinding {
    pub fn new(
        read: impl Fn() -> StatusFrame + Send + Sync + 'static,
        run: impl Fn(StatusCommand) + Send + Sync + 'static,
    ) -> Self {
        Self {
            read: Arc::new(read),
            run: Arc::new(run),
        }
    }
    pub fn frame(&self) -> StatusFrame {
        (self.read)()
    }
    pub fn run(&self, c: StatusCommand) {
        (self.run)(c);
    }
}
pub enum CloseCommand {
    Save,
    Discard,
    Cancel,
}
#[derive(Clone, Copy)]
pub struct CloseOutcome {
    pub exit: bool,
    pub resolved: bool,
}
#[derive(Clone)]
pub struct CloseBinding {
    read: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    run: Arc<dyn Fn(CloseCommand) -> CloseOutcome + Send + Sync>,
}
impl CloseBinding {
    pub fn new(
        read: impl Fn() -> Option<String> + Send + Sync + 'static,
        run: impl Fn(CloseCommand) -> CloseOutcome + Send + Sync + 'static,
    ) -> Self {
        Self {
            read: Arc::new(read),
            run: Arc::new(run),
        }
    }
    pub fn message(&self) -> Option<String> {
        (self.read)()
    }
    pub fn run(&self, c: CloseCommand) -> CloseOutcome {
        (self.run)(c)
    }
}

pub(crate) mod menu_binding;
pub(crate) mod sash_binding;

pub(crate) mod tab_state;
pub(crate) mod tabs_binding;

pub mod diff_binding;
