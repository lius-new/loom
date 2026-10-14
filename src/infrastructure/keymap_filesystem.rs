//! Keymap file I/O; parsing, templates and defaults are pure feature data.
pub use crate::features::keymap::keymap_file::*;
use crate::infrastructure::session_filesystem::config_path;
use std::path::PathBuf;
use std::{fs, io};
pub fn user_path() -> Option<PathBuf> {
    config_path(FILE_NAME)
}

pub fn load_user() -> UserKeymap {
    let Some(path) = user_path() else {
        return UserKeymap::Missing;
    };
    match fs::read_to_string(&path) {
        Ok(content) => match parse(&content, BindingSource::User) {
            Ok(outcome) => UserKeymap::Loaded(outcome),
            Err(error) => UserKeymap::Invalid(error),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => UserKeymap::Missing,
        Err(error) => UserKeymap::Invalid(error.to_string()),
    }
}

/// Create the user keymap from the template if it does not exist yet.
pub fn ensure_user_file() -> io::Result<PathBuf> {
    let path = user_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "could not determine the user configuration directory",
        )
    })?;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, USER_TEMPLATE)?;
    }
    Ok(path)
}

use crate::features::keymap::bindings::BindingSource;
