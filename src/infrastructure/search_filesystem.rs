//! Actual ignore traversal, path normalization and bounded file reads.
use crate::features::search::ports::{SearchSourcePort, SourceEntry};
use crate::features::search::{self, MAX_FILE_BYTES, Matcher, Request, Results, SearchSpec};
use ignore::WalkBuilder;
use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[derive(Default)]
pub struct SearchFilesystem;
impl SearchSourcePort for SearchFilesystem {
    fn normalize(&self, path: &Path) -> PathBuf {
        normalized_path(path)
    }
    fn is_directory(&self, path: &Path) -> bool {
        path.is_dir()
    }
    fn read_text(&self, path: &Path) -> Result<String, String> {
        read_text(path)
    }
    fn walk(
        &self,
        root: &Path,
        spec: &SearchSpec,
        matcher: Arc<Matcher>,
    ) -> Box<dyn Iterator<Item = Result<SourceEntry, String>> + '_> {
        Box::new(search_walk(root, spec, matcher).build().map(|entry| {
            entry
                .map(|entry| SourceEntry {
                    path: entry.path().to_path_buf(),
                    is_file: entry.file_type().is_some_and(|kind| kind.is_file()),
                })
                .map_err(|error| error.to_string())
        }))
    }
}
pub fn run(
    request: Request,
    cancel: &AtomicBool,
    publish: impl FnMut(Results),
) -> Result<(), String> {
    search::service::run(&SearchFilesystem, request, cancel, publish)
}

pub fn normalized_path(path: &Path) -> PathBuf {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    #[cfg(windows)]
    {
        let value = path.to_string_lossy();
        if let Some(value) = value.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{value}"));
        }
        if let Some(value) = value.strip_prefix(r"\\?\") {
            return PathBuf::from(value);
        }
    }
    path
}

/// Bound the actual read as well as metadata: a file may grow during searching.
pub fn read_text(path: &Path) -> Result<String, String> {
    let file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Not a regular file".into());
    }
    if metadata.len() > MAX_FILE_BYTES as u64 {
        return Err("File is larger than 4 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("File is larger than 4 MiB".into());
    }
    if bytes.contains(&0) {
        return Err("Binary file".into());
    }
    String::from_utf8(bytes).map_err(|_| "File is not UTF-8".into())
}

fn search_walk(root: &Path, spec: &SearchSpec, matcher: Arc<Matcher>) -> WalkBuilder {
    let mut walk = WalkBuilder::new(root);
    walk.hidden(!spec.include_hidden)
        .git_ignore(!spec.include_ignored)
        .git_global(!spec.include_ignored)
        .git_exclude(!spec.include_ignored)
        .ignore(!spec.include_ignored)
        .parents(!spec.include_ignored)
        .require_git(false)
        .follow_links(false);
    if !spec.include_ignored {
        walk.add_custom_ignore_filename(".rgignore");
    }
    let root = root.to_path_buf();
    walk.filter_entry(move |entry| {
        entry.depth() == 0
            || (entry.file_name() != ".git"
                && !matcher.excludes(entry.path().strip_prefix(&root).unwrap_or(entry.path())))
    });
    walk
}

/// Evaluate watcher paths off the UI thread, using the same ignore rules as
/// searching. Only walk their ancestor branches; never scan file contents.
/// Missing paths conservatively use their nearest remaining parent so removing
/// or renaming a directory cannot leave results for its children behind.
pub fn paths_affect_search(spec: &SearchSpec, roots: &[PathBuf], paths: &[PathBuf]) -> bool {
    let Ok(matcher) = Matcher::new(spec).map(Arc::new) else {
        return false;
    };
    for root in roots {
        let root = normalized_path(root);
        let mut candidates = HashSet::new();
        for path in paths {
            let path = normalized_path(path);
            let Ok(relative) = path.strip_prefix(&root) else {
                continue;
            };
            if relative.components().any(|part| part.as_os_str() == ".git") {
                continue;
            }
            if !spec.include_ignored
                && path.file_name().is_some_and(|name| {
                    name == ".gitignore" || name == ".ignore" || name == ".rgignore"
                })
            {
                return true;
            }
            if matcher.excludes(relative)
                || (!spec.include_hidden
                    && relative
                        .components()
                        .any(|part| part.as_os_str().to_string_lossy().starts_with('.')))
            {
                continue;
            }
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => continue,
                Ok(metadata) if metadata.is_file() && !matcher.includes(relative) => continue,
                _ => {}
            }
            let candidate = path
                .ancestors()
                .find(|parent| parent.exists())
                .unwrap_or(&root);
            candidates.insert(candidate.to_path_buf());
        }
        if candidates.is_empty() {
            continue;
        }
        let ancestors = candidates
            .iter()
            .flat_map(|path| path.ancestors().map(Path::to_path_buf))
            .collect::<HashSet<_>>();
        let mut walk = search_walk(&root, spec, matcher.clone());
        // filter_entry replaces the previous callback, so retain its explicit
        // exclusion checks as well as pruning unrelated ancestor branches.
        let filter = matcher.clone();
        let filter_root = root.clone();
        walk.filter_entry(move |entry| {
            ancestors.contains(entry.path())
                && (entry.depth() == 0
                    || (entry.file_name() != ".git"
                        && !filter.excludes(
                            entry
                                .path()
                                .strip_prefix(&filter_root)
                                .unwrap_or(entry.path()),
                        )))
        });
        for entry in walk.build() {
            match entry {
                Ok(entry) if candidates.contains(entry.path()) => return true,
                Err(_) => return true,
                _ => {}
            }
        }
    }
    false
}

#[cfg(test)]
#[path = "search_filesystem_tests.rs"]
mod tests;
