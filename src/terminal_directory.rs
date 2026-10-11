//! Shell directory reports, independent of rendered prompt text. OSC 9;9 is
//! also used by Windows Terminal. Keep parsing state across PTY read boundaries.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::terminal_session::ShellKind;

#[derive(Default)]
pub struct DirectoryReports {
    escape: bool,
    osc: Option<Vec<u8>>,
    oversized: bool,
}

impl DirectoryReports {
    pub fn process(&mut self, bytes: &[u8]) -> Option<PathBuf> {
        let mut latest = None;
        for &byte in bytes {
            if let Some(osc) = &mut self.osc {
                if byte == 7 || (self.escape && byte == b'\\') {
                    if !self.oversized {
                        latest = report_path(osc).or(latest);
                    }
                    self.osc = None;
                    self.escape = false;
                    self.oversized = false;
                } else if byte == 0x18 || byte == 0x1a {
                    self.osc = None;
                    self.escape = false;
                    self.oversized = false;
                } else {
                    if self.escape {
                        // An ESC not followed by ST cancels the malformed OSC.
                        self.osc = (byte == b']').then(Vec::new);
                        self.oversized = false;
                    } else if byte != 0x1b && !self.oversized {
                        if osc.len() < 32 * 1024 {
                            osc.push(byte);
                        } else {
                            osc.clear();
                            self.oversized = true;
                        }
                    }
                    self.escape = byte == 0x1b;
                }
            } else {
                if self.escape && byte == b']' {
                    self.osc = Some(Vec::new());
                }
                self.escape = byte == 0x1b;
            }
        }
        latest
    }
}

fn report_path(bytes: &[u8]) -> Option<PathBuf> {
    let value = std::str::from_utf8(bytes).ok()?.strip_prefix("9;9;")?;
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value);
    if value.is_empty() || value.chars().any(char::is_control) {
        return None;
    }
    let path = PathBuf::from(value);
    path.is_absolute().then_some(path)
}

/// Validate at process start (not on the UI thread for every output frame).
pub fn starting_directory(requested: Option<&Path>, fallback: Option<&Path>) -> Option<PathBuf> {
    requested
        .filter(|path| path.is_absolute() && path.is_dir())
        .map(Path::to_path_buf)
        .or_else(|| {
            fallback
                .filter(|path| path.is_absolute() && path.is_dir())
                .map(Path::to_path_buf)
        })
        .or_else(|| std::env::current_dir().ok())
}

pub struct ShellIntegration {
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    script: Option<PathBuf>,
}

impl ShellIntegration {
    pub fn new(shell: ShellKind, cwd: Option<&Path>) -> std::io::Result<Self> {
        let mut integration = Self {
            args: Vec::new(),
            env: Vec::new(),
            script: None,
        };
        if let Some(cwd) = cwd {
            integration
                .env
                .push(("LOOM_TERMINAL_CWD".into(), cwd.as_os_str().to_owned()));
        }
        match shell {
            ShellKind::PowerShell => {
                use base64::Engine;
                let bytes = POWERSHELL
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<_>>();
                integration.args = ["-NoLogo", "-NoExit", "-EncodedCommand"]
                    .map(OsString::from)
                    .to_vec();
                integration.args.push(
                    base64::engine::general_purpose::STANDARD
                        .encode(bytes)
                        .into(),
                );
            }
            ShellKind::Cmd => {
                let prompt = std::env::var("PROMPT").unwrap_or_else(|_| "$P$G".into());
                integration
                    .env
                    .push(("PROMPT".into(), format!(r"$E]9;9;$P$E\{prompt}").into()));
                // CMD starts in the process cwd; no command interpolation is needed.
            }
            ShellKind::Bash => {
                static NEXT: AtomicU64 = AtomicU64::new(1);
                let path = std::env::temp_dir().join(format!(
                    "loom-bash-{}-{}.sh",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                // create_new avoids following an existing path in a shared temp directory.
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options.open(&path)?;
                integration.script = Some(path.clone());
                use std::io::Write;
                file.write_all(BASH.as_bytes())?;
                let path = path.to_string_lossy().replace('\\', "/");
                integration.args = vec!["--rcfile".into(), path.into(), "-i".into()];
            }
        }
        Ok(integration)
    }
}

impl Drop for ShellIntegration {
    fn drop(&mut self) {
        if let Some(path) = &self.script {
            let _ = std::fs::remove_file(path);
        }
    }
}

const POWERSHELL: &str = r#"
if ($env:LOOM_TERMINAL_CWD) {
    Set-Location -LiteralPath $env:LOOM_TERMINAL_CWD
    Remove-Item Env:LOOM_TERMINAL_CWD
}
$global:__LoomOriginalPrompt = $function:prompt
function global:prompt {
    $loomPrompt = & $global:__LoomOriginalPrompt
    if ($PWD.Provider.Name -eq 'FileSystem') {
        $Host.UI.Write(([char]27).ToString() + ']9;9;"' + $PWD.ProviderPath + '"' + [char]7)
    }
    $loomPrompt
}
"#;

// Read the same profiles as the former --login launch, then install the hook
// after custom prompt setup. No user profile files are modified.
const BASH: &str = r#"
if [ -r /etc/profile ]; then . /etc/profile; fi
for __loom_profile in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
    if [ -r "$__loom_profile" ]; then . "$__loom_profile"; break; fi
done
unset __loom_profile
if [ -n "$LOOM_TERMINAL_CWD" ]; then
    __loom_cwd=$LOOM_TERMINAL_CWD
    if command -v cygpath >/dev/null 2>&1; then __loom_cwd=$(cygpath -u "$__loom_cwd"); fi
    builtin cd -- "$__loom_cwd"
    unset __loom_cwd LOOM_TERMINAL_CWD
fi
__loom_report_cwd() {
    local __loom_status=$? __loom_path=$PWD
    if command -v cygpath >/dev/null 2>&1; then __loom_path=$(cygpath -w "$PWD"); fi
    builtin printf '\033]9;9;%s\007' "$__loom_path"
    return "$__loom_status"
}
if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == 'declare -a '* ]]; then
    PROMPT_COMMAND=(__loom_report_cwd "${PROMPT_COMMAND[@]}")
else
    PROMPT_COMMAND="__loom_report_cwd${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
fi
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_or_missing_saved_directories_use_workspace_then_process_directory() {
        let workspace = std::env::temp_dir();
        let missing = workspace.join(format!("loom-missing-cwd-{}", std::process::id()));
        assert_eq!(
            starting_directory(Some(&workspace), Some(&missing)),
            Some(workspace.clone())
        );
        for requested in [None, Some(missing.as_path()), Some(Path::new("relative"))] {
            assert_eq!(
                starting_directory(requested, Some(&workspace)),
                Some(workspace.clone())
            );
        }
        assert_eq!(
            starting_directory(Some(&missing), None),
            std::env::current_dir().ok()
        );
    }

    #[test]
    fn reports_survive_every_possible_read_boundary() {
        let cwd = std::env::temp_dir().join("中文 folder;with spaces");
        for end in ["\x07", "\x1b\\"] {
            let report = format!("text\x1b]9;9;\"{}\"{end}prompt", cwd.display());
            for split in 0..=report.len() {
                let mut parser = DirectoryReports::default();
                let first = parser.process(&report.as_bytes()[..split]);
                let last = parser.process(&report.as_bytes()[split..]);
                assert_eq!(last.or(first), Some(cwd.clone()), "split {split}");
            }
        }
    }

    #[test]
    fn rejects_invalid_reports_and_recovers_after_oversized_or_cancelled_osc() {
        let cwd = std::env::temp_dir();
        let valid = format!("\x1b]9;9;{}\x07", cwd.display());
        for invalid in [
            b"\x1b]0;title\x07".to_vec(),
            b"\x1b]9;9;relative\x07".to_vec(),
            b"\x1b]9;9;\xff\x07".to_vec(),
            b"\x1b]9;9;\x18".to_vec(),
            format!("\x1b]9;9;{}\x07", "a".repeat(40_000)).into_bytes(),
        ] {
            let mut parser = DirectoryReports::default();
            assert_eq!(parser.process(&invalid), None);
            assert_eq!(parser.process(valid.as_bytes()), Some(cwd.clone()));
        }
    }
}
