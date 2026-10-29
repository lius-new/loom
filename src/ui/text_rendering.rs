//! Shared application text policy for window, layered and worker-rendered text.
//! Font raster choices belong here, not to individual document views.

pub(crate) fn defaults() -> lgui::text::TextRenderingOptions {
    // Symmetric grayscale smoothing avoids platform grid-fitting stair steps
    // on transparent targets. Native LCD text keeps the system raster mode.
    lgui::text::TextRenderingOptions::default()
        .with_grayscale_hinting(lgui::text::TextHinting::None)
}

// Retain 50% of the color-based correction. Returning
// partway to the reference curve keeps strokes more substantial on either
// polarity, without changing the font, weight, spacing or theme colors.
const COMPENSATION_STRENGTH: f32 = 0.5;

#[derive(Clone, Copy)]
pub(crate) struct ColorPair {
    pub foreground: u32,
    pub background: u32,
    pub weight: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Coverage {
    pub gamma: f32,
    pub contrast: f32,
}

/// Derive one window profile from its text/background pairs. Positive polarity
/// means darker text on a lighter background. Its default contrast boost needs
/// tapering as well as a higher gamma; applying the reverse-polarity curve would
/// thicken those strokes. Neutral pairs stay close to the native coverage curve.
pub(crate) fn coverage(pairs: impl IntoIterator<Item = ColorPair>) -> Option<Coverage> {
    let (mut polarity, mut total) = (0.0, 0.0);
    for pair in pairs {
        if !pair.weight.is_finite() || pair.weight <= 0.0 {
            continue;
        }
        let foreground = luminance(pair.foreground);
        let background = luminance(pair.background);
        // Avoid large corrections for two almost-black, low-contrast colors.
        let pair_polarity = (background - foreground) / (background + foreground).max(0.02);
        polarity += pair_polarity * pair.weight;
        total += pair.weight;
    }
    if total <= 0.0 || !total.is_finite() || !polarity.is_finite() {
        return None;
    }
    let polarity = (polarity / total).clamp(-1.0, 1.0);
    // Derive a bounded color correction without theme names or a brightness
    // cutoff, then apply the same strength to gamma and contrast.
    let strength = if polarity < 0.0 { 0.8 } else { 0.6 };
    let gamma = (2.2 + strength * polarity).clamp(1.4, 2.8);
    let contrast = (0.5 * (1.0 - polarity.max(0.0))).clamp(0.0, 0.5);
    Some(Coverage {
        gamma: 2.2 + (gamma - 2.2) * COMPENSATION_STRENGTH,
        contrast: 0.5 + (contrast - 0.5) * COMPENSATION_STRENGTH,
    })
}

fn luminance(rgb: u32) -> f32 {
    let linear = |channel: u32| {
        let value = (channel & 255) as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(rgb >> 16) + 0.7152 * linear(rgb >> 8) + 0.0722 * linear(rgb)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(foreground: u32, background: u32) -> ColorPair {
        ColorPair {
            foreground,
            background,
            weight: 1.0,
        }
    }

    #[test]
    fn both_polarities_receive_bounded_compensation() {
        let dark = coverage([pair(0xffffff, 0)]).unwrap();
        let light = coverage([pair(0, 0xffffff)]).unwrap();
        assert!((dark.gamma - 1.8).abs() < 0.000001);
        assert_eq!(dark.contrast, 0.5);
        assert!((light.gamma - 2.5).abs() < 0.000001);
        assert!((light.contrast - 0.25).abs() < 0.000001);
    }

    #[test]
    fn changing_colors_is_continuous_across_the_old_brightness_cutoff() {
        for foreground in [0x18181b, 0x64748b, 0xe4e4e7] {
            let mut previous = coverage([pair(foreground, 0)]).unwrap();
            for channel in 1..=255 {
                let background = channel * 0x010101;
                let current = coverage([pair(foreground, background)]).unwrap();
                assert!((current.gamma - previous.gamma).abs() < 0.06);
                assert!((current.contrast - previous.contrast).abs() < 0.03);
                assert!((1.8..=2.5).contains(&current.gamma));
                assert!((0.25..=0.5).contains(&current.contrast));
                previous = current;
            }
        }
    }

    #[test]
    fn muted_and_colored_samples_moderate_the_main_text_profile() {
        let strong = coverage([pair(0x18181b, 0xfafafa)]).unwrap();
        let combined = coverage([
            pair(0x18181b, 0xfafafa),
            pair(0x64748b, 0xfafafa),
            pair(0x2563eb, 0xfafafa),
        ])
        .unwrap();
        assert!(combined.gamma < strong.gamma);
        assert!(combined.gamma > 2.2);
        assert!(combined.contrast > strong.contrast);
        let neutral = coverage([pair(0x808080, 0x808080)]).unwrap();
        assert_eq!(
            neutral,
            Coverage {
                gamma: 2.2,
                contrast: 0.5
            }
        );
        assert!(coverage([pair(0x010101, 0)]).unwrap().gamma > 2.0);
    }

    #[test]
    fn color_hue_and_relative_weights_are_used_without_theme_names() {
        let green = coverage([pair(0x00ff00, 0x808080)]).unwrap();
        let blue = coverage([pair(0x0000ff, 0x808080)]).unwrap();
        assert!(green.gamma < 2.2 && blue.gamma > 2.2);
        let weighted = coverage([
            ColorPair {
                weight: 5.0,
                ..pair(0xffffff, 0)
            },
            pair(0, 0xffffff),
        ])
        .unwrap();
        assert!(weighted.gamma < 2.2);
    }

    #[test]
    fn empty_or_invalid_samples_fall_back_without_nan_parameters() {
        assert_eq!(coverage([]), None);
        for weight in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                coverage([ColorPair {
                    weight,
                    ..pair(0, 0xffffff)
                }]),
                None
            );
        }
    }
}
