use std::path::{Path, PathBuf};

use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{ExtendedColorType, ImageFormat, Rgb, RgbImage};
use serde_json::{Value, json};

use crate::error::Error;
use crate::stats;

const MAX_HEIGHT: u32 = 1600;
const GAP: u32 = 8;
const GAP_GREY: u8 = 128;
const JPEG_QUALITY: u8 = 85;

fn decode(jpeg: &[u8], which: &str) -> Result<RgbImage, Error> {
    let image = image::load_from_memory_with_format(jpeg, ImageFormat::Jpeg)
        .map_err(|e| {
            Error::failed(
                "compare_failed",
                format!("Cannot decode the {which} image: {e}"),
            )
        })?
        .to_rgb8();
    if image.width() == 0 || image.height() == 0 {
        return Err(Error::failed(
            "compare_failed",
            format!("The {which} image has no pixels."),
        ));
    }
    Ok(image)
}

fn scaled_to_height(image: RgbImage, height: u32) -> RgbImage {
    if image.height() == height {
        return image;
    }
    let width = (f64::from(image.width()) * f64::from(height) / f64::from(image.height()))
        .round()
        .max(1.0) as u32;
    imageops::resize(&image, width, height, FilterType::Lanczos3)
}

/// Both inputs are scaled to the smaller of their heights (at most 1600 px);
/// `before` goes on the left, `after` on the right.
pub fn compose(before: &[u8], after: &[u8]) -> Result<Vec<u8>, Error> {
    let (before, after) = (decode(before, "before")?, decode(after, "after")?);
    let height = before.height().min(after.height()).min(MAX_HEIGHT);
    let (before, after) = (
        scaled_to_height(before, height),
        scaled_to_height(after, height),
    );
    let width = before.width() + GAP + after.width();
    let mut canvas = RgbImage::from_pixel(width, height, Rgb([GAP_GREY; 3]));
    imageops::replace(&mut canvas, &before, 0, 0);
    imageops::replace(&mut canvas, &after, i64::from(before.width() + GAP), 0);

    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .encode(canvas.as_raw(), width, height, ExtendedColorType::Rgb8)
        .map_err(|e| {
            Error::failed(
                "compare_failed",
                format!("Cannot encode the comparison: {e}"),
            )
        })?;
    Ok(out)
}

/// `<after-stem>-compare.jpg` next to the after image.
pub fn default_output(after: &Path) -> PathBuf {
    let stem = after
        .file_stem()
        .map_or_else(|| "compare".into(), |s| s.to_string_lossy());
    after.with_file_name(format!("{stem}-compare.jpg"))
}

fn read(path: &Path) -> Result<Vec<u8>, Error> {
    std::fs::read(path)
        .map_err(|e| Error::io("io_error", format!("Cannot read {}", path.display()), e))
}

/// Writes the composite and returns `{file, before: {file, stats}, after: {file, stats}}`.
pub fn compare_files(before: &Path, after: &Path, out: &Path) -> Result<Value, Error> {
    let (before_bytes, after_bytes) = (read(before)?, read(after)?);
    let composite = compose(&before_bytes, &after_bytes)?;
    std::fs::write(out, composite)
        .map_err(|e| Error::io("io_error", format!("Cannot write {}", out.display()), e))?;
    Ok(json!({
        "file": out,
        "before": { "file": before, "stats": stats::analyze(&before_bytes, false)? },
        "after": { "file": after, "stats": stats::analyze(&after_bytes, false)? },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(width: u32, height: u32, pixel: [u8; 3]) -> Vec<u8> {
        let image = RgbImage::from_pixel(width, height, Rgb(pixel));
        let mut bytes = Vec::new();
        JpegEncoder::new_with_quality(&mut bytes, 100)
            .encode(image.as_raw(), width, height, ExtendedColorType::Rgb8)
            .unwrap();
        bytes
    }

    fn decoded(bytes: &[u8]) -> RgbImage {
        decode(bytes, "test").unwrap()
    }

    #[test]
    fn equal_sizes_sit_side_by_side_with_a_gap() {
        let out =
            decoded(&compose(&jpeg(40, 20, [250, 0, 0]), &jpeg(40, 20, [0, 0, 250])).unwrap());
        assert_eq!(out.dimensions(), (40 + GAP + 40, 20));
        assert!(out.get_pixel(10, 10)[0] > 200, "before is left");
        assert!(out.get_pixel(70, 10)[2] > 200, "after is right");
        let gap = out.get_pixel(44, 10);
        assert!(gap.0.iter().all(|c| c.abs_diff(GAP_GREY) < 6), "{gap:?}");
    }

    #[test]
    fn both_images_scale_to_the_smaller_height() {
        let out = decoded(&compose(&jpeg(60, 40, [9; 3]), &jpeg(100, 20, [9; 3])).unwrap());
        assert_eq!(out.dimensions(), (30 + GAP + 100, 20));
    }

    #[test]
    fn tall_images_are_capped_at_1600() {
        let out = decoded(&compose(&jpeg(400, 2000, [9; 3]), &jpeg(400, 2000, [9; 3])).unwrap());
        assert_eq!(out.dimensions(), (320 + GAP + 320, 1600));
    }

    #[test]
    fn undecodable_input_names_the_side() {
        let error = compose(&jpeg(4, 4, [0; 3]), b"nope").unwrap_err();
        assert_eq!(error.code, "compare_failed");
        assert!(error.message.contains("after"), "{}", error.message);
    }

    #[test]
    fn default_output_sits_next_to_the_after_image() {
        assert_eq!(
            default_output(Path::new("/p/lrcli-IMG_1.jpg")),
            Path::new("/p/lrcli-IMG_1-compare.jpg")
        );
    }

    #[test]
    fn compare_files_writes_the_composite_and_reports_stats() {
        let temp = tempfile::TempDir::new().unwrap();
        let (before, after) = (temp.path().join("b.jpg"), temp.path().join("a.jpg"));
        std::fs::write(&before, jpeg(16, 8, [0; 3])).unwrap();
        std::fs::write(&after, jpeg(16, 8, [255; 3])).unwrap();
        let out = default_output(&after);
        let result = compare_files(&before, &after, &out).unwrap();
        assert_eq!(result["file"], json!(out));
        assert_eq!(result["before"]["stats"]["meanLuminance"], 0.0);
        assert_eq!(result["after"]["stats"]["meanLuminance"], 1.0);
        assert_eq!(decoded(&std::fs::read(&out).unwrap()).width(), 40);
    }
}
