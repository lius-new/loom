//! Which server instance a document belongs to.
//!
//! An instance is a server definition run on a project root with a particular
//! launch (program, arguments, options). Documents of any language the server
//! serves share it, and a settings change that alters the launch makes a different
//! instance, so the old one is retired rather than patched.
use super::root;
use crate::domain::language::{Registry, ResolvedLanguage, ServerDefinitionId};
use crate::features::settings::servers::{LanguageServerSetting, ServerLaunch, launch_for};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceKey {
    pub definition: ServerDefinitionId,
    pub root: PathBuf,
    pub launch: ServerLaunch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub key: InstanceKey,
    /// The `languageId` the server expects for this document.
    pub language_id: String,
}

/// The instance for a file, or `None` when its language has no server, the user
/// turned the server off, or the server's root policy says this file gets none.
pub fn assign(
    registry: &Registry,
    language: ResolvedLanguage,
    path: &Path,
    workspace_roots: &[PathBuf],
    settings: &[LanguageServerSetting],
    has_entry: impl Fn(&Path, &str) -> bool,
) -> Option<Assignment> {
    let (definition, language_id) = registry.server_for(language)?;
    let launch = launch_for(registry, definition.id, settings)?;
    // The innermost open folder that holds the file bounds the root search.
    let workspace = workspace_roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count());
    let root = root::resolve(
        &definition.root,
        path,
        workspace.map(PathBuf::as_path),
        has_entry,
    )?;
    Some(Assignment {
        key: InstanceKey {
            definition: definition.id,
            root,
            launch,
        },
        language_id: language_id.to_owned(),
    })
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

    #[test]
    fn a_rust_file_belongs_to_the_outermost_cargo_folder_of_its_workspace() {
        let roots = [PathBuf::from("/w")];
        let found = assign(
            Registry::builtin(),
            rust(),
            Path::new("/w/crates/a/src/lib.rs"),
            &roots,
            &[],
            disk(&["/w|Cargo.toml", "/w/crates/a|Cargo.toml"]),
        )
        .unwrap();
        assert_eq!(found.key.root, PathBuf::from("/w"));
        assert_eq!(found.key.launch.command, "rust-analyzer");
        assert_eq!(found.language_id, "rust");
    }

    #[test]
    fn files_of_one_project_share_a_key_and_other_projects_do_not() {
        let roots = [PathBuf::from("/w"), PathBuf::from("/x")];
        let entries = ["/w|Cargo.toml", "/x|Cargo.toml"];
        let key = |path: &str| {
            assign(
                Registry::builtin(),
                rust(),
                Path::new(path),
                &roots,
                &[],
                disk(&entries),
            )
            .unwrap()
            .key
        };
        assert_eq!(key("/w/src/a.rs"), key("/w/src/deep/b.rs"));
        assert_ne!(key("/w/src/a.rs"), key("/x/src/a.rs"));
    }

    #[test]
    fn a_rust_file_with_no_cargo_project_runs_on_its_own_folder() {
        let found = assign(
            Registry::builtin(),
            rust(),
            Path::new("/loose/a.rs"),
            &[],
            &[],
            disk(&[]),
        )
        .unwrap();
        assert_eq!(found.key.root, PathBuf::from("/loose"));
    }

    #[test]
    fn languages_without_a_server_get_none() {
        let registry = Registry::builtin();
        let text = registry.plain_text();
        assert_eq!(
            assign(registry, text, Path::new("/w/a.txt"), &[], &[], disk(&[])),
            None
        );
    }

    #[test]
    fn turning_a_server_off_removes_its_assignments() {
        let off = [setting(
            r#"{ "server": "rust-analyzer", "enabled": false }"#,
        )];
        assert_eq!(
            assign(
                Registry::builtin(),
                rust(),
                Path::new("/w/a.rs"),
                &[],
                &off,
                disk(&[])
            ),
            None
        );
    }

    #[test]
    fn changing_the_launch_changes_the_key() {
        let plain = assign(
            Registry::builtin(),
            rust(),
            Path::new("/w/a.rs"),
            &[],
            &[],
            disk(&[]),
        )
        .unwrap();
        let other = [setting(
            r#"{ "server": "rust-analyzer", "command": "ra2" }"#,
        )];
        let changed = assign(
            Registry::builtin(),
            rust(),
            Path::new("/w/a.rs"),
            &[],
            &other,
            disk(&[]),
        )
        .unwrap();
        assert_ne!(plain.key, changed.key);
        assert_eq!(plain.key.root, changed.key.root);
    }

    #[test]
    fn the_innermost_workspace_folder_bounds_the_search() {
        let roots = [PathBuf::from("/w"), PathBuf::from("/w/inner")];
        let found = assign(
            Registry::builtin(),
            rust(),
            Path::new("/w/inner/src/a.rs"),
            &roots,
            &[],
            disk(&["/w|Cargo.toml", "/w/inner|Cargo.toml"]),
        )
        .unwrap();
        assert_eq!(found.key.root, PathBuf::from("/w/inner"));
    }
}
