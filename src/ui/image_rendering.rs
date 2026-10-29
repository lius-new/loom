//! Image display policy shared by previews, rich documents and cached text.
use lgui::core::{ImageRequest, ImageSampling, UiImageSource};

pub(crate) fn image(source: UiImageSource) -> ImageRequest {
    ImageRequest::new(source).sampling(ImageSampling::Linear)
}

/// Glyph tiles are already rendered at the final pixel size and snapped by
/// their host. Filtering them a second time would soften the glyphs.
pub(crate) fn pixel_exact(source: UiImageSource) -> ImageRequest {
    ImageRequest::new(source).sampling(ImageSampling::Nearest)
}
