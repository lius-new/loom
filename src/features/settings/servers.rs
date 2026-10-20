//! The `language_servers` setting: per-server overrides of the built-in
//! definitions.
//!
//! Only the user's `settings.json` may hold it. A project file that could choose
//! which program Loom runs would turn opening a folder into running its code.
use crate::features::language_service::server_set::{ServerDefinition, ServerSet};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One server's overrides. A server with no entry runs as its definition says.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LanguageServerSetting {
    /// The definition's name, such as `rust-analyzer`.
    pub server: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    /// Another program to run, for example a full path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Replaces the definition's arguments, including with an empty list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// Replaces the definition's `initializationOptions`; must be an object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initialization_options: Option<Value>,
}

fn enabled() -> bool {
    true
}

impl LanguageServerSetting {
    /// Why this entry cannot be used, if it cannot.
    pub fn validate(&self, servers: &ServerSet) -> Result<(), String> {
        if servers.get(&self.server).is_none() {
            return Err(format!("unknown language server \"{}\"", self.server));
        }
        if self.command.as_deref().is_some_and(|c| c.trim().is_empty()) {
            return Err("command is empty".to_owned());
        }
        if self
            .args
            .as_ref()
            .is_some_and(|a| a.iter().any(|x| x.contains('\0')))
        {
            return Err("args contain a NUL character".to_owned());
        }
        if self
            .initialization_options
            .as_ref()
            .is_some_and(|o| !o.is_object())
        {
            return Err("initialization_options must be an object".to_owned());
        }
        Ok(())
    }
}

/// What actually runs for one server once the definition and the user's override
/// are combined. Two equal values start the same process the same way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerLaunch {
    pub command: String,
    pub args: Vec<String>,
    /// Compact JSON object text.
    pub initialization_options: Option<String>,
}

/// The launch for `base`, or `None` when the user turned it off.
pub fn launch_for(
    base: &ServerDefinition,
    settings: &[LanguageServerSetting],
) -> Option<ServerLaunch> {
    // The last entry for a server wins, like a later key in a settings file.
    let over = settings.iter().rev().find(|s| s.server == *base.name);
    if over.is_some_and(|s| !s.enabled) {
        return None;
    }
    Some(ServerLaunch {
        command: over
            .and_then(|s| s.command.clone())
            .unwrap_or_else(|| base.command.to_string()),
        args: over
            .and_then(|s| s.args.clone())
            .unwrap_or_else(|| base.args.iter().map(|a| a.to_string()).collect()),
        initialization_options: match over.and_then(|s| s.initialization_options.as_ref()) {
            Some(options) => Some(options.to_string()),
            None => base.initialization_options.as_deref().map(str::to_owned),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn servers() -> std::sync::Arc<ServerSet> {
        ServerSet::builtin()
    }

    fn run(settings: &[LanguageServerSetting]) -> Option<ServerLaunch> {
        launch_for(servers().get("rust-analyzer").unwrap(), settings)
    }

    fn setting(json: &str) -> LanguageServerSetting {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn no_entry_runs_the_definition_as_written() {
        let launch = run(&[]).unwrap();
        assert_eq!(launch.command, "rust-analyzer");
        assert!(launch.args.is_empty());
        assert_eq!(launch.initialization_options, None);
    }

    #[test]
    fn an_entry_overrides_only_what_it_names() {
        let entries = [setting(
            r#"{ "server": "rust-analyzer", "command": "C:\\tools\\ra.exe" }"#,
        )];
        let launch = run(&entries).unwrap();
        assert_eq!(launch.command, "C:\\tools\\ra.exe");
        assert!(launch.args.is_empty());

        let entries = [setting(
            r#"{ "server": "rust-analyzer", "args": ["--log-file", "x"],
                 "initialization_options": { "checkOnSave": false } }"#,
        )];
        let launch = run(&entries).unwrap();
        assert_eq!(launch.command, "rust-analyzer");
        assert_eq!(launch.args, ["--log-file", "x"]);
        assert_eq!(
            launch.initialization_options.as_deref(),
            Some(r#"{"checkOnSave":false}"#)
        );
    }

    #[test]
    fn disabling_wins_and_the_last_entry_counts() {
        let off = [setting(
            r#"{ "server": "rust-analyzer", "enabled": false }"#,
        )];
        assert_eq!(run(&off), None);
        let on_again = [
            setting(r#"{ "server": "rust-analyzer", "enabled": false }"#),
            setting(r#"{ "server": "rust-analyzer", "command": "ra" }"#),
        ];
        assert_eq!(run(&on_again).unwrap().command, "ra");
    }

    #[test]
    fn empty_args_replace_the_default_args() {
        let entries = [setting(r#"{ "server": "rust-analyzer", "args": [] }"#)];
        assert!(run(&entries).unwrap().args.is_empty());
    }

    #[test]
    fn bad_entries_are_explained() {
        let set = servers();
        let check = |json: &str| {
            serde_json::from_str::<LanguageServerSetting>(json)
                .unwrap()
                .validate(&set)
        };
        assert!(check(r#"{ "server": "rust-analyzer" }"#).is_ok());
        assert!(
            check(r#"{ "server": "nope" }"#)
                .unwrap_err()
                .contains("unknown language server")
        );
        assert!(
            check(r#"{ "server": "rust-analyzer", "command": "  " }"#)
                .unwrap_err()
                .contains("command")
        );
        assert!(
            check(r#"{ "server": "rust-analyzer", "initialization_options": [] }"#)
                .unwrap_err()
                .contains("object")
        );
        assert!(
            serde_json::from_str::<LanguageServerSetting>(r#"{ "server": "x", "extra": 1 }"#)
                .is_err()
        );
    }

    #[test]
    fn defaults_are_not_written_back() {
        let json = serde_json::to_string(&setting(r#"{ "server": "rust-analyzer" }"#)).unwrap();
        assert_eq!(json, r#"{"server":"rust-analyzer","enabled":true}"#);
    }
}
