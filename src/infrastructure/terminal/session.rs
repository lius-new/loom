//! PTY controller and worker lifecycle; GUI notifications are injected.
use crate::features::terminal::input::paste_bytes;
use crate::features::terminal::screen::{
    GridPoint, Palette, Selection, line_runs, selection_text, visible_selection,
};
use crate::features::terminal::{ShellKind, TerminalSize, TerminalSnapshot, TerminalStatus};
use crate::infrastructure::terminal::notification::FrameNotifier;
use crate::infrastructure::terminal::pty::{
    RunningProcess, resize_process, spawn_shell_option, terminate_process,
};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::thread;
#[cfg(windows)]
use std::time::Duration;
const SCROLLBACK_ROWS: usize = 2_000;

#[derive(Clone)]
pub struct TerminalController {
    inner: Arc<TerminalInner>,
}

struct TerminalInner {
    lifecycle: Mutex<()>,
    disposed: AtomicBool,
    parser: Mutex<vt100::Parser>,
    process: Mutex<Option<RunningProcess>>,
    shell: Mutex<ShellKind>,
    status: Mutex<TerminalStatus>,
    selection: Mutex<Option<Selection>>,
    mouse: Mutex<crate::features::terminal::mouse::TerminalMouse>,
    generation: AtomicU64,
    cwd: Mutex<Option<PathBuf>>,
    profile: Mutex<Option<crate::features::terminal::profiles::ShellProfile>>,
    notice: Mutex<Option<String>>,
}

impl TerminalController {
    pub fn new() -> Self {
        Self::with_shell(ShellKind::default())
    }

    pub fn with_shell(shell: ShellKind) -> Self {
        Self {
            inner: Arc::new(TerminalInner {
                lifecycle: Mutex::new(()),
                disposed: AtomicBool::new(false),
                parser: Mutex::new(vt100::Parser::new(24, 80, SCROLLBACK_ROWS)),
                process: Mutex::new(None),
                shell: Mutex::new(shell),
                status: Mutex::new(TerminalStatus::Idle),
                selection: Mutex::new(None),
                mouse: Mutex::new(crate::features::terminal::mouse::TerminalMouse::default()),
                generation: AtomicU64::new(0),
                cwd: Mutex::new(None),
                profile: Mutex::new(None),
                notice: Mutex::new(None),
            }),
        }
    }

    pub fn ensure_started(
        &self,
        cwd: Option<&Path>,
        size: TerminalSize,
        application: impl Into<FrameNotifier>,
    ) {
        let application = application.into();
        let should_start = matches!(*lock(&self.inner.status), TerminalStatus::Idle);
        if should_start {
            self.restart(self.shell(), cwd, size, application);
        } else {
            self.resize(size);
        }
    }

    pub fn shell(&self) -> ShellKind {
        *lock(&self.inner.shell)
    }

    pub(crate) fn set_profile(
        &self,
        profile: Option<crate::features::terminal::profiles::ShellProfile>,
    ) {
        *lock(&self.inner.profile) = profile;
    }

    pub fn shell_name(&self) -> String {
        lock(&self.inner.profile)
            .as_ref()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| self.shell().label().into())
    }

    pub fn short_label(&self) -> String {
        lock(&self.inner.profile)
            .as_ref()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| self.shell().short_label().into())
    }

    pub fn working_directory(&self) -> Option<PathBuf> {
        lock(&self.inner.cwd).clone()
    }

    /// Inspect the OS process outside reactive UI state locks.
    pub fn program_candidate(
        &self,
        probe: &crate::infrastructure::terminal::program::ProgramProbe,
        preferred: Option<u32>,
    ) -> Option<crate::features::terminal::program::ProgramCandidate> {
        if !matches!(*lock(&self.inner.status), TerminalStatus::Running) {
            return None;
        }
        #[cfg(windows)]
        let (pid, console) = {
            let process = lock(&self.inner.process);
            let process = process.as_ref()?;
            (process.pid, process.console.clone())
        };
        #[cfg(windows)]
        {
            if let Some(console) = console
                && let Some(members) = console.members()
            {
                return probe.session_candidate(&members, &console.baseline, preferred);
            }
            probe.candidate(pid, None)
        }
        #[cfg(not(windows))]
        {
            let _ = preferred;
            let (pid, foreground) = {
                let process = lock(&self.inner.process);
                let process = process.as_ref()?;
                let foreground = process.master.process_group_leader().map(|pid| pid as u32);
                (process.pid, foreground)
            };
            probe.candidate(pid, foreground)
        }
    }

    pub fn restart(
        &self,
        shell: ShellKind,
        cwd: Option<&Path>,
        size: TerminalSize,
        application: impl Into<FrameNotifier>,
    ) {
        let application = application.into();
        let lifecycle = lock(&self.inner.lifecycle);
        if self.inner.disposed.load(Ordering::Acquire) {
            return;
        }
        let generation = self.inner.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.kill_current();
        *lock(&self.inner.shell) = shell;
        *lock(&self.inner.status) = TerminalStatus::Starting;
        *lock(&self.inner.parser) = vt100::Parser::new(size.rows, size.cols, SCROLLBACK_ROWS);
        *lock(&self.inner.selection) = None;

        let cwd = crate::infrastructure::terminal::integration::starting_directory(cwd, None);
        *lock(&self.inner.mouse) = crate::features::terminal::mouse::TerminalMouse::default();
        *lock(&self.inner.cwd) = cwd.clone();
        let selected = if matches!(shell, ShellKind::Custom(_)) {
            lock(&self.inner.profile)
                .as_ref()
                .map(crate::infrastructure::terminal::profiles::custom)
        } else {
            crate::infrastructure::terminal::profiles::builtin(shell)
        };
        let available = selected
            .clone()
            .filter(crate::features::terminal::profiles::ShellOption::available);
        let launch = available.clone().or_else(|| {
            crate::infrastructure::terminal::profiles::defaults()
                .into_iter()
                .next()
        });
        *lock(&self.inner.notice) = if available.is_none() {
            launch.as_ref().map(|fallback| {
                format!(
                    "{} is unavailable; using {}.",
                    selected
                        .as_ref()
                        .map(|s| s.name.as_str())
                        .unwrap_or(shell.label()),
                    fallback.name
                )
            })
        } else {
            None
        };
        // The tab keeps its configured shell, so a restored session or a
        // restart after the profile is repaired launches that shell again.
        let spawned = launch
            .ok_or_else(|| "No usable shell was found. Add a terminal in Settings.".to_owned())
            .and_then(|launch| spawn_shell_option(&launch, cwd.as_deref(), size));
        match spawned {
            Ok(spawned) => {
                let mut reader = spawned.reader;
                #[cfg(not(windows))]
                let child = spawned.child;
                *lock(&self.inner.process) = Some(spawned.process);
                *lock(&self.inner.status) = TerminalStatus::Running;
                drop(lifecycle);
                application.request_frame();

                let reader_inner = Arc::downgrade(&self.inner);
                let reader_application = application.clone();
                thread::Builder::new()
                    .name("loom-terminal-output".to_owned())
                    .spawn(move || {
                        let mut bytes = [0_u8; 8 * 1024];
                        let mut directories =
                            crate::features::terminal::directory::DirectoryReports::default();
                        loop {
                            let count = match reader.read(&mut bytes) {
                                Ok(count) => count,
                                Err(error) => {
                                    if let Some(inner) = current_inner(&reader_inner, generation) {
                                        *lock(&inner.status) = TerminalStatus::Failed(format!(
                                            "terminal output stream failed: {error}"
                                        ));
                                        reader_application.request_frame();
                                    }
                                    break;
                                }
                            };
                            if count == 0 {
                                break;
                            }
                            let Some(inner) = current_inner(&reader_inner, generation) else {
                                break;
                            };
                            if let Some(cwd) = directories.process(&bytes[..count]) {
                                *lock(&inner.cwd) = Some(cwd);
                            }
                            lock(&inner.parser).process(&bytes[..count]);
                            reader_application.request_frame();
                        }
                    })
                    .expect("terminal output thread should start");

                spawn_waiter(
                    Arc::downgrade(&self.inner),
                    generation,
                    #[cfg(not(windows))]
                    child,
                    application,
                );
            }
            Err(error) => {
                *lock(&self.inner.status) = TerminalStatus::Failed(error);
                drop(lifecycle);
                application.request_frame();
            }
        }
    }

    pub(super) fn dispose(&self) {
        let _lifecycle = lock(&self.inner.lifecycle);
        self.inner.disposed.store(true, Ordering::Release);
        self.shutdown();
    }

    pub fn shutdown(&self) {
        self.inner.generation.fetch_add(1, Ordering::AcqRel);
        self.kill_current();
        *lock(&self.inner.status) = TerminalStatus::Stopped;
    }
    pub fn terminate(&self, application: impl Into<FrameNotifier>) {
        self.shutdown();
        application.into().request_frame();
    }

    pub fn write(&self, bytes: &[u8]) {
        lock(&self.inner.parser).screen_mut().set_scrollback(0);
        // Typing into the shell ends the selection, as in other terminals.
        *lock(&self.inner.selection) = None;
        let mut process = lock(&self.inner.process);
        let Some(process) = process.as_mut() else {
            return;
        };
        if let Err(error) = process
            .writer
            .write_all(bytes)
            .and_then(|_| process.writer.flush())
        {
            *lock(&self.inner.status) = TerminalStatus::Failed(error.to_string());
        }
    }

    pub fn resize(&self, size: TerminalSize) {
        {
            let mut parser = lock(&self.inner.parser);
            if parser.screen().size() != (size.rows, size.cols) {
                let scrollback = parser.screen().scrollback();
                parser.screen_mut().set_size(size.rows, size.cols);
                parser.screen_mut().set_scrollback(scrollback);
            }
        }

        let mut process = lock(&self.inner.process);
        let Some(process) = process.as_mut() else {
            return;
        };
        if process.size != size && resize_process(process, size).is_ok() {
            process.size = size;
        }
    }

    pub fn scroll(&self, rows: i32) {
        let mut parser = lock(&self.inner.parser);
        crate::features::terminal::mouse::scroll_history(parser.screen_mut(), rows);
    }

    pub fn set_mouse_position(&self, row: u16, col: u16) {
        lock(&self.inner.mouse).set_position(row, col);
    }

    pub fn wheel(&self, delta: f32, modifiers: crate::features::terminal::input::Modifiers) {
        let bytes = {
            let mut parser = lock(&self.inner.parser);
            lock(&self.inner.mouse).wheel(parser.screen_mut(), delta, modifiers)
        };
        if !bytes.is_empty() {
            self.write(&bytes);
        }
    }

    /// Writes pasted text, wrapping it for shells that enabled bracketed paste.
    pub fn paste(&self, value: &str) {
        if value.is_empty() {
            return;
        }
        let bracketed = lock(&self.inner.parser).screen().bracketed_paste();
        self.write(&paste_bytes(value, bracketed));
    }

    /// Starts a selection at a visible cell boundary.
    pub fn begin_selection(&self, row: u16, col: u16) {
        let point = self.grid_point(row, col);
        *lock(&self.inner.selection) = Some(Selection {
            anchor: point,
            head: point,
        });
    }

    /// Moves the free end of the selection to a visible cell boundary.
    pub fn extend_selection(&self, row: u16, col: u16) {
        let point = self.grid_point(row, col);
        if let Some(selection) = lock(&self.inner.selection).as_mut() {
            selection.head = point;
        }
    }

    pub fn clear_selection(&self) -> bool {
        lock(&self.inner.selection).take().is_some()
    }

    pub fn selected_text(&self) -> Option<String> {
        let mut parser = lock(&self.inner.parser);
        let (start, end) = (*lock(&self.inner.selection))?.ordered()?;
        Some(selection_text(parser.screen_mut(), start, end))
    }

    fn grid_point(&self, row: u16, col: u16) -> GridPoint {
        let parser = lock(&self.inner.parser);
        let screen = parser.screen();
        let (rows, cols) = screen.size();
        GridPoint {
            row: i64::from(row.min(rows.saturating_sub(1))) - screen.scrollback() as i64,
            col: col.min(cols),
        }
    }

    pub fn snapshot_with_palette(&self, palette: &Palette) -> TerminalSnapshot {
        let parser = lock(&self.inner.parser);
        let screen = parser.screen();
        let (rows, cols) = screen.size();
        let lines = (0..rows)
            .map(|row| line_runs(screen, row, cols, palette))
            .collect();
        let selection = lock(&self.inner.selection)
            .and_then(Selection::ordered)
            .map(|(start, end)| visible_selection(start, end, screen.scrollback(), rows, cols))
            .unwrap_or_default();
        TerminalSnapshot {
            shell: self.shell(),
            shell_name: self.shell_name(),
            notice: lock(&self.inner.notice).clone(),
            status: lock(&self.inner.status).clone(),
            rows,
            cols,
            lines,
            cursor: screen.cursor_position(),
            cursor_visible: !screen.hide_cursor() && screen.scrollback() == 0,
            application_cursor: screen.application_cursor(),
            selection,
        }
    }

    fn kill_current(&self) {
        if let Some(mut process) = lock(&self.inner.process).take() {
            terminate_process(&mut process);
        }
    }
}

impl Default for TerminalController {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TerminalInner {
    fn drop(&mut self) {
        if let Ok(process) = self.process.get_mut()
            && let Some(mut process) = process.take()
        {
            terminate_process(&mut process);
        }
    }
}

#[cfg(windows)]
fn spawn_waiter(inner: Weak<TerminalInner>, generation: u64, application: FrameNotifier) {
    thread::Builder::new()
        .name("loom-terminal-wait".to_owned())
        .spawn(move || {
            loop {
                let Some(inner) = current_inner(&inner, generation) else {
                    return;
                };
                let exit = {
                    let process = lock(&inner.process);
                    let Some(process) = process.as_ref() else {
                        return;
                    };
                    (!process.process.is_alive()).then(|| process.process.wait(Some(0)))
                };
                if let Some(exit) = exit {
                    *lock(&inner.status) = match exit {
                        Ok(code) => TerminalStatus::Exited(code),
                        Err(error) => TerminalStatus::Failed(error.to_string()),
                    };
                    application.request_frame();
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }
        })
        .expect("terminal wait thread should start");
}

#[cfg(not(windows))]
fn spawn_waiter(
    inner: Weak<TerminalInner>,
    generation: u64,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    application: FrameNotifier,
) {
    thread::Builder::new()
        .name("loom-terminal-wait".to_owned())
        .spawn(move || {
            let status = child.wait();
            let Some(inner) = current_inner(&inner, generation) else {
                return;
            };
            *lock(&inner.status) = match status {
                Ok(status) => TerminalStatus::Exited(status.exit_code()),
                Err(error) => TerminalStatus::Failed(error.to_string()),
            };
            application.request_frame();
        })
        .expect("terminal wait thread should start");
}

fn current_inner(inner: &Weak<TerminalInner>, generation: u64) -> Option<Arc<TerminalInner>> {
    let inner = inner.upgrade()?;
    (inner.generation.load(Ordering::Acquire) == generation).then_some(inner)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
impl TerminalController {
    pub(crate) fn test_parser(&self) -> MutexGuard<'_, vt100::Parser> {
        lock(&self.inner.parser)
    }
    pub(crate) fn test_profile(
        &self,
    ) -> MutexGuard<'_, Option<crate::features::terminal::profiles::ShellProfile>> {
        lock(&self.inner.profile)
    }
    pub(crate) fn test_cwd(&self) -> MutexGuard<'_, Option<PathBuf>> {
        lock(&self.inner.cwd)
    }
    pub(crate) fn test_selection(&self) -> Option<Selection> {
        *lock(&self.inner.selection)
    }
    pub(crate) fn test_generation(&self) -> u64 {
        self.inner.generation.load(Ordering::Acquire)
    }
    pub(crate) fn test_same_runtime(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
    #[cfg(windows)]
    pub(crate) fn test_has_console(&self) -> bool {
        lock(&self.inner.process)
            .as_ref()
            .unwrap()
            .console
            .is_some()
    }
    #[cfg(windows)]
    pub(crate) fn test_process_alive(&self) -> bool {
        lock(&self.inner.process)
            .as_ref()
            .is_some_and(|process| process.process.is_alive())
    }
    #[cfg(windows)]
    pub(crate) fn test_observer_pid(&self) -> Option<u32> {
        lock(&self.inner.process)
            .as_ref()
            .and_then(|p| p.console.as_ref().map(|c| c.test_pid()))
    }
    #[cfg(windows)]
    pub(crate) fn test_pid(&self) -> u32 {
        lock(&self.inner.process).as_ref().unwrap().pid
    }
}

#[cfg(test)]
#[cfg(target_os = "windows")]
pub(crate) fn start_launcher_backed_test_shell() -> TerminalController {
    let command = crate::infrastructure::terminal::profiles::shell_command(ShellKind::Bash);
    let program = command
        .ancestors()
        .skip(1)
        .map(|parent| parent.join("bin/bash.exe"))
        .find(|path| path != &command && path.is_file())
        .unwrap_or(command);
    let spawned = crate::infrastructure::terminal::pty::spawn_shell_program(
        ShellKind::Bash,
        None,
        TerminalSize {
            rows: 12,
            cols: 80,
            pixel_width: 560,
            pixel_height: 228,
        },
        &program,
    )
    .unwrap();
    let controller = TerminalController::with_shell(ShellKind::Bash);
    let mut output = spawned.reader;
    thread::spawn(move || {
        let _ = std::io::copy(&mut output, &mut std::io::sink());
    });
    *lock(&controller.inner.process) = Some(spawned.process);
    *lock(&controller.inner.status) = TerminalStatus::Running;
    controller
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shutdown_uses_injected_notification_and_rejects_the_previous_generation() {
        use std::sync::atomic::AtomicUsize;
        let count = Arc::new(AtomicUsize::new(0));
        let observed = count.clone();
        let notify = FrameNotifier::new(move || {
            observed.fetch_add(1, Ordering::AcqRel);
        });
        let controller = TerminalController::new();
        let weak = Arc::downgrade(&controller.inner);
        assert!(current_inner(&weak, 0).is_some());
        controller.terminate(notify);
        assert_eq!(count.load(Ordering::Acquire), 1);
        assert!(current_inner(&weak, 0).is_none());
        assert_eq!(*lock(&controller.inner.status), TerminalStatus::Stopped);
    }
}
