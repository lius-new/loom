//! Platform defaults and user-defined shell profiles. Discovery never launches
//! a program and custom arguments are passed directly, without shell expansion.

use crate::features::terminal::ShellKind;
use std::env;
use std::path::{Path, PathBuf};

use crate::features::terminal::profiles::{ShellOption, ShellProfile, system_shell_from};

impl ShellOption {
    pub fn custom(profile: &ShellProfile) -> Self {
        Self {
            kind: ShellKind::Custom(profile.id),
            name: profile.name.clone(),
            program: resolve_program(&profile.program),
            args: profile.args.clone(),
        }
    }
}

pub fn options(profiles: &[ShellProfile]) -> Vec<ShellOption> {
    let mut options = defaults();
    options.extend(profiles.iter().map(ShellOption::custom));
    options
}

pub fn defaults() -> Vec<ShellOption> {
    #[cfg(windows)]
    let kinds = &ShellKind::ALL[..];
    #[cfg(not(windows))]
    let kinds = &[ShellKind::System][..];
    kinds.iter().filter_map(|kind| builtin(*kind)).collect()
}

pub fn builtin(kind: ShellKind) -> Option<ShellOption> {
    let program = match kind {
        ShellKind::PowerShell => ["pwsh.exe", "powershell.exe", "pwsh", "powershell"]
            .iter()
            .find_map(|p| resolve_program(Path::new(p))),
        ShellKind::Cmd => {
            let comspec = env::var_os("COMSPEC")
                .map(PathBuf::from)
                .and_then(|p| executable(&p).then_some(p));
            comspec
                .or_else(|| resolve_program(Path::new("cmd.exe")))
                .or_else(|| resolve_program(Path::new("cmd")))
        }
        ShellKind::Bash => {
            #[cfg(windows)]
            let git = crate::terminal_session::find_git_bash();
            #[cfg(not(windows))]
            let git = None;
            git.or_else(|| resolve_program(Path::new("bash.exe")))
                .or_else(|| resolve_program(Path::new("bash")))
        }
        ShellKind::System => system_shell(),
        ShellKind::Custom(_) => None,
    }?;
    Some(ShellOption {
        kind,
        name: kind.label().into(),
        program: Some(program),
        args: Vec::new(),
    })
}

pub fn resolve_program(program: &Path) -> Option<PathBuf> {
    if program.as_os_str().is_empty() {
        return None;
    }
    if program.is_absolute() || program.components().count() > 1 {
        return executable(program).then(|| program.to_path_buf());
    }
    env::split_paths(&env::var_os("PATH")?).find_map(|directory| {
        let path = directory.join(program);
        if executable(&path) {
            return Some(path);
        }
        #[cfg(windows)]
        if path.extension().is_none() {
            let path = path.with_extension("exe");
            if executable(&path) {
                return Some(path);
            }
        }
        None
    })
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return metadata.permissions().mode() & 0o111 != 0;
    }
    #[cfg(not(unix))]
    {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("com"))
    }
}

fn system_shell() -> Option<PathBuf> {
    system_shell_from(
        env::var_os("SHELL").map(PathBuf::from),
        login_shell(),
        executable,
    )
}

#[cfg(unix)]
fn login_shell() -> Option<PathBuf> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;
    let mut size = 4096;
    loop {
        let mut buffer = vec![0u8; size];
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        // getpwuid_r owns no shared static storage; copy pw_shell before the
        // buffer and passwd record are dropped.
        let error = unsafe {
            libc::getpwuid_r(
                libc::getuid(),
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if error == libc::ERANGE && size < 1_048_576 {
            size *= 2;
            continue;
        }
        if error != 0 || result.is_null() {
            return None;
        }
        let shell = unsafe { (*result).pw_shell };
        if shell.is_null() {
            return None;
        }
        let bytes = unsafe { CStr::from_ptr(shell) }.to_bytes();
        return Some(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)));
    }
}

#[cfg(not(unix))]
fn login_shell() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::terminal::profiles::effective_default;

    #[test]
    fn unavailable_custom_default_falls_back_without_dropping_its_profile() {
        let profile = ShellProfile {
            id: 42,
            name: "Missing shell".into(),
            program: PathBuf::from("/missing-loom-shell"),
            args: vec![],
        };
        let missing = ShellOption::custom(&profile);
        assert!(!missing.available());
        let available = ShellOption {
            kind: ShellKind::System,
            name: "System default".into(),
            program: Some("/bin/sh".into()),
            args: vec![],
        };
        assert_eq!(
            effective_default(ShellKind::Custom(42), &[missing.clone(), available]),
            Some(ShellKind::System)
        );
        assert_eq!(effective_default(ShellKind::Custom(42), &[missing]), None);
    }
}
