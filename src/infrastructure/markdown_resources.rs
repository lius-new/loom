//! Bounded file/network image loading. Hosts supply the remote policy explicitly.
use crate::features::markdown::resources::{Address, resolve};
use std::{
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub path: PathBuf,
    pub length: u64,
    pub modified: Option<SystemTime>,
}
pub fn stamp(path: &Path) -> Option<FileStamp> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(FileStamp {
        path: path.to_owned(),
        length: metadata.len(),
        modified: metadata.modified().ok(),
    })
}
#[derive(Clone, Debug)]
pub struct LoadedImage {
    pub width: u32,
    pub height: u32,
    pub encoded: Arc<Vec<u8>>,
    pub local: Option<FileStamp>,
}
pub fn network_path(path: &Path) -> bool {
    let value = path.to_string_lossy();
    value.starts_with("\\\\") || value.starts_with("//")
}
pub fn load(base: Option<&Path>, destination: &str, remote: bool) -> Result<LoadedImage, String> {
    let address = resolve(base, destination).ok_or("Unsupported image address")?;
    let mut bytes = Vec::new();
    let mut local = None;
    match address {
        Address::Local { path, .. } => {
            if !remote && network_path(&path) {
                return Err("Network image disabled; choose Load Remote Images".into());
            }
            let before = stamp(&path);
            let file = std::fs::File::open(&path).map_err(|error| error.to_string())?;
            if file.metadata().map_err(|error| error.to_string())?.len() > 8 * 1024 * 1024 {
                return Err("Image exceeds the 8 MiB encoded limit".into());
            }
            file.take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            let after = stamp(&path);
            if before != after {
                return Err("Image changed while loading; retry after it settles".into());
            }
            local = after;
        }
        Address::Web(url) => {
            if !remote {
                return Err("Remote image disabled; choose Load Remote Images".into());
            }
            let agent = ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(3))
                .timeout_read(Duration::from_secs(5))
                .timeout(Duration::from_secs(8))
                .build();
            let response = agent.get(&url).call().map_err(|error| error.to_string())?;
            response
                .into_reader()
                .take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
        }
        Address::Anchor(_) => return Err("An anchor is not an image".into()),
    }
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Image exceeds the 8 MiB encoded limit".into());
    }
    let format = image::guess_format(&bytes)
        .map_err(|_| "Unsupported image format; PNG and JPEG are supported")?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return Err("Unsupported image format; PNG and JPEG are supported".into());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let (width, height) = image::ImageReader::with_format(Cursor::new(&bytes), format)
        .into_dimensions()
        .map_err(|error| error.to_string())?;
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || (width as u64) * (height as u64) * 4 > 16 * 1024 * 1024
    {
        return Err("Image dimensions exceed the decode budget".into());
    }
    // Validate decoding under limits before handing encoded data to the GUI loader.
    reader.decode().map_err(|error| error.to_string())?;
    Ok(LoadedImage {
        width,
        height,
        encoded: Arc::new(bytes),
        local,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_and_unsupported_inputs_fail_without_loading() {
        assert!(
            load(None, "https://example.com/image.png", false)
                .unwrap_err()
                .contains("disabled")
        );
        assert!(load(None, "data:image/png,a", true).is_err());
    }
}
