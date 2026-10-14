//! Service ownership belongs to the mounted application, never its snapshots.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::file_loading::FileLoader;
use crate::infrastructure::git::PollControl;
use lgui::ApplicationHandle;

#[derive(Default)]
pub(crate) struct AppRuntime {
    services: Mutex<Services>,
    git_poll: PollControl,
    terminals: Arc<crate::infrastructure::terminal::runtime::TerminalRuntime>,
    terminal_mounts: Arc<crate::app::terminal_effects::Mounts>,
}

#[derive(Default)]
struct Services {
    file_loader: Option<FileLoader>,
    application: Option<ApplicationHandle>,
    search_cancel: Option<Arc<AtomicBool>>,
}

impl AppRuntime {
    pub(crate) fn terminal_mounts(&self) -> &Arc<crate::app::terminal_effects::Mounts> {
        &self.terminal_mounts
    }
    pub(crate) fn terminals(
        &self,
    ) -> &Arc<crate::infrastructure::terminal::runtime::TerminalRuntime> {
        &self.terminals
    }

    pub(crate) fn file_loader(&self) -> Option<FileLoader> {
        self.services.lock().unwrap().file_loader.clone()
    }

    pub(crate) fn install_file_loader(&self, loader: FileLoader) {
        let previous = self.services.lock().unwrap().file_loader.replace(loader);
        if let Some(previous) = previous {
            previous.shutdown();
        }
    }

    pub(crate) fn clear_file_loader(&self) {
        let loader = self.services.lock().unwrap().file_loader.take();
        if let Some(loader) = loader {
            loader.shutdown();
        }
    }

    pub(crate) fn application(&self) -> Option<ApplicationHandle> {
        self.services.lock().unwrap().application.clone()
    }

    pub(crate) fn set_application(&self, application: ApplicationHandle) {
        self.services.lock().unwrap().application = Some(application);
    }

    pub(crate) fn clear_application(&self) {
        self.services.lock().unwrap().application = None;
    }

    pub(crate) fn install_search_cancel(&self, cancel: Arc<AtomicBool>) {
        self.services.lock().unwrap().search_cancel = Some(cancel);
    }

    pub(crate) fn cancel_search(&self) {
        let cancel = self.services.lock().unwrap().search_cancel.clone();
        if let Some(cancel) = cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn git_poll_control(&self) -> PollControl {
        self.git_poll.clone()
    }
}

impl Drop for AppRuntime {
    fn drop(&mut self) {
        self.terminals.shutdown();
        let services = self.services.get_mut().unwrap();
        if let Some(cancel) = services.search_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        if let Some(loader) = services.file_loader.take() {
            loader.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::app::test_support::UiTasks;

    #[test]
    fn snapshots_do_not_keep_runtime_services_alive() {
        let runtime = Arc::new(AppRuntime::default());
        runtime.set_application(UiTasks::default().application());
        let mut state = AppState::new();
        state.attach_runtime(&runtime);
        let snapshot = state.clone();
        assert_eq!(Arc::strong_count(&runtime), 1);
        assert!(snapshot.runtime().unwrap().application().is_some());
        drop(runtime);
        assert!(state.runtime().is_none());
        assert!(snapshot.runtime().is_none());
        assert_eq!(state.settings(), snapshot.settings());
    }

    #[test]
    fn instances_have_separate_dispatchers_and_cleanup_is_idempotent() {
        let a = AppRuntime::default();
        let b = AppRuntime::default();
        let queue = UiTasks::default();
        a.set_application(queue.application());
        assert!(b.application().is_none());
        a.application().unwrap().post(|| {});
        assert_eq!(queue.pending_ids().len(), 1);
        a.clear_application();
        a.clear_application();
        a.clear_file_loader();
        a.clear_file_loader();
        assert!(a.application().is_none());
    }

    #[test]
    fn dropping_snapshots_does_not_cancel_search_but_runtime_teardown_does() {
        let runtime = Arc::new(AppRuntime::default());
        let token = Arc::new(AtomicBool::new(false));
        runtime.install_search_cancel(token.clone());
        let mut state = AppState::new();
        state.attach_runtime(&runtime);
        drop(state.clone());
        assert!(!token.load(Ordering::Relaxed));
        drop(runtime);
        assert!(token.load(Ordering::Relaxed));
        assert!(state.runtime().is_none());
    }
}
