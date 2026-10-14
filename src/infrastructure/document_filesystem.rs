//! Bounded UTF-8 reads and document writes.
use std::io::Read;
use std::path::Path;
use std::{fs, io};
const MAX_EDITABLE_FILE_MIB: u64 = 64;
pub(crate) const MAX_EDITABLE_FILE_BYTES: u64 = MAX_EDITABLE_FILE_MIB * 1024 * 1024;

pub(crate) fn read_text_file(path: &Path) -> Result<String, String> {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let file = fs::File::open(path).map_err(|error| format!("Could not open {name}: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Could not open {name}: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("Could not open {name}: not a regular file"));
    }
    let too_large =
        || format!("Could not open {name}: file is larger than {MAX_EDITABLE_FILE_MIB} MiB");
    if metadata.len() > MAX_EDITABLE_FILE_BYTES {
        return Err(too_large());
    }
    // Bound the read as well: another process can grow the file after metadata.
    let mut bytes = Vec::new();
    file.take(MAX_EDITABLE_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not open {name}: {error}"))?;
    if bytes.len() as u64 > MAX_EDITABLE_FILE_BYTES {
        return Err(too_large());
    }
    String::from_utf8(bytes).map_err(|error| format!("Could not open {name} as UTF-8: {error}"))
}
pub fn write(path: &Path, contents: &[u8]) -> io::Result<()> {
    fs::write(path, contents)
}
