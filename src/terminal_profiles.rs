//! Platform defaults and user-defined shell profiles. Discovery never launches
//! a program and custom arguments are passed directly, without shell expansion.

use crate::terminal_session::ShellKind;
use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ShellProfile {
    pub id: u64,
    pub name: String,
    pub program: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellOption {
    pub kind: ShellKind,
    pub name: String,
    pub program: Option<PathBuf>,
    pub args: Vec<String>,
}

impl ShellOption {
    pub fn custom(profile: &ShellProfile) -> Self {
        Self {
            kind: ShellKind::Custom(profile.id),
            name: profile.name.clone(),
            program: resolve_program(&profile.program),
            args: profile.args.clone(),
        }
    }

    pub fn available(&self) -> bool {
        self.program.is_some()
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

pub fn effective_default(selected: ShellKind, options: &[ShellOption]) -> Option<ShellKind> {
    options
        .iter()
        .find(|option| option.kind == selected && option.available())
        .or_else(|| options.iter().find(|option| option.available()))
        .map(|option| option.kind)
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

fn system_shell_from(
    shell: Option<PathBuf>,
    login: Option<PathBuf>,
    available: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    shell
        .into_iter()
        .chain(login)
        .chain([PathBuf::from("/bin/sh")])
        .find(|path| path.has_root() && available(path))
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

/// A small argument editor grammar: spaces separate arguments, quotes group
/// them, and backslashes escape quotes/backslashes inside double quotes.
pub fn parse_arguments(value: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut started = false;
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\0' {
            return Err("Arguments cannot contain a null character.".into());
        }
        match quote {
            Some(q) if ch == q => quote = None,
            Some('"') if ch == '\\' && matches!(chars.peek(), Some('"' | '\\')) => {
                current.push(chars.next().unwrap())
            }
            Some(_) => current.push(ch),
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                started = true;
            }
            None if ch.is_whitespace() => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            None => {
                current.push(ch);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err("Close the quotation mark in the arguments.".into());
    }
    if started {
        args.push(current);
    }
    Ok(args)
}

pub fn format_arguments(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            // Backslashes are literal outside quotes, so plain paths stay readable.
            if !arg.is_empty()
                && !arg.contains(|c: char| c.is_whitespace() || c == '"' || c == '\'')
            {
                arg.clone()
            } else {
                format!("\"{}\"", arg.replace('\\', "\\\\").replace('"', "\\\""))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unix_default_follows_user_shell_then_login_shell_then_sh() {
        let shell = Some(PathBuf::from("/custom/fish"));
        let login = Some(PathBuf::from("/bin/zsh"));
        assert_eq!(
            system_shell_from(shell.clone(), login.clone(), |_| true),
            shell
        );
        assert_eq!(
            system_shell_from(shell, login.clone(), |p| p != Path::new("/custom/fish")),
            login
        );
        assert_eq!(
            system_shell_from(None, None, |_| true),
            Some(PathBuf::from("/bin/sh"))
        );
        assert_eq!(
            system_shell_from(Some("relative".into()), None, |_| false),
            None
        );
    }
    #[test]
    fn argument_editor_preserves_empty_values_quotes_windows_paths_and_literals() {
        let args = vec![
            "--login".into(),
            "".into(),
            r"C:\Program Files\shell\".into(),
            "a\"b'c".into(),
            "$(echo never-expanded)".into(),
        ];
        assert_eq!(parse_arguments(&format_arguments(&args)).unwrap(), args);
        assert_eq!(
            parse_arguments(r"--path C:\shell\bin --name 'hello world'").unwrap(),
            vec!["--path", r"C:\shell\bin", "--name", "hello world"]
        );
        assert!(parse_arguments("'unclosed").is_err());
    }
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
