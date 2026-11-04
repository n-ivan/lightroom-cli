use image::ImageFormat;
use serde_json::{Value, json};

use crate::error::Error;

const BINS: usize = 256;
const WEIGHTS: [f64; 3] = [0.2126, 0.7152, 0.0722];

type Histogram = [u64; BINS];

fn percent(count: u64, pixels: u64) -> f64 {
    round(count as f64 * 100.0 / pixels as f64, 2)
}

fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

fn clipping(histograms: &[Histogram; 4], bin: usize, pixels: u64) -> Value {
    let [luma, r, g, b] = histograms;
    json!({
        "r": percent(r[bin], pixels),
        "g": percent(g[bin], pixels),
        "b": percent(b[bin], pixels),
        "luma": percent(luma[bin], pixels),
    })
}

/// Decodes a JPEG and measures it. Luminance uses Rec. 709 weights on the
/// stored sRGB values (no linearisation), binned to 0..=255.
pub fn analyze(jpeg: &[u8], with_histograms: bool) -> Result<Value, Error> {
    let image = image::load_from_memory_with_format(jpeg, ImageFormat::Jpeg)
        .map_err(|e| Error::failed("stats_failed", format!("Cannot decode the preview: {e}")))?
        .to_rgb8();
    let (width, height) = image.dimensions();
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 {
        return Err(Error::failed("stats_failed", "The preview has no pixels."));
    }

    let mut histograms = [[0u64; BINS]; 4];
    let mut luminance_sum = 0.0;
    for pixel in image.pixels() {
        let [r, g, b] = pixel.0;
        let luma =
            WEIGHTS[0] * f64::from(r) + WEIGHTS[1] * f64::from(g) + WEIGHTS[2] * f64::from(b);
        luminance_sum += luma;
        histograms[0][luma.round() as usize] += 1;
        histograms[1][usize::from(r)] += 1;
        histograms[2][usize::from(g)] += 1;
        histograms[3][usize::from(b)] += 1;
    }

    let mut stats = json!({
        "width": width,
        "height": height,
        "meanLuminance": round(luminance_sum / (pixels as f64 * 255.0), 4),
        "clipping": {
            "highlights": clipping(&histograms, BINS - 1, pixels),
            "shadows": clipping(&histograms, 0, pixels),
        },
    });
    if with_histograms {
        let [luma, r, g, b] = histograms;
        stats["histograms"] = json!({
            "luma": luma.as_slice(),
            "r": r.as_slice(),
            "g": g.as_slice(),
            "b": b.as_slice(),
        });
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use image::codecs::jpeg::JpegEncoder;
    use image::{ExtendedColorType, RgbImage};

    use super::*;

    fn jpeg(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let image = RgbImage::from_fn(width, height, |x, y| image::Rgb(pixel(x, y)));
        let mut bytes = Vec::new();
        JpegEncoder::new_with_quality(&mut bytes, 100)
            .encode(image.as_raw(), width, height, ExtendedColorType::Rgb8)
            .unwrap();
        bytes
    }

    #[test]
    fn all_black_is_fully_clipped_in_the_shadows() {
        let stats = analyze(&jpeg(16, 8, |_, _| [0, 0, 0]), false).unwrap();
        assert_eq!(
            (stats["width"].as_u64(), stats["height"].as_u64()),
            (Some(16), Some(8))
        );
        assert_eq!(stats["meanLuminance"], 0.0);
        assert_eq!(stats["clipping"]["shadows"]["luma"], 100.0);
        assert_eq!(stats["clipping"]["shadows"]["r"], 100.0);
        assert_eq!(stats["clipping"]["highlights"]["luma"], 0.0);
        assert!(stats.get("histograms").is_none());
    }

    #[test]
    fn all_white_is_fully_clipped_in_the_highlights() {
        let stats = analyze(&jpeg(16, 8, |_, _| [255, 255, 255]), false).unwrap();
        assert_eq!(stats["meanLuminance"], 1.0);
        for channel in ["r", "g", "b", "luma"] {
            assert_eq!(stats["clipping"]["highlights"][channel], 100.0, "{channel}");
            assert_eq!(stats["clipping"]["shadows"][channel], 0.0, "{channel}");
        }
    }

    #[test]
    fn gradient_has_a_mid_mean_and_little_clipping() {
        let stats = analyze(&jpeg(256, 4, |x, _| [x as u8; 3]), true).unwrap();
        let mean = stats["meanLuminance"].as_f64().unwrap();
        assert!((mean - 0.5).abs() < 0.02, "{mean}");
        let low = stats["clipping"]["shadows"]["luma"].as_f64().unwrap();
        let high = stats["clipping"]["highlights"]["luma"].as_f64().unwrap();
        assert!(low <= 2.0 && high <= 2.0, "{low} {high}");
        let histograms = &stats["histograms"];
        for name in ["luma", "r", "g", "b"] {
            let bins = histograms[name].as_array().unwrap();
            assert_eq!(bins.len(), 256, "{name}");
            let total: u64 = bins.iter().map(|n| n.as_u64().unwrap()).sum();
            assert_eq!(total, 256 * 4, "{name}");
        }
    }

    #[test]
    fn undecodable_input_is_an_error() {
        let error = analyze(b"not a jpeg", false).unwrap_err();
        assert_eq!(error.code, "stats_failed");
    }
}
