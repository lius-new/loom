//! Shell DTOs, argument grammar and selection from supplied candidates.
use crate::features::terminal::ShellKind;
use serde::{Deserialize, Serialize};
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
    pub fn available(&self) -> bool {
        self.program.is_some()
    }
}

pub fn effective_default(selected: ShellKind, options: &[ShellOption]) -> Option<ShellKind> {
    options
        .iter()
        .find(|option| option.kind == selected && option.available())
        .or_else(|| options.iter().find(|option| option.available()))
        .map(|option| option.kind)
}

pub(crate) fn system_shell_from(
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
}
