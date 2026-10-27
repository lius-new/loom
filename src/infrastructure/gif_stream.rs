//! Incremental, composited GIF decoding. No whole-animation allocation or PNG compression.
use crate::domain::media::Frame;
use image::{AnimationDecoder, ImageDecoder};
use std::io::Cursor;
use std::sync::Arc;

pub struct Stream {
    source: Arc<[u8]>,
    frames: image::Frames<'static>,
    sequence: u64,
}
fn decoder(source: Arc<[u8]>) -> Result<image::Frames<'static>, String> {
    let mut decoder =
        image::codecs::gif::GifDecoder::new(Cursor::new(source)).map_err(|e| e.to_string())?;
    decoder
        .set_limits(crate::infrastructure::image_filesystem::limits())
        .map_err(|e| e.to_string())?;
    Ok(decoder.into_frames())
}
impl Stream {
    pub fn new(source: Arc<[u8]>) -> Result<Self, String> {
        Ok(Self {
            frames: decoder(source.clone())?,
            source,
            sequence: 0,
        })
    }
    pub fn next_frame(&mut self) -> Result<Frame, String> {
        let frame = match self.frames.next() {
            Some(frame) => frame,
            None => {
                self.frames = decoder(self.source.clone())?;
                self.frames.next().ok_or("GIF has no frames")?
            }
        }
        .map_err(|e| e.to_string())?;
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = u64::from(numerator) / u64::from(denominator.max(1));
        let duration_ms = if delay == 0 { 100 } else { delay.max(10) };
        let rgba = frame.into_buffer();
        // Top-down BMP V4 with RGBA masks lets the renderer consume the original
        // rows in one bulk copy, without compression or per-pixel BGRA conversion.
        // lgui's image port accepts encoded containers rather than raw pixels.
        let mut header = [0u8; 122];
        header[..2].copy_from_slice(b"BM");
        header[2..6].copy_from_slice(&((rgba.len() + 122) as u32).to_le_bytes());
        header[10..14].copy_from_slice(&122u32.to_le_bytes());
        header[14..18].copy_from_slice(&108u32.to_le_bytes());
        header[18..22].copy_from_slice(&rgba.width().to_le_bytes());
        header[22..26].copy_from_slice(&(-(rgba.height() as i32)).to_le_bytes());
        header[26..28].copy_from_slice(&1u16.to_le_bytes());
        header[28..30].copy_from_slice(&32u16.to_le_bytes());
        header[30..34].copy_from_slice(&3u32.to_le_bytes()); // BI_BITFIELDS
        header[34..38].copy_from_slice(&(rgba.len() as u32).to_le_bytes());
        for (offset, mask) in [
            (54, 0xffu32),
            (58, 0xff00),
            (62, 0xff0000),
            (66, 0xff000000),
            (70, 0x73524742),
        ] {
            header[offset..offset + 4].copy_from_slice(&mask.to_le_bytes());
        }
        let mut bytes = Vec::with_capacity(rgba.len() + 122);
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(rgba.as_raw());
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        Ok(Frame {
            bytes: Arc::new(bytes),
            duration_ms,
            sequence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gif_stream_preserves_transparency_disposal_delays_and_loop_pixels() {
        let red = image::RgbaImage::from_pixel(3, 2, image::Rgba([255, 0, 0, 255]));
        let mut transparent = image::RgbaImage::new(3, 2);
        transparent.put_pixel(1, 0, image::Rgba([0, 0, 255, 255]));
        let expected = [red, transparent];
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for (pixels, delay) in expected.iter().zip([40, 100]) {
                encoder
                    .encode_frame(image::Frame::from_parts(
                        pixels.clone(),
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(delay, 1),
                    ))
                    .unwrap();
            }
        }
        let mut stream = Stream::new(Arc::from(bytes)).unwrap();
        for sequence in 0..6 {
            let frame = stream.next_frame().unwrap();
            assert_eq!(frame.sequence, sequence);
            assert_eq!(frame.duration_ms, [40, 100][sequence as usize % 2]);
            let pixels = image::load_from_memory(&frame.bytes).unwrap().into_rgba8();
            assert_eq!(pixels, expected[sequence as usize % 2]);
            let image =
                skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&frame.bytes)).unwrap();
            let mut surface = skia_safe::surfaces::raster_n32_premul((3, 2)).unwrap();
            surface.canvas().clear(skia_safe::Color::TRANSPARENT);
            surface.canvas().draw_image(&image, (0.0, 0.0), None);
            let info = skia_safe::ImageInfo::new(
                (3, 2),
                skia_safe::ColorType::RGBA8888,
                skia_safe::AlphaType::Unpremul,
                None,
            );
            let mut rendered = [0u8; 24];
            assert!(
                surface
                    .canvas()
                    .read_pixels(&info, &mut rendered, 12, (0, 0))
            );
            assert_eq!(
                rendered.as_slice(),
                expected[sequence as usize % 2].as_raw()
            );
        }
    }
}
