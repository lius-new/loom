//! Explorer rules against an in-memory directory port; no disk access.
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::DirEntry;
use super::edit::{validate_create_path, validate_rename_path};
use super::ports::DirectoryPort;
use super::roots::{self, MAX_RECENT_FOLDERS};
use super::tree::{self, Listings, TreeCache};

/// Directories mapped to their child names; a trailing `/` marks a folder.
#[derive(Default)]
struct MemoryDirectories {
    directories: BTreeMap<PathBuf, Vec<&'static str>>,
    files: HashSet<PathBuf>,
    aliases: BTreeMap<PathBuf, PathBuf>,
}

impl MemoryDirectories {
    fn dir(mut self, path: &str, children: &[&'static str]) -> Self {
        self.directories
            .insert(PathBuf::from(path), children.to_vec());
        self
    }

    fn file(mut self, path: &str) -> Self {
        self.files.insert(PathBuf::from(path));
        self
    }
}

impl DirectoryPort for MemoryDirectories {
    fn read_directory(&self, directory: &Path) -> Arc<Vec<DirEntry>> {
        let mut entries = self
            .directories
            .get(directory)
            .into_iter()
            .flatten()
            .map(|child| {
                let name = child.trim_end_matches('/');
                DirEntry {
                    name: name.to_owned(),
                    path: directory.join(name),
                    is_dir: child.ends_with('/'),
                }
            })
            .collect::<Vec<_>>();
        tree::sort_entries(&mut entries);
        Arc::new(entries)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.directories.contains_key(path)
    }

    fn exists(&self, path: &Path) -> bool {
        self.is_dir(path) || self.files.contains(path)
    }

    fn canonicalize(&self, path: &Path) -> Option<PathBuf> {
        self.aliases
            .get(path)
            .cloned()
            .or_else(|| self.exists(path).then(|| path.to_path_buf()))
    }
}

fn cache(port: &MemoryDirectories, loaded: &[&str], expanded: &[&str]) -> TreeCache {
    let listings = loaded
        .iter()
        .map(|path| (path.to_string(), port.read_directory(Path::new(path))))
        .collect::<Listings>();
    let expanded = expanded.iter().map(|path| path.to_string()).collect();
    TreeCache::from_parts(listings, expanded)
}

fn names(listings: &Listings, key: &str) -> Vec<String> {
    listings[key]
        .iter()
        .map(|entry| entry.name.clone())
        .collect()
}

#[test]
fn entries_sort_folders_before_files_then_by_name() {
    let port = MemoryDirectories::default().dir("root", &["b.txt", "z-dir/", "a.txt", "c-dir/"]);
    let entries = port.read_directory(Path::new("root"));
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["c-dir", "z-dir", "a.txt", "b.txt"]
    );
}

#[test]
fn cached_directory_refresh_is_idempotent() {
    let mut port = MemoryDirectories::default().dir("root", &[]);
    let mut tree = cache(&port, &["root"], &[]);

    assert!(!tree.refresh_directory(&port, Path::new("root")));
    port = port.dir("root", &["new.txt"]);
    assert!(tree.refresh_directory(&port, Path::new("root")));
    assert!(!tree.refresh_directory(&port, Path::new("root")));
    assert_eq!(names(&tree.into_parts().0, "root"), ["new.txt"]);
}

#[test]
fn refresh_never_loads_lazy_directories() {
    let port = MemoryDirectories::default()
        .dir("root", &["child/"])
        .dir("root/child", &["inner.txt"]);
    let mut tree = cache(&port, &["root"], &["root"]);

    assert!(!tree.refresh_directory(&port, Path::new("root/child")));
    assert!(!tree.refresh_affected(
        &port,
        &[PathBuf::from("root")],
        [PathBuf::from("root/child/inner.txt")]
    ));
    assert!(!tree.into_parts().0.contains_key("root/child"));
}

#[test]
fn refresh_prunes_removed_descendant_cache() {
    let mut port = MemoryDirectories::default()
        .dir("root", &["child/"])
        .dir("root/child", &[]);
    let mut tree = cache(&port, &["root", "root/child"], &["root", "root/child"]);

    port = MemoryDirectories::default().dir("root", &[]);
    assert!(tree.refresh_all(&port, &[PathBuf::from("root")]));
    let (listings, expanded) = tree.into_parts();
    assert!(!listings.contains_key("root/child"));
    assert!(!expanded.contains("root/child"));
    assert!(listings["root"].is_empty());
}

#[test]
fn refresh_keeps_missing_roots_but_prunes_directories_outside_roots() {
    let port = MemoryDirectories::default().dir("other", &[]);
    let mut tree = cache(&port, &["gone-root", "other"], &["gone-root", "other"]);

    assert!(tree.refresh_all(&port, &[PathBuf::from("gone-root")]));
    let (listings, expanded) = tree.into_parts();
    assert!(listings.contains_key("gone-root"));
    assert!(expanded.contains("gone-root"));
    assert!(!listings.contains_key("other"));
}

#[test]
fn refresh_affected_reloads_parents_and_loaded_paths() {
    let mut port = MemoryDirectories::default()
        .dir("root", &["src/"])
        .dir("root/src", &[]);
    let mut tree = cache(&port, &["root", "root/src"], &["root"]);

    port = MemoryDirectories::default()
        .dir("root", &["src/", "added.txt"])
        .dir("root/src", &["main.rs"]);
    assert!(tree.refresh_affected(
        &port,
        &[PathBuf::from("root")],
        [PathBuf::from("root/added.txt"), PathBuf::from("root/src")]
    ));
    let (listings, _) = tree.into_parts();
    assert_eq!(names(&listings, "root"), ["src", "added.txt"]);
    assert_eq!(names(&listings, "root/src"), ["main.rs"]);
}

#[test]
fn unchanged_refresh_keeps_listing_identity() {
    let port = MemoryDirectories::default().dir("root", &["a.txt"]);
    let mut tree = cache(&port, &["root"], &[]);
    let before = tree.clone().into_parts().0["root"].clone();

    assert!(!tree.refresh_all(&port, &[PathBuf::from("root")]));
    assert!(Arc::ptr_eq(&before, &tree.into_parts().0["root"]));
}

#[test]
fn create_path_validation_accepts_nested_relative_names() {
    let port = MemoryDirectories::default();
    let parent = PathBuf::from("workspace");

    assert_eq!(
        validate_create_path(&port, &parent, "src/components/button.rs").unwrap(),
        PathBuf::from("src").join("components").join("button.rs")
    );
    assert_eq!(
        validate_create_path(&port, &parent, "assets\\icons\\").unwrap(),
        PathBuf::from("assets").join("icons")
    );
    assert_eq!(
        validate_create_path(&port, &parent, "笔记/日志 🙂.md").unwrap(),
        PathBuf::from("笔记").join("日志 🙂.md")
    );
}

#[test]
fn create_path_validation_rejects_empty_absolute_and_parent_names() {
    let port = MemoryDirectories::default();
    let parent = PathBuf::from("workspace");

    assert!(validate_create_path(&port, &parent, "   ").is_err());
    assert!(validate_create_path(&port, &parent, "/absolute").is_err());
    assert!(validate_create_path(&port, &parent, "../outside").is_err());
    assert!(validate_create_path(&port, &parent, "src/../outside").is_err());
    assert!(validate_create_path(&port, &parent, "src/ padded").is_err());
}

#[test]
fn create_and_rename_reject_existing_targets() {
    let port = MemoryDirectories::default()
        .dir("workspace", &["taken.rs", "old.rs"])
        .file("workspace/taken.rs")
        .file("workspace/old.rs");
    let parent = PathBuf::from("workspace");

    assert!(validate_create_path(&port, &parent, "taken.rs").is_err());
    assert!(validate_rename_path(&port, &parent.join("old.rs"), "taken.rs").is_err());
    assert_eq!(
        validate_rename_path(&port, &parent.join("old.rs"), "old.rs").unwrap(),
        parent.join("old.rs")
    );
}

#[test]
fn rename_validation_accepts_one_folder_name_and_rejects_paths() {
    let port = MemoryDirectories::default();
    let path = PathBuf::from("__loom_missing__/old-name");

    assert_eq!(
        validate_rename_path(&port, &path, "new-name").unwrap(),
        PathBuf::from("__loom_missing__/new-name")
    );
    assert!(validate_rename_path(&port, &path, "nested/name").is_err());
    assert!(validate_rename_path(&port, &path, "..").is_err());
    assert!(validate_rename_path(&port, &path, "   ").is_err());
}

#[cfg(target_os = "windows")]
#[test]
fn create_path_validation_rejects_windows_device_and_invalid_names() {
    let port = MemoryDirectories::default();
    let parent = PathBuf::from("workspace");

    assert!(validate_create_path(&port, &parent, "CON.txt").is_err());
    assert!(validate_create_path(&port, &parent, "bad?.txt").is_err());
    assert!(validate_create_path(&port, &parent, "trailing.").is_err());
}

#[test]
fn recent_folders_are_mru_ordered_and_bounded() {
    let mut recents = Vec::new();

    for index in 0..MAX_RECENT_FOLDERS + 2 {
        roots::remember(&mut recents, PathBuf::from(format!("workspace-{index}")));
    }
    roots::remember(&mut recents, PathBuf::from("workspace-4"));

    assert_eq!(recents.len(), MAX_RECENT_FOLDERS);
    assert_eq!(recents[0], PathBuf::from("workspace-4"));
    assert_eq!(recents[1], PathBuf::from("workspace-9"));
}

#[test]
fn hydrate_drops_missing_folders_and_lists_roots_among_recents() {
    let mut port = MemoryDirectories::default()
        .dir("a", &[])
        .dir("b", &[])
        .dir("canonical", &[]);
    port.aliases
        .insert(PathBuf::from("alias"), PathBuf::from("canonical"));
    let mut workspace_roots = vec![
        PathBuf::from("a"),
        PathBuf::from("gone"),
        PathBuf::from("alias"),
        PathBuf::from("canonical"),
    ];
    let mut recents = vec![PathBuf::from("b"), PathBuf::from("missing")];

    roots::hydrate(&port, &mut workspace_roots, &mut recents);

    assert_eq!(
        workspace_roots,
        [PathBuf::from("a"), PathBuf::from("canonical")]
    );
    assert_eq!(
        recents,
        [
            PathBuf::from("b"),
            PathBuf::from("a"),
            PathBuf::from("canonical")
        ]
    );
}

#[test]
fn restored_roots_do_not_reorder_existing_recent_history() {
    let port = MemoryDirectories::default()
        .dir("first-workspace", &[])
        .dir("second-workspace", &[])
        .dir("older-workspace", &[]);
    let first = PathBuf::from("first-workspace");
    let second = PathBuf::from("second-workspace");
    let older = PathBuf::from("older-workspace");
    let mut workspace_roots = vec![first.clone(), second.clone()];
    let mut recents = vec![second.clone(), older.clone(), first.clone()];

    roots::hydrate(&port, &mut workspace_roots, &mut recents);

    assert_eq!(recents, vec![second, older, first]);
}

#[test]
fn root_for_path_prefers_the_deepest_containing_root() {
    let workspace_roots = [
        PathBuf::from("workspace"),
        PathBuf::from("workspace/nested"),
        PathBuf::from("other"),
    ];

    assert_eq!(
        roots::root_for_path(&workspace_roots, Path::new("workspace/nested/src/lib.rs")),
        Some(&PathBuf::from("workspace/nested"))
    );
    assert_eq!(
        roots::root_for_path(&workspace_roots, Path::new("workspace/src")),
        Some(&PathBuf::from("workspace"))
    );
    assert_eq!(
        roots::root_for_path(&workspace_roots, Path::new("elsewhere")),
        None
    );
}
