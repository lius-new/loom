//! Project configuration filesystem adapter.
pub use crate::features::settings::project::*;
use crate::features::settings::rules::{self as settings_persistence, Scope};
use std::path::{Path, PathBuf};
use std::{fs, io};
/// Read `root`'s project settings; `Ok(None)` when it has none.
pub fn read(root: &Path) -> Result<Option<ProjectSettings>, String> {
    match fs::read_to_string(path(root)) {
        Ok(text) => {
            let (overrides, errors) = settings_persistence::parse_overrides(&text, Scope::Project)?;
            Ok(Some(ProjectSettings { overrides, errors }))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// Create `root`'s project settings file from the template if needed.
pub fn ensure(root: &Path) -> io::Result<PathBuf> {
    let path = path(root);
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, template())?;
    }
    Ok(path)
}
