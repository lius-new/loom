//! Disk observations supplied to workspace rules; no reactive state or UI handles.
use crate::domain::document::{DiskRead, DiskState};
use std::path::Path;

pub(crate) fn capture(path: &Path, contents: &str) -> DiskState {
    let metadata = std::fs::metadata(path).ok();
    DiskState {
        modified_time: metadata.as_ref().and_then(|value| value.modified().ok()),
        size: metadata
            .as_ref()
            .map_or(contents.len() as u64, std::fs::Metadata::len),
        content_hash: crate::domain::document_snapshot::content_hash(contents),
    }
}
pub(crate) fn read(path: &Path) -> DiskRead {
    match read_contents(path) {
        Ok(contents) => {
            let state = capture(path, &contents);
            DiskRead::Found { contents, state }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => DiskRead::Missing,
        Err(error) => DiskRead::Failed(error.to_string()),
    }
}
pub(crate) fn read_contents(path: &Path) -> std::io::Result<String> {
    std::fs::read_to_string(path)
}
