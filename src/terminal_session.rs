//! Legacy terminal tab/model composition, retained until R7.3.
#[cfg(test)]
use crate::features::terminal::TerminalSize;
use crate::features::terminal::{SessionId, ShellKind, TERMINAL_PURPOSES};
use crate::infrastructure::terminal::session::TerminalController;
use std::path::PathBuf;

#[derive(Clone)]
pub struct TerminalTab {
    pub id: SessionId,
    /// Sessions with the same group share a tab and render side by side.
    pub group_id: u64,
    pub width_weight: f32,
    pub controller: TerminalController,
    pub cwd: Option<PathBuf>,
    pub purpose: Option<String>,
    pub running_program: Option<String>,
}

impl TerminalTab {
    pub fn working_directory(&self) -> Option<PathBuf> {
        self.controller
            .working_directory()
            .or_else(|| self.cwd.clone())
    }

    pub fn label(&self) -> String {
        let shell = self.controller.short_label();
        match self.purpose.as_ref().or(self.running_program.as_ref()) {
            Some(name) => format!("{shell} · {name}"),
            None => shell.to_owned(),
        }
    }
}

#[derive(Clone)]
pub struct TerminalTabs {
    tabs: Vec<TerminalTab>,
    active_id: Option<u64>,
    next_id: u64,
    profiles: Vec<crate::features::terminal::profiles::ShellProfile>,
}

impl TerminalTabs {
    pub fn new() -> Self {
        let mut tabs = Self::empty();
        tabs.add(ShellKind::default());
        tabs
    }

    pub fn restored(shells: Vec<ShellKind>, active_index: Option<usize>) -> Self {
        Self::restored_with_groups(shells, active_index, &[])
    }

    pub fn restored_with_groups(
        shells: Vec<ShellKind>,
        active_index: Option<usize>,
        groups: &[usize],
    ) -> Self {
        let mut tabs = Self::empty();
        for (index, shell) in shells.into_iter().enumerate() {
            let id = tabs.add(shell);
            if index > 0
                && groups.get(index).is_some()
                && groups.get(index) == groups.get(index - 1)
            {
                let group = tabs.tabs[index - 1].group_id;
                tabs.tabs.last_mut().unwrap().group_id = group;
            }
            debug_assert_eq!(tabs.active_id, Some(id));
        }
        if let Some(active_id) = active_index
            .and_then(|index| tabs.tabs.get(index))
            .map(|tab| tab.id)
        {
            tabs.active_id = Some(active_id);
        }
        tabs
    }

    fn empty() -> Self {
        Self {
            tabs: Vec::new(),
            active_id: None,
            next_id: 1,
            profiles: Vec::new(),
        }
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

    pub fn active_group(&self) -> Vec<TerminalTab> {
        let Some(active) = self.active() else {
            return Vec::new();
        };
        self.tabs
            .iter()
            .filter(|tab| tab.group_id == active.group_id)
            .cloned()
            .collect()
    }

    /// A tabbed region shares controller Arcs. Legacy split groups must not make
    /// its tabs render as simultaneous terminal bodies.
    pub fn region_view(&self, ids: &[u64], active: Option<u64>) -> Self {
        let tabs = ids
            .iter()
            .filter_map(|id| self.tabs.iter().find(|tab| tab.id == *id))
            .map(|tab| {
                let mut tab = tab.clone();
                tab.group_id = tab.id;
                tab
            })
            .collect::<Vec<_>>();
        let active_id = self
            .active_id
            .filter(|id| ids.contains(id))
            .or(active.filter(|id| ids.contains(id)))
            .or_else(|| tabs.first().map(|t| t.id));
        Self {
            tabs,
            active_id,
            next_id: self.next_id,
            profiles: self.profiles.clone(),
        }
    }

    /// Moving a session out of a legacy side-by-side group does not restart it.
    pub fn detach_group(&mut self, id: u64) {
        if let Some(group) = self
            .tabs
            .iter()
            .find(|tab| tab.id == id)
            .map(|tab| tab.group_id)
            && let Some(other) = self
                .tabs
                .iter()
                .find(|tab| tab.id != id && tab.group_id == group)
                .map(|tab| tab.id)
        {
            for tab in &mut self.tabs {
                if tab.group_id == group && tab.id != id {
                    tab.group_id = other;
                }
            }
        }
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
            tab.group_id = tab.id;
            tab.width_weight = 1.0;
        }
    }

    pub fn saved_groups(&self) -> Vec<usize> {
        let mut group = 0;
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                if index > 0 && self.tabs[index - 1].group_id != tab.group_id {
                    group += 1;
                }
                group
            })
            .collect()
    }

    pub fn widths(&self) -> Vec<f32> {
        self.tabs.iter().map(|tab| tab.width_weight).collect()
    }

    pub fn purposes(&self) -> Vec<Option<String>> {
        self.tabs.iter().map(|tab| tab.purpose.clone()).collect()
    }

    pub fn directories(&self) -> Vec<Option<PathBuf>> {
        self.tabs
            .iter()
            .map(TerminalTab::working_directory)
            .collect()
    }

    pub fn restore_directories(&mut self, directories: &[Option<PathBuf>]) {
        for (tab, cwd) in self.tabs.iter_mut().zip(directories) {
            tab.cwd = cwd.clone();
        }
    }

    pub fn set_profiles(&mut self, profiles: &[crate::features::terminal::profiles::ShellProfile]) {
        self.profiles = profiles.to_vec();
        for tab in &self.tabs {
            tab.controller.set_profile(
                profiles
                    .iter()
                    .find(|profile| tab.controller.shell() == ShellKind::Custom(profile.id))
                    .cloned(),
            );
        }
    }

    fn controller_for(&self, shell: ShellKind) -> TerminalController {
        let controller = TerminalController::with_shell(shell);
        controller.set_profile(
            self.profiles
                .iter()
                .find(|p| shell == ShellKind::Custom(p.id))
                .cloned(),
        );
        controller
    }

    pub fn update_directories(&mut self, directories: &[(u64, Option<PathBuf>)]) {
        for (id, cwd) in directories {
            if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == *id) {
                tab.cwd = cwd.clone();
            }
        }
    }

    pub fn restore_purposes(&mut self, purposes: &[Option<String>]) {
        for (tab, purpose) in self.tabs.iter_mut().zip(purposes) {
            tab.purpose = normalized_purpose(purpose.as_deref());
        }
    }

    pub fn set_purpose(&mut self, id: u64, purpose: Option<&str>) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
            tab.purpose = normalized_purpose(purpose);
        }
    }

    pub fn update_programs(&mut self, programs: &[(u64, Option<String>)]) {
        for (id, program) in programs {
            if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == *id) {
                tab.running_program = program.clone();
            }
        }
    }

    pub fn restore_widths(&mut self, widths: &[f32]) {
        for (tab, width) in self.tabs.iter_mut().zip(widths) {
            if width.is_finite() && *width > 0.0 {
                tab.width_weight = width.clamp(0.001, 1000.0);
            }
        }
    }

    pub fn split(&mut self, target: u64) -> Option<u64> {
        let index = self.tabs.iter().position(|tab| tab.id == target)?;
        let group_id = self.tabs[index].group_id;
        let shell = self.tabs[index].controller.shell();
        let cwd = self.tabs[index].working_directory();
        let width_weight = self.tabs[index].width_weight * 0.5;
        self.tabs[index].width_weight = width_weight;
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.tabs.insert(
            index + 1,
            TerminalTab {
                id,
                group_id,
                width_weight,
                controller: self.controller_for(shell),
                cwd,
                purpose: None,
                running_program: None,
            },
        );
        self.active_id = Some(id);
        Some(id)
    }

    pub fn shells(&self) -> Vec<ShellKind> {
        self.tabs.iter().map(|tab| tab.controller.shell()).collect()
    }

    pub fn active_index(&self) -> Option<usize> {
        let active_id = self.active_id?;
        self.tabs.iter().position(|tab| tab.id == active_id)
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn add(&mut self, shell: ShellKind) -> u64 {
        self.add_at(shell, None)
    }

    pub fn add_at(&mut self, shell: ShellKind, cwd: Option<PathBuf>) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.tabs.push(TerminalTab {
            id,
            group_id: id,
            width_weight: 1.0,
            controller: self.controller_for(shell),
            cwd,
            purpose: None,
            running_program: None,
        });
        self.active_id = Some(id);
        id
    }

    pub fn select(&mut self, id: u64) {
        if self.tabs.iter().any(|tab| tab.id == id) {
            self.active_id = Some(id);
        }
    }

    pub fn resize_pair(&mut self, left: u64, right: u64, fraction: f32) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == left) else {
            return;
        };
        let Some(next) = self.tabs.get(index + 1) else {
            return;
        };
        if next.id != right || next.group_id != self.tabs[index].group_id {
            return;
        }
        let total = self.tabs[index].width_weight + next.width_weight;
        let fraction = fraction.clamp(0.001, 0.999);
        self.tabs[index].width_weight = total * fraction;
        self.tabs[index + 1].width_weight = total * (1.0 - fraction);
    }

    pub fn close(&mut self, id: u64) -> Option<TerminalController> {
        let index = self.tabs.iter().position(|tab| tab.id == id)?;
        let removed = self.tabs.remove(index);
        if self.active_id == Some(id) {
            self.active_id = self
                .tabs
                .iter()
                .skip(index)
                .find(|tab| tab.group_id == removed.group_id)
                .or_else(|| {
                    self.tabs
                        .iter()
                        .rev()
                        .find(|tab| tab.group_id == removed.group_id)
                })
                .or_else(|| self.tabs.get(index).or_else(|| self.tabs.last()))
                .map(|tab| tab.id);
        }
        Some(removed.controller)
    }
}

fn normalized_purpose(purpose: Option<&str>) -> Option<String> {
    let purpose = purpose.map(str::trim).filter(|value| !value.is_empty())?;
    // Older saved custom purposes use the general preset under the new policy.
    Some(
        if TERMINAL_PURPOSES.contains(&purpose) {
            purpose
        } else {
            "Other"
        }
        .to_owned(),
    )
}

impl Default for TerminalTabs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::terminal_runtime::TerminalDisplay;
    #[cfg(windows)]
    use crate::features::terminal::TerminalStatus;
    #[cfg(windows)]
    use crate::infrastructure::terminal::pty::shell_friendly_path;
    use lgui::ApplicationHandle;
    use std::env;
    use std::sync::Arc;
    #[cfg(windows)]
    use std::thread;
    #[cfg(windows)]
    use std::{io::Write, path::Path, time::Duration};

    #[cfg(windows)]
    #[test]
    fn custom_profiles_preserve_arguments_split_configuration_and_missing_path_fallback() {
        use crate::features::terminal::profiles::ShellProfile;
        use std::time::{Instant, SystemTime, UNIX_EPOCH};
        let root = env::temp_dir().join(format!(
            "loom-custom-shell-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let source = root.join("probe.rs");
        let program = root.join("中文 custom shell.exe");
        std::fs::write(&source, r#"use std::io::Write; fn main() { for (i, arg) in std::env::args().skip(1).enumerate() { println!("ARG{i}={arg:?}"); } std::io::stdout().flush().unwrap(); let mut line = String::new(); let _ = std::io::stdin().read_line(&mut line); }"#).unwrap();
        let built = std::process::Command::new("rustc")
            .arg("--crate-name=loom_custom_shell_probe")
            .arg(&source)
            .arg("-o")
            .arg(&program)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let profile = ShellProfile {
            id: 123,
            name: "Custom shell".into(),
            program,
            args: vec![
                "".into(),
                "space path".into(),
                "literal \"quotes\"".into(),
                "C:\\trailing path\\".into(),
                "中文 $() ` &".into(),
            ],
        };
        let mut tabs = TerminalTabs::restored(vec![], None);
        tabs.set_profiles(std::slice::from_ref(&profile));
        let first = tabs.add_at(ShellKind::Custom(profile.id), Some(root.clone()));
        let controller = tabs.active().unwrap().controller.clone();
        let second = tabs.split(first).unwrap();
        assert_eq!(
            *tabs.active().unwrap().controller.test_profile(),
            Some(profile.clone())
        );
        assert_eq!(tabs.active_id(), Some(second));
        let application = Arc::new(ApplicationHandle::new(|task| task(), || {}));
        let size = TerminalSize {
            rows: 24,
            cols: 120,
            pixel_width: 840,
            pixel_height: 456,
        };
        controller.restart(
            ShellKind::Custom(profile.id),
            Some(&root),
            size,
            application.clone(),
        );
        let expected = profile
            .args
            .iter()
            .enumerate()
            .map(|(i, arg)| format!("ARG{i}={arg:?}"))
            .collect::<Vec<_>>();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let output = controller.test_parser().screen().contents();
            if expected.iter().all(|arg| output.contains(arg)) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "custom args: {output:?}; expected: {expected:?}"
            );
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(controller.shell(), ShellKind::Custom(profile.id));
        controller.terminate(&application);
        let mut missing = profile.clone();
        missing.program = root.join("missing.exe");
        tabs.set_profiles(std::slice::from_ref(&missing));
        controller.restart(
            ShellKind::Custom(profile.id),
            Some(&root),
            size,
            application.clone(),
        );
        assert_eq!(controller.shell(), ShellKind::Custom(profile.id));
        assert_eq!(controller.short_label(), missing.name);
        assert!(matches!(
            controller.snapshot().status,
            TerminalStatus::Running
        ));
        assert!(
            controller
                .snapshot()
                .notice
                .unwrap()
                .contains("Custom shell is unavailable")
        );
        assert_eq!(tabs.profiles, vec![missing]);
        controller.terminate(&application);

        // Repairing discovery must launch the configured profile again, rather
        // than preserving the shell that was temporarily used as a fallback.
        tabs.set_profiles(std::slice::from_ref(&profile));
        controller.restart(
            ShellKind::Custom(profile.id),
            Some(&root),
            size,
            application.clone(),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let output = controller.test_parser().screen().contents();
            if expected.iter().all(|arg| output.contains(arg)) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "repaired custom args: {output:?}"
            );
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(controller.shell(), ShellKind::Custom(profile.id));
        assert_eq!(controller.short_label(), profile.name);
        assert!(controller.snapshot().notice.is_none());
        assert_eq!(tabs.profiles, vec![profile]);
        controller.terminate(&application);
    }

    #[cfg(windows)]
    #[test]
    fn conpty_delivers_wheels_as_native_mouse_records_and_restores_history() {
        use crate::features::terminal::input::Modifiers as KeyModifiers;
        use std::time::Instant;

        if !crate::infrastructure::terminal::profiles::shell_command(ShellKind::Bash).is_file() {
            eprintln!("Git Bash not installed; skipping native mouse round trip");
            return;
        }
        let application = Arc::new(ApplicationHandle::new(|task| task(), || {}));
        let controller = TerminalController::with_shell(ShellKind::Bash);
        let size = TerminalSize {
            rows: 32,
            cols: 120,
            pixel_width: 840,
            pixel_height: 608,
        };
        controller.restart(ShellKind::Bash, None, size, application);

        fn wait_for(controller: &TerminalController, predicate: impl Fn(&vt100::Screen) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if predicate(controller.test_parser().screen()) {
                    return;
                }
                thread::sleep(Duration::from_millis(20));
            }
            panic!(
                "native mouse probe timed out: {:?}",
                controller.test_parser().screen().contents()
            );
        }

        let executable = std::env::current_exe()
            .unwrap()
            .display()
            .to_string()
            .replace('\\', "/")
            .replace('\'', "'\\''");
        controller.write(format!("LOOM_MOUSE_PROBE=1 '{executable}' --exact terminal_session::tests::native_mouse_probe --ignored --nocapture\r").as_bytes());
        wait_for(&controller, |screen| {
            screen.contents().contains("MOUSE_READY")
                && screen.mouse_protocol_encoding() == vt100::MouseProtocolEncoding::Sgr
        });
        assert!(controller.test_parser().screen().alternate_screen());
        assert_eq!(
            controller.test_parser().screen().mouse_protocol_mode(),
            vt100::MouseProtocolMode::AnyMotion
        );
        controller.set_mouse_position(7, 13);
        controller.wheel(1.0, KeyModifiers::empty());
        wait_for(&controller, |screen| {
            screen.contents().contains("WHEEL:up:13:7:false:false")
        });
        // Shift must bypass application reporting, even in the alternate screen.
        controller.wheel(1.0, KeyModifiers::SHIFT);
        controller.wheel(-1.0, KeyModifiers::CONTROL | KeyModifiers::ALT);
        wait_for(&controller, |screen| {
            screen.contents().contains("WHEEL:down:13:7:true:true")
        });
        controller.write(b"q");
        wait_for(&controller, |screen| {
            !screen.alternate_screen()
                && screen.mouse_protocol_mode() == vt100::MouseProtocolMode::None
        });
        controller.write(b"for ((i=1;i<=80;i++)); do printf 'history-%03d\\n' $i; done\r");
        wait_for(&controller, |screen| {
            screen.contents().contains("history-080")
        });
        controller.wheel(1.0, KeyModifiers::empty());
        assert_eq!(controller.test_parser().screen().scrollback(), 3);
        controller.wheel(-1.0, KeyModifiers::empty());
        assert_eq!(controller.test_parser().screen().scrollback(), 0);
    }

    /// Runs inside the test's ConPTY to exercise the same Windows input API
    /// used by Codex/crossterm, rather than merely inspecting generated bytes.
    #[cfg(windows)]
    #[test]
    #[ignore = "native console helper launched by the mouse round-trip test"]
    fn native_mouse_probe() {
        if std::env::var_os("LOOM_MOUSE_PROBE").is_none() {
            return;
        }
        use std::ffi::c_void;
        #[repr(C)]
        struct InputRecord {
            kind: u16,
            padding: u16,
            data: [u32; 4],
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetStdHandle(kind: u32) -> *mut c_void;
            fn GetConsoleMode(handle: *mut c_void, mode: *mut u32) -> i32;
            fn SetConsoleMode(handle: *mut c_void, mode: u32) -> i32;
            fn ReadConsoleInputW(
                handle: *mut c_void,
                records: *mut InputRecord,
                length: u32,
                read: *mut u32,
            ) -> i32;
        }
        unsafe {
            let input = GetStdHandle((-10i32) as u32);
            let output = GetStdHandle((-11i32) as u32);
            let (mut input_mode, mut output_mode) = (0, 0);
            assert_ne!(GetConsoleMode(input, &mut input_mode), 0);
            assert_ne!(GetConsoleMode(output, &mut output_mode), 0);
            assert_ne!(SetConsoleMode(output, output_mode | 4), 0);
            assert_ne!(SetConsoleMode(input, (input_mode | 0x98) & !0x247), 0);
            print!("\x1b[?1049h\x1b[?1006hMOUSE_READY\r\n");
            std::io::stdout().flush().unwrap();
            let mut wheels = 0;
            loop {
                let mut record = InputRecord {
                    kind: 0,
                    padding: 0,
                    data: [0; 4],
                };
                let mut read = 0;
                assert_ne!(ReadConsoleInputW(input, &mut record, 1, &mut read), 0);
                if record.kind == 2 && record.data[3] == 4 {
                    let x = record.data[0] as u16;
                    let y = (record.data[0] >> 16) as u16;
                    let delta = (record.data[1] >> 16) as i16;
                    let alt = record.data[2] & 3 != 0;
                    let ctrl = record.data[2] & 12 != 0;
                    let direction = if delta > 0 { "up" } else { "down" };
                    println!("WHEEL:{direction}:{x}:{y}:{alt}:{ctrl}");
                    std::io::stdout().flush().unwrap();
                    wheels += 1;
                } else if wheels >= 2 && record.kind == 1 && record.data[0] != 0 {
                    break;
                }
            }
            print!("\x1b[?1006l\x1b[?1049l");
            std::io::stdout().flush().unwrap();
            assert_ne!(SetConsoleMode(input, input_mode), 0);
            assert_ne!(SetConsoleMode(output, output_mode), 0);
        }
    }

    #[test]
    fn saved_directories_stay_with_independent_tabs_when_siblings_close() {
        let mut tabs = TerminalTabs::restored(vec![ShellKind::Cmd; 3], Some(1));
        let dirs = (0..3)
            .map(|i| Some(env::temp_dir().join(format!("cwd-{i}"))))
            .collect::<Vec<_>>();
        tabs.restore_directories(&dirs);
        assert_eq!(tabs.directories(), dirs);
        let ids = tabs.tabs().iter().map(|t| t.id).collect::<Vec<_>>();
        let changed = env::temp_dir().join("changed-live-directory");
        *tabs.tabs()[1].controller.test_cwd() = Some(changed.clone());
        assert_eq!(
            tabs.region_view(&[ids[2], ids[1]], Some(ids[1]))
                .directories(),
            vec![dirs[2].clone(), Some(changed.clone())]
        );
        tabs.close(ids[0]);
        assert_eq!(tabs.directories(), vec![Some(changed), dirs[2].clone()]);
        let mut legacy = TerminalTabs::restored(tabs.shells(), tabs.active_index());
        legacy.restore_directories(&[]);
        assert_eq!(legacy.directories(), vec![None, None]);
    }

    #[cfg(windows)]
    #[test]
    fn real_shell_directories_follow_cd_and_survive_session_restoration() {
        use crate::workspace_persistence::{WorkspaceSession, write_json};
        use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

        let root = env::temp_dir().join(format!(
            "loom-cwd-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let initial = root.join("initial");
        let changed = root.join("中文 spaces & quote'");
        std::fs::create_dir_all(&initial).unwrap();
        std::fs::create_dir_all(&changed).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let application = Arc::new(ApplicationHandle::new(|task| task(), || {}));
        let size = TerminalSize {
            rows: 24,
            cols: 160,
            pixel_width: 1120,
            pixel_height: 456,
        };

        fn wait_for_directory(controller: &TerminalController, expected: &Path, shell: ShellKind) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if controller
                    .working_directory()
                    .and_then(|p| p.canonicalize().ok())
                    == expected.canonicalize().ok()
                {
                    return;
                }
                thread::sleep(Duration::from_millis(20));
            }
            let output = controller.test_parser().screen().contents();
            panic!(
                "{} cwd={:?} status={:?} output={output:?}",
                shell.label(),
                controller.working_directory(),
                controller.snapshot().status
            );
        }

        for shell in ShellKind::ALL {
            if !crate::infrastructure::terminal::profiles::shell_command(shell).is_file() {
                eprintln!("{} not installed; skipping real shell test", shell.label());
                continue;
            }
            let mut tabs = TerminalTabs::restored(vec![], None);
            let first = tabs.add_at(shell, Some(initial.clone()));
            let controller = tabs.active().unwrap().controller.clone();
            controller.restart(shell, Some(&initial), size, application.clone());
            let command = match shell {
                ShellKind::PowerShell => format!(
                    "Set-Location -LiteralPath '{}'\r",
                    changed.display().to_string().replace('\'', "''")
                ),
                ShellKind::Cmd => format!("cd /d \"{}\"\r", changed.display()),
                ShellKind::Bash => format!(
                    "cd -- '{}'\r",
                    changed
                        .display()
                        .to_string()
                        .replace('\\', "/")
                        .replace('\'', "'\\''")
                ),
                ShellKind::System | ShellKind::Custom(_) => unreachable!("Windows builtin test"),
            };
            controller.write(command.as_bytes());
            wait_for_directory(&controller, &changed, shell);
            // Inheritance must read the controller immediately, before any UI poll.
            assert_eq!(tabs.directories(), vec![controller.working_directory()]);
            let second = tabs.split(first).unwrap();
            assert_eq!(tabs.tabs()[1].cwd, controller.working_directory());
            let session = WorkspaceSession {
                terminal_tabs: tabs.shells(),
                terminal_groups: tabs.saved_groups(),
                terminal_directories: tabs.directories(),
                active_terminal: tabs.active_index(),
                ..Default::default()
            };
            let path = root.join("workspace.json");
            write_json(&path, &session).unwrap();
            let decoded: WorkspaceSession =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            controller.terminate(&application);
            let mut restored = TerminalTabs::restored_with_groups(
                decoded.terminal_tabs,
                decoded.active_terminal,
                &decoded.terminal_groups,
            );
            restored.restore_directories(&decoded.terminal_directories);
            assert_eq!(restored.active_id(), Some(second));
            for tab in restored.tabs() {
                let cwd = tab.working_directory().unwrap();
                tab.controller
                    .restart(shell, Some(&cwd), size, application.clone());
                // Require a new real prompt report, not just the requested spawn cwd.
                *tab.controller.test_cwd() = None;
                tab.controller.write(b"\r");
                wait_for_directory(&tab.controller, &changed, shell);
                tab.controller.terminate(&application);
            }
        }
    }

    #[test]
    fn tab_names_keep_shell_type_and_prioritize_purpose_then_running_program() {
        let mut tabs =
            TerminalTabs::restored(vec![ShellKind::Bash, ShellKind::Bash, ShellKind::Cmd], None);
        let first = tabs.tabs()[0].id;
        assert_eq!(
            tabs.tabs()
                .iter()
                .map(TerminalTab::label)
                .collect::<Vec<_>>(),
            vec!["bash", "bash", "cmd"]
        );
        tabs.update_programs(&[(first, Some("node".into()))]);
        assert_eq!(tabs.tabs()[0].label(), "bash · node");
        tabs.set_purpose(first, Some("  Testing  "));
        assert_eq!(tabs.tabs()[0].label(), "bash · Testing");
        tabs.update_programs(&[(first, Some("python".into()))]);
        assert_eq!(tabs.tabs()[0].label(), "bash · Testing");
        tabs.set_purpose(first, None);
        assert_eq!(tabs.tabs()[0].label(), "bash · python");
        tabs.update_programs(&[(first, None)]);
        assert_eq!(tabs.tabs()[0].label(), "bash");
        tabs.set_purpose(first, Some("\r\n "));
        assert_eq!(tabs.tabs()[0].label(), "bash");
    }

    #[test]
    fn purposes_survive_restoration_moves_and_sibling_closure_without_saving_programs() {
        let mut tabs = TerminalTabs::new();
        let first = tabs.active_id().unwrap();
        tabs.set_purpose(first, Some("Server"));
        tabs.update_programs(&[(first, Some("node".into()))]);
        let second = tabs.split(first).unwrap();
        assert_eq!(tabs.tabs()[1].purpose, None);
        tabs.set_purpose(second, Some("Testing"));
        let mut restored = TerminalTabs::restored_with_groups(
            tabs.shells(),
            tabs.active_index(),
            &tabs.saved_groups(),
        );
        restored.restore_purposes(&tabs.purposes());
        assert_eq!(restored.purposes(), tabs.purposes());
        assert!(
            restored
                .tabs()
                .iter()
                .all(|tab| tab.running_program.is_none())
        );
        restored.detach_group(second);
        assert_eq!(
            restored
                .region_view(&[second], Some(second))
                .active()
                .unwrap()
                .label(),
            "pwsh · Testing"
        );
        restored.close(first);
        assert_eq!(restored.purposes(), vec![Some("Testing".into())]);
    }

    #[test]
    fn saved_custom_purposes_migrate_to_other_and_only_presets_are_kept() {
        let mut tabs = TerminalTabs::restored(vec![ShellKind::Bash; 3], None);
        tabs.restore_purposes(&[Some("Local testing".into()), Some("Server".into()), None]);
        assert_eq!(
            tabs.purposes(),
            vec![Some("Other".into()), Some("Server".into()), None]
        );
        let first = tabs.tabs()[0].id;
        for &purpose in TERMINAL_PURPOSES {
            tabs.set_purpose(first, Some(purpose));
            assert_eq!(tabs.tabs()[0].purpose.as_deref(), Some(purpose));
        }
        tabs.set_purpose(first, Some("arbitrary custom name"));
        assert_eq!(tabs.tabs()[0].purpose.as_deref(), Some("Other"));
    }

    #[cfg(windows)]
    #[test]
    fn shell_cwd_drops_the_verbatim_prefix() {
        assert_eq!(
            shell_friendly_path(Path::new(r"\\?\D:\Documents\loom")),
            PathBuf::from(r"D:\Documents\loom")
        );
        assert_eq!(
            shell_friendly_path(Path::new(r"\\?\UNC\server\share\dir")),
            PathBuf::from(r"\\server\share\dir")
        );
        assert_eq!(
            shell_friendly_path(Path::new(r"D:\plain")),
            PathBuf::from(r"D:\plain")
        );
        assert_eq!(
            shell_friendly_path(Path::new(r"\\?\Volume{abc}\dir")),
            PathBuf::from(r"\\?\Volume{abc}\dir")
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

    #[test]
    fn splitting_keeps_independent_sessions_in_one_group_and_close_keeps_siblings() {
        let mut tabs = TerminalTabs::new();
        let first = tabs.active_id().unwrap();
        let other = tabs.add(ShellKind::Cmd);
        let second = tabs.split(first).unwrap();
        let third = tabs.split(second).unwrap();
        assert_eq!(
            tabs.active_group()
                .iter()
                .map(|tab| tab.id)
                .collect::<Vec<_>>(),
            vec![first, second, third]
        );
        assert_eq!(tabs.saved_groups(), vec![0, 0, 0, 1]);
        assert!(
            !tabs.tabs()[0]
                .controller
                .test_same_runtime(&tabs.tabs()[1].controller)
        );
        tabs.select(first);
        tabs.close(first);
        assert_eq!(tabs.active_id(), Some(second));
        tabs.close(second);
        assert_eq!(tabs.active_id(), Some(third));
        assert_eq!(tabs.saved_groups(), vec![0, 1]);
        tabs.close(third);
        assert_eq!(tabs.active_id(), Some(other));
        assert!(tabs.split(999).is_none());
    }

    #[test]
    fn moving_a_group_leader_preserves_controller_cwd_scrollback_and_selection() {
        use crate::domain::application_layout::{ApplicationLayout, Placement, Visibility};
        use crate::model::pane_layout::Direction;
        use lgui::prelude::UiRect;
        let mut tabs = TerminalTabs::restored(vec![], None);
        let cwd = PathBuf::from("existing-workspace");
        let first = tabs.add_at(ShellKind::Cmd, Some(cwd.clone()));
        let sibling = tabs.split(first).unwrap();
        let controller = tabs.tabs()[0].controller.clone();
        for _ in 0..30 {
            controller.test_parser().process(b"preserved output\r\n");
        }
        controller.scroll(3);
        controller.begin_selection(0, 0);
        controller.extend_selection(0, 9);
        let selected = controller.selected_text();
        let selection = controller.test_selection();
        let scrollback = controller.test_parser().screen().scrollback();
        let generation = controller.test_generation();
        let rect = UiRect::new(0.0, 0.0, 1200.0, 800.0);
        let visible = Visibility {
            git: true,
            files: true,
            terminal: true,
        };
        let mut layout = ApplicationLayout::initial(rect.into(), 200.0, 200.0, 240.0);
        layout.sync(&[(first, first), (sibling, first)], Some(first));
        let plan = layout
            .plan(
                first,
                &Placement::Side(1, Direction::Right),
                rect.into(),
                visible,
                vec![first, sibling],
            )
            .unwrap();
        let destination = plan.destination;
        assert!(layout.commit(plan, rect.into(), visible, &[first, sibling]));
        tabs.detach_group(first);
        tabs.select(first);
        let (sessions, active) = layout.terminal_view(destination).unwrap();
        let view = tabs.region_view(&sessions, active);
        assert!(controller.test_same_runtime(&view.active().unwrap().controller));
        assert_eq!(view.active().unwrap().cwd, Some(cwd));
        assert_eq!(
            view.active().unwrap().controller.test_generation(),
            generation
        );
        assert_ne!(tabs.tabs()[0].group_id, tabs.tabs()[1].group_id);
        controller.resize(TerminalSize {
            rows: 12,
            cols: 60,
            pixel_width: 440,
            pixel_height: 230,
        });
        assert_eq!(controller.test_selection(), selection);
        assert_eq!(controller.test_parser().screen().scrollback(), scrollback);
        assert_eq!(controller.selected_text(), selected);
        // Newly created sessions join the remaining default tab strip.
        let third = tabs.split(sibling).unwrap();
        layout.sync(
            &tabs
                .tabs()
                .iter()
                .map(|t| (t.id, t.group_id))
                .collect::<Vec<_>>(),
            Some(third),
        );
        assert_eq!(
            layout.terminal_region(third),
            layout.terminal_region(sibling)
        );
        assert!(layout.terminal_region(third).is_some());
        assert_ne!(layout.terminal_region(third), layout.terminal_region(first));
        let ordered = tabs.region_view(&[sibling, first], Some(first));
        assert_eq!(
            ordered.tabs().iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![sibling, first]
        );
    }

    #[test]
    fn grouped_terminal_layout_round_trips_and_legacy_sessions_stay_separate() {
        let mut original = TerminalTabs::new();
        let first = original.active_id().unwrap();
        let split = original.split(first).unwrap();
        original.add_at(ShellKind::Bash, Some("workspace".into()));
        original.select(split);
        original.resize_pair(first, split, 0.75);
        let mut restored = TerminalTabs::restored_with_groups(
            original.shells(),
            original.active_index(),
            &original.saved_groups(),
        );
        restored.restore_widths(&original.widths());
        assert_eq!(restored.widths(), original.widths());
        assert_eq!(restored.saved_groups(), vec![0, 0, 1]);
        assert_eq!(restored.active_index(), Some(1));
        assert_eq!(restored.active_group().len(), 2);
        let legacy = TerminalTabs::restored(original.shells(), original.active_index());
        assert_eq!(legacy.saved_groups(), vec![0, 1, 2]);
    }

    #[test]
    fn terminal_tab_can_request_its_own_working_directory() {
        let mut tabs = TerminalTabs::restored(Vec::new(), None);
        let cwd = PathBuf::from("workspace").join("nested");

        tabs.add_at(ShellKind::default(), Some(cwd.clone()));

        assert_eq!(tabs.active().and_then(|tab| tab.cwd.as_ref()), Some(&cwd));
    }

    #[test]
    fn terminal_tabs_restore_shell_order_and_active_index() {
        let tabs = TerminalTabs::restored(
            vec![ShellKind::Cmd, ShellKind::Bash, ShellKind::PowerShell],
            Some(1),
        );

        assert_eq!(
            tabs.shells(),
            vec![ShellKind::Cmd, ShellKind::Bash, ShellKind::PowerShell]
        );
        assert_eq!(tabs.active_index(), Some(1));
        assert_eq!(
            tabs.active().map(|tab| tab.controller.shell()),
            Some(ShellKind::Bash)
        );

        let empty = TerminalTabs::restored(Vec::new(), None);
        assert!(empty.is_empty());
        assert_eq!(empty.active_index(), None);
    }

    #[cfg(target_os = "windows")]
    fn start_launcher_backed_test_shell() -> (TerminalController, Arc<ApplicationHandle>) {
        (
            crate::infrastructure::terminal::session::start_launcher_backed_test_shell(),
            Arc::new(ApplicationHandle::new(|task| task(), || {})),
        )
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "manual launcher-backed ConPTY detection regression"]
    fn launcher_backed_shell_is_idle_then_detects_and_clears_programs() {
        use crate::features::terminal::program::SustainedProgram;
        use crate::infrastructure::terminal::program::ProgramProbe;
        use std::time::{Duration, Instant};
        let (controller, application) = start_launcher_backed_test_shell();
        let console = controller.test_has_console();
        let mut startup_tracker = SustainedProgram::default();
        let mut startup_labels = Vec::new();
        for _ in 0..20 {
            let now = Instant::now();
            let candidate = controller
                .program_candidate(&ProgramProbe::new(), startup_tracker.sustained_pid(now));
            // Match app/shell: short-lived startup candidates never get labels.
            if let Some(label) = startup_tracker.observe(candidate, now) {
                startup_labels.push(label);
            }
            thread::sleep(Duration::from_millis(100));
        }
        let mut tracker = SustainedProgram::default();
        controller.write(b"ping.exe -n 4 127.0.0.1 >/dev/null\r");
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut detected = false;
        let mut cleared = false;
        while Instant::now() < deadline {
            let now = Instant::now();
            let candidate =
                controller.program_candidate(&ProgramProbe::new(), tracker.sustained_pid(now));
            detected |= tracker
                .observe(candidate.clone(), now)
                .is_some_and(|name| name.eq_ignore_ascii_case("ping"));
            if detected && candidate.is_none() {
                cleared = true;
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        controller.terminate(&application);
        assert!(
            console,
            "observer must attach to the launcher-backed console"
        );
        assert!(
            startup_labels.is_empty(),
            "startup processes must never become applications: {startup_labels:?}"
        );
        assert!(detected && cleared);
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "manual Git Bash process replacement smoke test"]
    fn exited_shell_launchers_do_not_hide_unknown_applications() {
        use crate::features::terminal::program::SustainedProgram;
        use crate::infrastructure::terminal::program::ProgramProbe;
        use std::time::{Duration, Instant};

        let name = format!("loom-session-probe-{}", std::process::id());
        let executable = env::temp_dir().join(format!("{name}.exe"));
        let source = executable.with_extension("rs");
        std::fs::write(&source, r#"fn main() {
            if std::env::args().nth(1).as_deref() == Some("--child") {
                std::thread::sleep(std::time::Duration::from_secs(4));
            } else {
                std::process::Command::new(std::env::current_exe().unwrap()).arg("--child").spawn().unwrap();
            }
        }"#).unwrap();
        let built = std::process::Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let (controller, application) = start_launcher_backed_test_shell();
        assert!(
            controller.test_has_console(),
            "console observer should be available"
        );
        let command = format!("\"{}\"\r", executable.to_string_lossy().replace('\\', "/"));
        controller.write(command.as_bytes());
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut tracker = SustainedProgram::default();
        let mut identified = false;
        let mut cleared = false;
        let mut reproduced_missing_parent = false;
        while Instant::now() < deadline {
            let now = Instant::now();
            let probe = ProgramProbe::new();
            let candidate = controller.program_candidate(&probe, tracker.sustained_pid(now));
            if candidate.as_ref().is_some_and(|app| app.name == name) {
                let pid = controller.test_pid();
                reproduced_missing_parent |= probe
                    .candidate(pid, None)
                    .is_none_or(|entry| entry.name != name);
            }
            let label = tracker.observe(candidate.clone(), now);
            identified |= label.as_deref() == Some(name.as_str());
            if identified && candidate.is_none() && label.is_none() {
                cleared = true;
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        controller.terminate(&application);
        std::fs::remove_file(executable).unwrap();
        std::fs::remove_file(source).unwrap();
        assert!(
            reproduced_missing_parent,
            "the application must remain invisible to ancestry-only detection"
        );
        assert!(
            identified,
            "the arbitrarily named application should be identified"
        );
        assert!(cleared, "the application name should clear after exit");
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "manual Windows ConPTY process-name smoke test"]
    fn running_application_is_detected_and_cleared_in_each_real_shell() {
        use crate::features::terminal::program::SustainedProgram;
        use crate::infrastructure::terminal::program::ProgramProbe;
        use std::time::{Duration, Instant};

        for shell in ShellKind::ALL {
            let controller = TerminalController::with_shell(shell);
            let application = Arc::new(ApplicationHandle::new(|task| task(), || {}));
            controller.restart(
                shell,
                None,
                TerminalSize {
                    rows: 12,
                    cols: 80,
                    pixel_width: 560,
                    pixel_height: 228,
                },
                application.clone(),
            );
            let command: &[u8] = match shell {
                ShellKind::PowerShell => b"ping.exe -n 4 127.0.0.1 | Out-Null\r",
                ShellKind::Cmd => b"ping.exe -n 4 127.0.0.1 >nul\r",
                ShellKind::Bash => b"ping.exe -n 4 127.0.0.1 >/dev/null\r",
                ShellKind::System | ShellKind::Custom(_) => unreachable!("Windows builtin test"),
            };
            controller.write(command);
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut tracker = SustainedProgram::default();
            let mut saw_program = false;
            let mut returned_to_idle = false;
            while Instant::now() < deadline {
                let probe = ProgramProbe::new();
                let candidate =
                    controller.program_candidate(&probe, tracker.sustained_pid(Instant::now()));
                let name = tracker.observe(candidate.clone(), Instant::now());
                if name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case("ping"))
                {
                    saw_program = true;
                }
                if saw_program && candidate.is_none() && name.is_none() {
                    returned_to_idle = true;
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            controller.terminate(&application);
            assert!(
                saw_program,
                "{} did not identify its sustained application",
                shell.label()
            );
            assert!(
                returned_to_idle,
                "{} did not clear its application after exit",
                shell.label()
            );
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "manual Windows ConPTY smoke test"]
    fn supported_shells_round_trip_through_real_conpty_sessions() {
        assert_shell_round_trip(ShellKind::PowerShell, b"Write-Output (6*7)\r", "42");
        assert_shell_round_trip(
            ShellKind::PowerShell,
            "Write-Output '中文输入测试'\r".as_bytes(),
            "中文输入测试",
        );
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
