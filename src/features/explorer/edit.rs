//! Validation of names typed into the inline create and rename rows.
use std::path::{Path, PathBuf};

use super::ports::DirectoryPort;

/// Validate a possibly nested relative name below `parent`. A trailing slash
/// is accepted (it requests a folder) and stripped from the result.
pub fn validate_create_path(
    port: &dyn DirectoryPort,
    parent: &Path,
    raw_name: &str,
) -> Result<PathBuf, String> {
    let name = raw_name.trim_matches('\t').trim_end_matches(['/', '\\']);
    if name.is_empty() || name.chars().all(char::is_whitespace) {
        return Err("A file or folder name must be provided.".to_owned());
    }
    if name.starts_with(['/', '\\']) || Path::new(name).is_absolute() {
        return Err("A file or folder name cannot start with a slash or drive prefix.".to_owned());
    }

    let mut relative = PathBuf::new();
    for segment in name
        .split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
    {
        if matches!(segment, "." | "..") {
            return Err("Relative path segments '.' and '..' are not allowed.".to_owned());
        }
        if segment.trim() != segment {
            return Err("Leading or trailing whitespace is not allowed in a name.".to_owned());
        }
        if invalid_file_name_segment(segment) {
            return Err(format!("The name '{segment}' is not valid."));
        }
        relative.push(segment);
    }
    if relative.as_os_str().is_empty() {
        return Err("A file or folder name must be provided.".to_owned());
    }
    let target = parent.join(&relative);
    if port.exists(&target) {
        return Err(format!("{} already exists.", target.display()));
    }
    Ok(relative)
}

/// Validate a single replacement name for `path`; returns the renamed path.
pub fn validate_rename_path(
    port: &dyn DirectoryPort,
    path: &Path,
    raw_name: &str,
) -> Result<PathBuf, String> {
    let name = raw_name.trim_matches('\t');
    if name.is_empty() || name.chars().all(char::is_whitespace) {
        return Err("A name must be provided.".to_owned());
    }
    if name.contains(['/', '\\']) || matches!(name, "." | "..") || invalid_file_name_segment(name) {
        return Err(format!("The name '{name}' is not valid."));
    }
    let parent = path
        .parent()
        .ok_or_else(|| "This item cannot be renamed.".to_owned())?;
    let target = parent.join(name);
    if target == path {
        return Ok(target);
    }
    if port.exists(&target) {
        return Err(format!("{} already exists.", target.display()));
    }
    Ok(target)
}

fn invalid_file_name_segment(segment: &str) -> bool {
    if segment.len() > 255 || segment.chars().any(|character| character == '\0') {
        return true;
    }
    #[cfg(target_os = "windows")]
    {
        if segment
            .chars()
            .any(|character| character.is_control() || r#"<>:"|?*"#.contains(character))
            || segment.ends_with(['.', ' '])
        {
            return true;
        }
        let device_name = segment
            .split('.')
            .next()
            .unwrap_or(segment)
            .to_ascii_uppercase();
        if matches!(
            device_name.as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        ) {
            return true;
        }
    }
    false
}
