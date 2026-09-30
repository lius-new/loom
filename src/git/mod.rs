//! Git integration boundary.
//!
//! UI code never starts `git` directly.  Runtime selection, process policy,
//! parsing and repository state all live below this module.

pub mod backend;
pub mod command;
pub mod diff;
pub mod discovery;
pub mod error;
pub mod parser;
pub mod runtime;
pub mod store;
pub mod types;

pub use backend::CliGitBackend;
pub use error::{GitError, GitResult};
pub use runtime::GitRuntimeManager;
pub use store::{GitService, GitStoreSnapshot};
pub use types::*;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, mpsc};
use std::thread;
use std::time::Duration;

/// Process-wide service.  It contains only thread-safe runtime state; UI state
/// remains in a separate `State<GitStoreSnapshot>`.
pub fn service() -> GitResult<Arc<GitService>> {
    static SERVICE: OnceLock<Result<Arc<GitService>, GitError>> = OnceLock::new();
    SERVICE
        .get_or_init(|| {
            let runtime = GitRuntimeManager::default().resolve()?;
            Ok(Arc::new(GitService::new(CliGitBackend::new(runtime))))
        })
        .clone()
}

/// Resolve the runtime and start status polling entirely off the UI thread.
/// This keeps slow PATH/runtime checks and the first repository scan from
/// delaying the initial frame.
pub fn start_polling_async(
    roots: Vec<PathBuf>,
    active_path: Option<PathBuf>,
    interval: Duration,
    publish: impl Fn(GitStoreSnapshot) + Send + 'static,
) -> AsyncPollingHandle {
    let (stop_sender, stop_receiver) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("loom-git-bootstrap".into())
        .spawn(move || match service() {
            Ok(service) => {
                let polling = service.start_polling(roots, active_path, interval, publish);
                let _ = stop_receiver.recv();
                drop(polling);
            }
            Err(error) => publish(GitStoreSnapshot {
                last_error: Some(error),
                initializing: false,
                ..GitStoreSnapshot::default()
            }),
        })
        .ok();
    AsyncPollingHandle {
        stop_sender: Some(stop_sender),
        worker,
    }
}

pub struct AsyncPollingHandle {
    stop_sender: Option<mpsc::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Drop for AsyncPollingHandle {
    fn drop(&mut self) {
        if let Some(sender) = self.stop_sender.take() {
            let _ = sender.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
