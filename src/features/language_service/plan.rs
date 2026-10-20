//! Which server instances a document belongs to.
//!
//! An instance is a server definition run on a project root with a particular
//! launch (program, arguments, options). Documents of any language the server
//! serves share it, and a settings change that alters the launch makes a different
//! instance, so the old one is retired rather than patched. A document has one
//! assignment for each server its language is bound to; each server finds its own
//! project root, so one file can belong to instances on different roots.
use super::root;
use super::server_set::ServerSet;
use crate::domain::language::{Registry, ResolvedLanguage};
use crate::features::settings::servers::{LanguageServerSetting, ServerLaunch, launch_for};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceKey {
    /// The server definition's name.
    pub server: Arc<str>,
    pub root: PathBuf,
    pub launch: ServerLaunch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub key: InstanceKey,
    /// The `languageId` the server expects for this document.
    pub language_id: String,
}

/// The instances for a file, in the servers' listing order. A server is left out
/// when the user turned it off or its root policy says this file gets none, so the
/// result is empty for a language no server is bound to.
pub fn assign(
    servers: &ServerSet,
    registry: &Registry,
    language: ResolvedLanguage,
    path: &Path,
    workspace_roots: &[PathBuf],
    settings: &[LanguageServerSetting],
    has_entry: impl Fn(&Path, &str) -> bool,
) -> Vec<Assignment> {
    // The innermost open folder that holds the file bounds the root search.
    let workspace = workspace_roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count());
    servers
        .for_language(registry, language)
        .into_iter()
        .filter_map(|(definition, language_id)| {
            let launch = launch_for(definition, settings)?;
            let root = root::resolve(
                &definition.root,
                path,
                workspace.map(PathBuf::as_path),
                &has_entry,
            )?;
            Some(Assignment {
                key: InstanceKey {
                    server: definition.name.clone(),
                    root,
                    launch,
                },
                language_id: language_id.to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn disk(entries: &[&str]) -> impl Fn(&Path, &str) -> bool {
        let set: HashSet<String> = entries.iter().map(|e| e.to_string()).collect();
        move |dir, name| set.contains(&format!("{}|{name}", dir.display()))
    }

    fn rust() -> ResolvedLanguage {
        let registry = Registry::builtin();
        registry.resolve_path(Path::new("a.rs"))
    }

    fn setting(json: &str) -> LanguageServerSetting {
        serde_json::from_str(json).unwrap()
    }

    /// The single assignment of a one-server language.
    fn only(
        language: ResolvedLanguage,
        path: &str,
        roots: &[PathBuf],
        settings: &[LanguageServerSetting],
        entries: &[&str],
    ) -> Option<Assignment> {
        let found = assign(
            &ServerSet::builtin(),
            Registry::builtin(),
            language,
            Path::new(path),
            roots,
            settings,
            disk(entries),
        );
        assert!(found.len() <= 1, "{found:?}");
        found.into_iter().next()
    }

    #[test]
    fn a_rust_file_belongs_to_the_outermost_cargo_folder_of_its_workspace() {
        let roots = [PathBuf::from("/w")];
        let found = only(
            rust(),
            "/w/crates/a/src/lib.rs",
            &roots,
            &[],
            &["/w|Cargo.toml", "/w/crates/a|Cargo.toml"],
        )
        .unwrap();
        assert_eq!(found.key.root, PathBuf::from("/w"));
        assert_eq!(&*found.key.server, "rust-analyzer");
        assert_eq!(found.key.launch.command, "rust-analyzer");
        assert_eq!(found.language_id, "rust");
    }

    #[test]
    fn files_of_one_project_share_a_key_and_other_projects_do_not() {
        let roots = [PathBuf::from("/w"), PathBuf::from("/x")];
        let entries = ["/w|Cargo.toml", "/x|Cargo.toml"];
        let key = |path: &str| only(rust(), path, &roots, &[], &entries).unwrap().key;
        assert_eq!(key("/w/src/a.rs"), key("/w/src/deep/b.rs"));
        assert_ne!(key("/w/src/a.rs"), key("/x/src/a.rs"));
    }

    #[test]
    fn a_rust_file_with_no_cargo_project_runs_on_its_own_folder() {
        let found = only(rust(), "/loose/a.rs", &[], &[], &[]).unwrap();
        assert_eq!(found.key.root, PathBuf::from("/loose"));
    }

    #[test]
    fn languages_without_a_server_get_none() {
        let registry = Registry::builtin();
        let text = registry.plain_text();
        assert_eq!(only(text, "/w/a.txt", &[], &[], &[]), None);
    }

    #[test]
    fn turning_a_server_off_removes_its_assignments() {
        let off = [setting(
            r#"{ "server": "rust-analyzer", "enabled": false }"#,
        )];
        assert_eq!(only(rust(), "/w/a.rs", &[], &off, &[]), None);
    }

    #[test]
    fn changing_the_launch_changes_the_key() {
        let plain = only(rust(), "/w/a.rs", &[], &[], &[]).unwrap();
        let other = [setting(
            r#"{ "server": "rust-analyzer", "command": "ra2" }"#,
        )];
        let changed = only(rust(), "/w/a.rs", &[], &other, &[]).unwrap();
        assert_ne!(plain.key, changed.key);
        assert_eq!(plain.key.root, changed.key.root);
    }

    #[test]
    fn the_innermost_workspace_folder_bounds_the_search() {
        let roots = [PathBuf::from("/w"), PathBuf::from("/w/inner")];
        let found = only(
            rust(),
            "/w/inner/src/a.rs",
            &roots,
            &[],
            &["/w|Cargo.toml", "/w/inner|Cargo.toml"],
        )
        .unwrap();
        assert_eq!(found.key.root, PathBuf::from("/w/inner"));
    }

    fn two_rust_servers() -> ServerSet {
        ServerSet::from_sources(&[
            (
                "checker",
                r#"{ "name": "checker", "display_name": "Checker", "command": "checker",
                    "root": { "markers": ["checker.toml"], "strategy": "nearest",
                              "fallback": "workspace" },
                    "languages": [{ "language": "rust", "language_id": "rust-check" }] }"#,
            ),
            (
                "rust-analyzer",
                r#"{ "name": "rust-analyzer", "display_name": "rust-analyzer",
                    "command": "rust-analyzer", "priority": 10,
                    "root": { "markers": ["Cargo.toml"], "strategy": "outermost",
                              "fallback": "file_directory" },
                    "languages": [{ "language": "rust", "language_id": "rust" }] }"#,
            ),
        ])
        .unwrap()
    }

    fn assign_two(settings: &[LanguageServerSetting]) -> Vec<Assignment> {
        let roots = [PathBuf::from("/w")];
        assign(
            &two_rust_servers(),
            Registry::builtin(),
            rust(),
            Path::new("/w/crates/a/src/lib.rs"),
            &roots,
            settings,
            disk(&["/w|Cargo.toml", "/w/crates/a|checker.toml"]),
        )
    }

    #[test]
    fn a_language_with_two_servers_gets_two_assignments_in_listing_order() {
        let found = assign_two(&[]);
        let summary: Vec<_> = found
            .iter()
            .map(|a| (&*a.key.server, a.language_id.as_str(), a.key.root.clone()))
            .collect();
        assert_eq!(
            summary,
            [
                ("rust-analyzer", "rust", PathBuf::from("/w")),
                ("checker", "rust-check", PathBuf::from("/w/crates/a")),
            ],
            "each server finds its own root and speaks its own language id"
        );
    }

    #[test]
    fn turning_one_of_two_servers_off_keeps_the_other() {
        let off = [setting(r#"{ "server": "checker", "enabled": false }"#)];
        let found = assign_two(&off);
        assert_eq!(found.len(), 1);
        assert_eq!(&*found[0].key.server, "rust-analyzer");
    }
}
