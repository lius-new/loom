//! A single real terminal session backed by the platform PTY.
//!
//! The UI reads immutable snapshots from this type. PTY output is parsed on a
//! background thread and asks the application event loop for another frame.

use std::env;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::thread;
#[cfg(windows)]
use std::time::Duration;

use lgui::ApplicationHandle;
#[cfg(not(windows))]
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

const SCROLLBACK_ROWS: usize = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShellKind {
    #[default]
    PowerShell,
    Cmd,
    Bash,
}

impl ShellKind {
    pub const ALL: [Self; 3] = [Self::PowerShell, Self::Cmd, Self::Bash];

    pub const fn label(self) -> &'static str {
        match self {
            Self::PowerShell => "PowerShell",
            Self::Cmd => "Command Prompt",
            Self::Bash => "Bash",
        }
    }

    pub const fn short_label(self) -> &'static str {
        match self {
            Self::PowerShell => "pwsh",
            Self::Cmd => "cmd",
            Self::Bash => "bash",
        }
    }

    fn candidates(self) -> &'static [&'static str] {
        match self {
            Self::PowerShell => &["pwsh.exe", "powershell.exe", "pwsh", "powershell"],
            Self::Cmd => &["cmd.exe", "cmd"],
            Self::Bash => &["bash.exe", "bash"],
        }
    }

    fn command(self) -> PathBuf {
        #[cfg(windows)]
        if self == Self::Bash
            && let Some(git_bash) = find_git_bash()
        {
            return git_bash;
        }
        self.candidates()
            .iter()
            .find_map(|candidate| find_executable(candidate))
            .unwrap_or_else(|| PathBuf::from(self.candidates()[0]))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalStatus {
    Idle,
    Starting,
    Running,
    Exited(u32),
    Stopped,
    Failed(String),
}

impl TerminalStatus {
    pub fn message(&self) -> Option<String> {
        match self {
            Self::Idle | Self::Running => None,
            Self::Starting => Some("Starting terminal…".to_owned()),
            Self::Exited(code) => Some(format!("Process exited with code {code}")),
            Self::Stopped => Some("Terminal stopped — press + to start it again".to_owned()),
            Self::Failed(error) => Some(format!("Unable to start terminal: {error}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalCellStyle {
    pub foreground: Option<u32>,
    pub background: Option<u32>,
    pub bold: bool,
    pub dim: bool,
    pub underline: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalRun {
    pub start_col: u16,
    pub columns: u16,
    pub text: String,
    pub style: TerminalCellStyle,
}

#[derive(Clone, Debug)]
pub struct TerminalSnapshot {
    pub shell: ShellKind,
    pub status: TerminalStatus,
    pub rows: u16,
    pub cols: u16,
    pub lines: Vec<Vec<TerminalRun>>,
    pub cursor: (u16, u16),
    pub cursor_visible: bool,
}

#[derive(Clone)]
pub struct TerminalController {
    inner: Arc<TerminalInner>,
}

#[derive(Clone)]
pub struct TerminalTab {
    pub id: u64,
    pub controller: TerminalController,
}

#[derive(Clone)]
pub struct TerminalTabs {
    tabs: Vec<TerminalTab>,
    active_id: Option<u64>,
    next_id: u64,
}

impl TerminalTabs {
    pub fn new() -> Self {
        let mut tabs = Self {
            tabs: Vec::new(),
            active_id: None,
            next_id: 1,
        };
        tabs.add(ShellKind::default());
        tabs
    }

    pub fn tabs(&self) -> &[TerminalTab] {
        &self.tabs
    }

    pub fn active_id(&self) -> Option<u64> {
        self.active_id
    }

    pub fn active(&self) -> Option<&TerminalTab> {
        let active_id = self.active_id?;
        self.tabs.iter().find(|tab| tab.id == active_id)
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn add(&mut self, shell: ShellKind) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.tabs.push(TerminalTab {
            id,
            controller: TerminalController::with_shell(shell),
        });
        self.active_id = Some(id);
        id
    }

    pub fn select(&mut self, id: u64) {
        if self.tabs.iter().any(|tab| tab.id == id) {
            self.active_id = Some(id);
        }
    }

    pub fn close(&mut self, id: u64) -> Option<TerminalController> {
        let index = self.tabs.iter().position(|tab| tab.id == id)?;
        let removed = self.tabs.remove(index);
        if self.active_id == Some(id) {
            self.active_id = self
                .tabs
                .get(index)
                .or_else(|| self.tabs.last())
                .map(|tab| tab.id);
        }
        Some(removed.controller)
    }
}

impl Default for TerminalTabs {
    fn default() -> Self {
        Self::new()
    }
}

struct TerminalInner {
    parser: Mutex<vt100::Parser>,
    process: Mutex<Option<RunningProcess>>,
    shell: Mutex<ShellKind>,
    status: Mutex<TerminalStatus>,
    generation: AtomicU64,
}

struct RunningProcess {
    #[cfg(windows)]
    process: conpty::Process,
    #[cfg(not(windows))]
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    #[cfg(not(windows))]
    killer: Box<dyn ChildKiller + Send + Sync>,
    size: TerminalSize,
}

struct SpawnedShell {
    process: RunningProcess,
    reader: Box<dyn Read + Send>,
    #[cfg(not(windows))]
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl TerminalController {
    pub fn new() -> Self {
        Self::with_shell(ShellKind::default())
    }

    pub fn with_shell(shell: ShellKind) -> Self {
        Self {
            inner: Arc::new(TerminalInner {
                parser: Mutex::new(vt100::Parser::new(24, 80, SCROLLBACK_ROWS)),
                process: Mutex::new(None),
                shell: Mutex::new(shell),
                status: Mutex::new(TerminalStatus::Idle),
                generation: AtomicU64::new(0),
            }),
        }
    }

    pub fn ensure_started(
        &self,
        cwd: Option<&Path>,
        size: TerminalSize,
        application: Arc<ApplicationHandle>,
    ) {
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

    pub fn restart(
        &self,
        shell: ShellKind,
        cwd: Option<&Path>,
        size: TerminalSize,
        application: Arc<ApplicationHandle>,
    ) {
        let generation = self.inner.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.kill_current();
        *lock(&self.inner.shell) = shell;
        *lock(&self.inner.status) = TerminalStatus::Starting;
        *lock(&self.inner.parser) = vt100::Parser::new(size.rows, size.cols, SCROLLBACK_ROWS);

        match spawn_shell(shell, cwd, size) {
            Ok(spawned) => {
                let mut reader = spawned.reader;
                #[cfg(not(windows))]
                let mut child = spawned.child;
                *lock(&self.inner.process) = Some(spawned.process);
                *lock(&self.inner.status) = TerminalStatus::Running;
                application.request_frame();

                let reader_inner = Arc::downgrade(&self.inner);
                let reader_application = Arc::clone(&application);
                thread::Builder::new()
                    .name("leditor-terminal-output".to_owned())
                    .spawn(move || {
                        let mut bytes = [0_u8; 8 * 1024];
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
                application.request_frame();
            }
        }
    }

    pub fn terminate(&self, application: &ApplicationHandle) {
        self.inner.generation.fetch_add(1, Ordering::AcqRel);
        self.kill_current();
        *lock(&self.inner.status) = TerminalStatus::Stopped;
        application.request_frame();
    }

    pub fn write(&self, bytes: &[u8]) {
        lock(&self.inner.parser).screen_mut().set_scrollback(0);
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
                parser.screen_mut().set_size(size.rows, size.cols);
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
        let current = parser.screen().scrollback();
        let next = if rows >= 0 {
            current.saturating_add(rows as usize)
        } else {
            current.saturating_sub(rows.unsigned_abs() as usize)
        };
        parser.screen_mut().set_scrollback(next);
    }

    pub fn snapshot(&self) -> TerminalSnapshot {
        let parser = lock(&self.inner.parser);
        let screen = parser.screen();
        let (rows, cols) = screen.size();
        let lines = (0..rows).map(|row| line_runs(screen, row, cols)).collect();
        TerminalSnapshot {
            shell: self.shell(),
            status: lock(&self.inner.status).clone(),
            rows,
            cols,
            lines,
            cursor: screen.cursor_position(),
            cursor_visible: !screen.hide_cursor() && screen.scrollback() == 0,
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
fn spawn_shell(
    shell: ShellKind,
    cwd: Option<&Path>,
    size: TerminalSize,
) -> Result<SpawnedShell, String> {
    let program = shell.command();
    let mut command = std::process::Command::new(&program);
    configure_std_command(&mut command, shell, cwd);

    let mut options = conpty::ProcessOptions::default();
    options.set_console_size(Some((
        size.cols.min(i16::MAX as u16) as i16,
        size.rows.min(i16::MAX as u16) as i16,
    )));
    let mut process = options
        .spawn(command)
        .map_err(|error| format!("{error} ({})", program.display()))?;
    let reader = process.output().map_err(|error| error.to_string())?;
    let writer = process.input().map_err(|error| error.to_string())?;
    Ok(SpawnedShell {
        process: RunningProcess {
            process,
            writer: Box::new(writer),
            size,
        },
        reader: Box::new(reader),
    })
}

#[cfg(windows)]
fn configure_std_command(
    command: &mut std::process::Command,
    shell: ShellKind,
    cwd: Option<&Path>,
) {
    match shell {
        ShellKind::PowerShell => {
            command.arg("-NoLogo");
        }
        ShellKind::Cmd => {}
        ShellKind::Bash => {
            command.arg("--login");
        }
    }
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    // conpty builds an explicit environment block when any override exists,
    // so copy the inherited environment before adding terminal capabilities.
    command.envs(env::vars_os());
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("TERM_PROGRAM", "leditor");
}

#[cfg(not(windows))]
fn spawn_shell(
    shell: ShellKind,
    cwd: Option<&Path>,
    size: TerminalSize,
) -> Result<SpawnedShell, String> {
    let pty = native_pty_system();
    let pty_size = PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: size.pixel_width,
        pixel_height: size.pixel_height,
    };
    let pair = pty.openpty(pty_size).map_err(|error| error.to_string())?;
    let program = shell.command();
    let mut command = CommandBuilder::new(&program);
    match shell {
        ShellKind::PowerShell => command.arg("-NoLogo"),
        ShellKind::Cmd => {}
        ShellKind::Bash => command.arg("--login"),
    }
    if let Some(cwd) = cwd {
        command.cwd(cwd.as_os_str());
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("TERM_PROGRAM", "leditor");

    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| format!("{} ({})", error, program.display()))?;
    drop(pair.slave);
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| error.to_string())?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| error.to_string())?;
    let killer = child.clone_killer();
    Ok(SpawnedShell {
        process: RunningProcess {
            master: pair.master,
            writer,
            killer,
            size,
        },
        reader,
        child,
    })
}

#[cfg(windows)]
fn resize_process(process: &mut RunningProcess, size: TerminalSize) -> Result<(), String> {
    process
        .process
        .resize(
            size.cols.min(i16::MAX as u16) as i16,
            size.rows.min(i16::MAX as u16) as i16,
        )
        .map_err(|error| error.to_string())
}

#[cfg(not(windows))]
fn resize_process(process: &mut RunningProcess, size: TerminalSize) -> Result<(), String> {
    process
        .master
        .resize(PtySize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: size.pixel_width,
            pixel_height: size.pixel_height,
        })
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
fn terminate_process(process: &mut RunningProcess) {
    let _ = process.process.exit(1);
}

#[cfg(not(windows))]
fn terminate_process(process: &mut RunningProcess) {
    let _ = process.killer.kill();
}

#[cfg(windows)]
fn spawn_waiter(inner: Weak<TerminalInner>, generation: u64, application: Arc<ApplicationHandle>) {
    thread::Builder::new()
        .name("leditor-terminal-wait".to_owned())
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
    application: Arc<ApplicationHandle>,
) {
    thread::Builder::new()
        .name("leditor-terminal-wait".to_owned())
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

fn line_runs(screen: &vt100::Screen, row: u16, cols: u16) -> Vec<TerminalRun> {
    let mut runs = Vec::new();
    let mut current: Option<TerminalRun> = None;

    for col in 0..cols {
        let Some(cell) = screen.cell(row, col) else {
            continue;
        };
        let style = cell_style(cell);
        let cell_text = if cell.is_wide_continuation() {
            ""
        } else if cell.has_contents() {
            cell.contents()
        } else {
            " "
        };

        match current.as_mut() {
            Some(run) if run.style == style => {
                run.columns += 1;
                run.text.push_str(cell_text);
            }
            Some(_) => {
                let run = current.take().expect("current terminal run exists");
                push_visible_run(&mut runs, run);
                current = Some(TerminalRun {
                    start_col: col,
                    columns: 1,
                    text: cell_text.to_owned(),
                    style,
                });
            }
            None => {
                current = Some(TerminalRun {
                    start_col: col,
                    columns: 1,
                    text: cell_text.to_owned(),
                    style,
                });
            }
        }
    }
    if let Some(run) = current {
        push_visible_run(&mut runs, run);
    }
    runs
}

fn push_visible_run(runs: &mut Vec<TerminalRun>, mut run: TerminalRun) {
    if run.style.background.is_none() {
        let trimmed = run.text.trim_end_matches(' ');
        let removed = run.text.len() - trimmed.len();
        run.columns = run.columns.saturating_sub(removed as u16);
        run.text.truncate(trimmed.len());
    }
    if !run.text.is_empty() || run.style.background.is_some() {
        runs.push(run);
    }
}

fn cell_style(cell: &vt100::Cell) -> TerminalCellStyle {
    let mut foreground = terminal_color(cell.fgcolor());
    let mut background = terminal_color(cell.bgcolor());
    if cell.inverse() {
        std::mem::swap(&mut foreground, &mut background);
        if foreground.is_none() {
            foreground = Some(0x14161b);
        }
        if background.is_none() {
            background = Some(0xe4e4e7);
        }
    }
    TerminalCellStyle {
        foreground,
        background,
        bold: cell.bold(),
        dim: cell.dim(),
        underline: cell.underline(),
    }
}

fn terminal_color(color: vt100::Color) -> Option<u32> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Rgb(red, green, blue) => {
            Some((u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue))
        }
        vt100::Color::Idx(index) => Some(indexed_color(index)),
    }
}

fn indexed_color(index: u8) -> u32 {
    const ANSI: [u32; 16] = [
        0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, 0x666666,
        0xf14c4c, 0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xffffff,
    ];
    match index {
        0..=15 => ANSI[index as usize],
        16..=231 => {
            let index = index - 16;
            let red = index / 36;
            let green = (index % 36) / 6;
            let blue = index % 6;
            let channel = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            (u32::from(channel(red)) << 16)
                | (u32::from(channel(green)) << 8)
                | u32::from(channel(blue))
        }
        232..=255 => {
            let value = 8 + (index - 232) * 10;
            (u32::from(value) << 16) | (u32::from(value) << 8) | u32::from(value)
        }
    }
}

fn find_executable(command: &str) -> Option<PathBuf> {
    let command_path = Path::new(command);
    if command_path.components().count() > 1 && command_path.is_file() {
        return Some(command_path.to_owned());
    }
    env::split_paths(&env::var_os("PATH")?).find_map(|directory| {
        let candidate = directory.join(command);
        candidate.is_file().then_some(candidate)
    })
}

#[cfg(windows)]
fn find_git_bash() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(program_files) = env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(program_files).join("Git\\bin\\bash.exe"));
    }
    if let Some(program_files_x86) = env::var_os("ProgramFiles(x86)") {
        candidates.push(PathBuf::from(program_files_x86).join("Git\\bin\\bash.exe"));
    }
    if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local_app_data).join("Programs\\Git\\bin\\bash.exe"));
    }
    if let Some(git) = find_executable("git.exe")
        && let Some(root) = git.parent().and_then(Path::parent)
    {
        candidates.push(root.join("bin\\bash.exe"));
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_labels_are_stable() {
        assert_eq!(
            ShellKind::ALL.map(ShellKind::short_label),
            ["pwsh", "cmd", "bash"]
        );
    }

    #[test]
    fn indexed_terminal_colors_include_the_ansi_and_rgb_cube_ranges() {
        assert_eq!(indexed_color(1), 0xcd3131);
        assert_eq!(indexed_color(16), 0x000000);
        assert_eq!(indexed_color(231), 0xffffff);
        assert_eq!(indexed_color(255), 0xeeeeee);
    }

    #[test]
    fn ansi_parser_produces_colored_runs() {
        let mut parser = vt100::Parser::new(2, 20, 0);
        parser.process(b"plain \x1b[31mred\x1b[0m");
        let runs = line_runs(parser.screen(), 0, 20);
        assert!(runs.iter().any(|run| run.text.contains("plain")));
        assert!(
            runs.iter()
                .any(|run| { run.text == "red" && run.style.foreground == Some(0xcd3131) })
        );
    }

    #[test]
    fn terminal_tabs_add_select_and_close_neighboring_sessions() {
        let mut tabs = TerminalTabs::new();
        let first = tabs.active_id().expect("initial terminal tab");
        let second = tabs.add(ShellKind::Bash);
        assert_eq!(tabs.tabs().len(), 2);
        assert_eq!(tabs.active_id(), Some(second));
        assert_eq!(
            tabs.active().map(|tab| tab.controller.shell()),
            Some(ShellKind::Bash)
        );

        tabs.select(first);
        assert_eq!(tabs.active_id(), Some(first));
        tabs.close(first);
        assert_eq!(tabs.active_id(), Some(second));
        tabs.close(second);
        assert!(tabs.is_empty());
        assert_eq!(tabs.active_id(), None);
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "manual Windows ConPTY smoke test"]
    fn supported_shells_round_trip_through_real_conpty_sessions() {
        assert_shell_round_trip(ShellKind::PowerShell, b"Write-Output (6*7)\r", "42");
        assert_shell_round_trip(ShellKind::Cmd, b"set /a 6*7\r", "42");
        assert_shell_round_trip(ShellKind::Bash, b"printf '%s\\n' $((6*7))\r", "42");
    }

    #[cfg(target_os = "windows")]
    fn assert_shell_round_trip(shell: ShellKind, command: &[u8], expected: &str) {
        use std::time::{Duration, Instant};

        let controller = TerminalController::new();
        let application = Arc::new(ApplicationHandle::new(|task| task(), || {}));
        let size = TerminalSize {
            rows: 12,
            cols: 80,
            pixel_width: 7,
            pixel_height: 19,
        };
        controller.restart(
            shell,
            env::current_dir().ok().as_deref(),
            size,
            application.clone(),
        );
        controller.write(command);

        let deadline = Instant::now() + Duration::from_secs(8);
        let mut saw_answer = false;
        let mut last_contents = String::new();
        let mut last_status = TerminalStatus::Idle;
        while Instant::now() < deadline {
            let snapshot = controller.snapshot();
            last_status = snapshot.status;
            last_contents = snapshot
                .lines
                .into_iter()
                .flatten()
                .map(|run| run.text)
                .collect::<String>();
            if last_contents.contains(expected) {
                saw_answer = true;
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        controller.terminate(&application);
        assert!(
            saw_answer,
            "{} did not return {expected:?}; status={last_status:?}; contents={last_contents:?}",
            shell.label(),
        );
    }
}
