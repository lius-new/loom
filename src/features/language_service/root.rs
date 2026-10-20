//! Which folder a language server should treat as the project for a file.
//!
//! The rule comes from the server definition (marker files, nearest or outermost,
//! and what to do when none is found). Looking at the disk is a closure the caller
//! supplies, so the rule is testable and the caller decides which thread does the
//! reading.
use super::server_set::{RootFallback, RootPolicy, RootStrategy};
use std::path::{Path, PathBuf};

/// The project root for `file`, or `None` when the definition says such files get
/// no server. `workspace` is the open folder that holds the file, if any; a search
/// never goes above it. `has_entry(dir, name)` says whether `dir` holds `name`.
pub fn resolve(
    policy: &RootPolicy,
    file: &Path,
    workspace: Option<&Path>,
    has_entry: impl Fn(&Path, &str) -> bool,
) -> Option<PathBuf> {
    let directory = file.parent()?;
    // Only folders from the file's own up to the workspace are looked at. A file
    // outside every workspace is judged by its own folder alone.
    let boundary = workspace.filter(|w| directory.starts_with(w));
    let mut candidates = Vec::new();
    for ancestor in directory.ancestors() {
        candidates.push(ancestor);
        if boundary.is_none_or(|b| ancestor == b) {
            break;
        }
    }
    let marked = |dir: &Path| policy.markers.iter().any(|m| has_entry(dir, m));
    let found = match policy.strategy {
        RootStrategy::Nearest => candidates.iter().find(|d| marked(d)),
        RootStrategy::Outermost => candidates.iter().rfind(|d| marked(d)),
    };
    if let Some(dir) = found {
        return Some(dir.to_path_buf());
    }
    match policy.fallback {
        RootFallback::Workspace => boundary.map(Path::to_path_buf),
        RootFallback::FileDirectory => Some(directory.to_path_buf()),
        RootFallback::None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn policy(strategy: RootStrategy, fallback: RootFallback) -> RootPolicy {
        RootPolicy {
            markers: vec!["Cargo.toml".into(), "rust-project.json".into()],
            strategy,
            fallback,
        }
    }

    /// A pretend disk: "dir|name" entries.
    fn disk(entries: &[&str]) -> impl Fn(&Path, &str) -> bool {
        let set: HashSet<String> = entries.iter().map(|e| e.to_string()).collect();
        move |dir, name| set.contains(&format!("{}|{name}", dir.display()))
    }

    fn root(
        policy: &RootPolicy,
        file: &str,
        workspace: Option<&str>,
        entries: &[&str],
    ) -> Option<String> {
        resolve(
            policy,
            Path::new(file),
            workspace.map(Path::new),
            disk(entries),
        )
        .map(|p| p.display().to_string())
    }

    const ENTRIES: &[&str] = &[
        "/w|Cargo.toml",
        "/w/crates/a|Cargo.toml",
        "/w/crates/b|Cargo.toml",
    ];

    #[test]
    fn nearest_picks_the_closest_marked_folder() {
        let p = policy(RootStrategy::Nearest, RootFallback::None);
        assert_eq!(
            root(&p, "/w/crates/a/src/lib.rs", Some("/w"), ENTRIES).as_deref(),
            Some("/w/crates/a")
        );
        assert_eq!(
            root(&p, "/w/build.rs", Some("/w"), ENTRIES).as_deref(),
            Some("/w")
        );
    }

    #[test]
    fn outermost_picks_the_top_one_but_not_above_the_workspace() {
        let p = policy(RootStrategy::Outermost, RootFallback::None);
        assert_eq!(
            root(&p, "/w/crates/a/src/lib.rs", Some("/w"), ENTRIES).as_deref(),
            Some("/w")
        );
        // A marker above the workspace is out of bounds.
        let above = ["/|Cargo.toml", "/w/crates/a|Cargo.toml"];
        assert_eq!(
            root(&p, "/w/crates/a/src/lib.rs", Some("/w"), &above).as_deref(),
            Some("/w/crates/a")
        );
    }

    #[test]
    fn any_marker_in_the_policy_counts() {
        let p = policy(RootStrategy::Nearest, RootFallback::None);
        let entries = ["/w/x|rust-project.json"];
        assert_eq!(
            root(&p, "/w/x/y/a.rs", Some("/w"), &entries).as_deref(),
            Some("/w/x")
        );
    }

    #[test]
    fn without_a_marker_the_fallback_decides() {
        let none: [&str; 0] = [];
        let file = "/w/scripts/a.rs";
        let p = policy(RootStrategy::Nearest, RootFallback::Workspace);
        assert_eq!(root(&p, file, Some("/w"), &none).as_deref(), Some("/w"));
        assert_eq!(
            root(&p, file, None, &none),
            None,
            "no workspace to fall back to"
        );
        let p = policy(RootStrategy::Nearest, RootFallback::FileDirectory);
        assert_eq!(
            root(&p, file, Some("/w"), &none).as_deref(),
            Some("/w/scripts")
        );
        assert_eq!(root(&p, file, None, &none).as_deref(), Some("/w/scripts"));
        let p = policy(RootStrategy::Nearest, RootFallback::None);
        assert_eq!(root(&p, file, Some("/w"), &none), None);
    }

    #[test]
    fn a_file_outside_the_workspace_is_judged_by_its_own_folder() {
        let p = policy(RootStrategy::Outermost, RootFallback::None);
        let entries = ["/other|Cargo.toml", "/other/deep|Cargo.toml"];
        assert_eq!(
            root(&p, "/other/deep/a.rs", Some("/w"), &entries).as_deref(),
            Some("/other/deep")
        );
        assert_eq!(root(&p, "/other/deep/sub/a.rs", Some("/w"), &entries), None);
        assert_eq!(
            root(&p, "/other/deep/a.rs", None, &entries).as_deref(),
            Some("/other/deep")
        );
    }

    #[test]
    fn a_marker_in_the_workspace_folder_itself_is_found() {
        let p = policy(RootStrategy::Nearest, RootFallback::None);
        assert_eq!(
            root(&p, "/w/a.rs", Some("/w"), &["/w|Cargo.toml"]).as_deref(),
            Some("/w")
        );
    }

    #[test]
    fn a_path_with_no_parent_has_no_root() {
        let p = policy(RootStrategy::Nearest, RootFallback::FileDirectory);
        assert_eq!(resolve(&p, Path::new(""), None, |_, _| false), None);
    }
}
