use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::Context;
use crate::error::Error;

pub const DEFAULT_QUALITY: u8 = 90;
pub const DEFAULT_NAME: &str = "{{image_name}}";
pub const MAX_PIXELS: u32 = 65535;
pub const MAX_MEGAPIXELS: f64 = 1000.0;
const BASE_TIMEOUT_SECS: u64 = 60;
const PER_PHOTO_TIMEOUT_SECS: u64 = 10;
const SELECTION_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Clone, PartialEq)]
pub enum Size {
    LongEdge(u32),
    ShortEdge(u32),
    Dimensions(u32, u32),
    Megapixels(f64),
}

impl Size {
    fn to_json(&self) -> Value {
        match self {
            Size::LongEdge(px) => json!({ "type": "longEdge", "value": px }),
            Size::ShortEdge(px) => json!({ "type": "shortEdge", "value": px }),
            Size::Dimensions(width, height) => {
                json!({ "type": "dimensions", "width": width, "height": height })
            }
            Size::Megapixels(mp) => json!({ "type": "megapixels", "value": mp }),
        }
    }
}

/// Values are the plugin's wire names.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub ids: Option<Vec<i64>>,
    pub dest: PathBuf,
    pub format: &'static str,
    pub quality: Option<u8>,
    pub size: Option<Size>,
    pub color_space: &'static str,
    pub bit_depth: Option<u8>,
    pub tiff_compression: Option<&'static str>,
    pub sharpen: Option<(&'static str, u8)>,
    pub metadata: &'static str,
    pub remove_location: bool,
    pub name: String,
    pub on_conflict: &'static str,
    pub subfolder: Option<String>,
    pub add_to_catalog: bool,
}

/// Renders take a few seconds per photo, so the request timeout grows with the batch.
/// With no ids the selection size is unknown, so assume a generous batch.
pub fn default_timeout(ids: Option<&[i64]>) -> Duration {
    Duration::from_secs(match ids {
        Some(ids) => BASE_TIMEOUT_SECS + PER_PHOTO_TIMEOUT_SECS * ids.len() as u64,
        None => SELECTION_TIMEOUT_SECS,
    })
}

pub fn export_args(options: &ExportOptions, dest: &Path) -> Value {
    let mut args = Map::new();
    if let Some(ids) = &options.ids {
        args.insert("photos".into(), json!(ids));
    }
    args.insert("dest".into(), json!(dest));
    args.insert("format".into(), json!(options.format));
    if let Some(quality) = options.quality {
        args.insert("quality".into(), json!(quality));
    }
    if let Some(size) = &options.size {
        args.insert("size".into(), size.to_json());
    }
    args.insert("colorSpace".into(), json!(options.color_space));
    if let Some(bit_depth) = options.bit_depth {
        args.insert("bitDepth".into(), json!(bit_depth));
    }
    if let Some(compression) = options.tiff_compression {
        args.insert("tiffCompression".into(), json!(compression));
    }
    if let Some((media, level)) = options.sharpen {
        args.insert("sharpen".into(), json!({ "media": media, "level": level }));
    }
    args.insert("metadata".into(), json!(options.metadata));
    args.insert("removeLocation".into(), json!(options.remove_location));
    args.insert("name".into(), json!(options.name));
    args.insert("onConflict".into(), json!(options.on_conflict));
    if let Some(subfolder) = &options.subfolder {
        args.insert("subfolder".into(), json!(subfolder));
    }
    args.insert("addToCatalog".into(), json!(options.add_to_catalog));
    Value::Object(args)
}

fn resolve_dest(dest: &Path) -> Result<PathBuf, Error> {
    std::fs::create_dir_all(dest)
        .map_err(|e| Error::io("io_error", format!("Cannot create {}", dest.display()), e))?;
    std::path::absolute(dest)
        .map_err(|e| Error::io("io_error", format!("Cannot resolve {}", dest.display()), e))
}

fn first_error(reply: &Value) -> Option<String> {
    let photos = reply.get("photos")?.as_array()?;
    let succeeded = |p: &Value| {
        p.get("file").is_some_and(Value::is_string)
            || p.get("skipped").and_then(Value::as_bool) == Some(true)
    };
    if photos.iter().any(succeeded) {
        return None;
    }
    let first = photos
        .iter()
        .find_map(|p| p.get("error").and_then(Value::as_str))
        .unwrap_or("Lightroom exported no files.");
    Some(match photos.len() {
        0 | 1 => first.to_string(),
        n => format!("No photo was exported ({n} failed). First error: {first}"),
    })
}

pub async fn run(ctx: &Context, options: &ExportOptions) -> Result<Value, Error> {
    let dest = resolve_dest(&options.dest)?;
    let mut reply = ctx
        .connect()
        .await?
        .request("export", export_args(options, &dest))
        .await?;

    if let Some(photos) = reply.get_mut("photos").and_then(Value::as_array_mut) {
        for photo in photos {
            let bytes = photo
                .get("file")
                .and_then(Value::as_str)
                .and_then(|file| std::fs::metadata(file).ok())
                .map(|meta| meta.len());
            if let Some(bytes) = bytes {
                photo["bytes"] = json!(bytes);
            }
        }
    }
    if let Some(message) = first_error(&reply) {
        return Err(Error::failed("export_failed", message));
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> ExportOptions {
        ExportOptions {
            ids: None,
            dest: PathBuf::from("out"),
            format: "JPEG",
            quality: Some(DEFAULT_QUALITY),
            size: None,
            color_space: "sRGB",
            bit_depth: None,
            tiff_compression: None,
            sharpen: None,
            metadata: "all",
            remove_location: false,
            name: DEFAULT_NAME.into(),
            on_conflict: "rename",
            subfolder: None,
            add_to_catalog: false,
        }
    }

    #[test]
    fn timeout_scales_with_the_batch() {
        assert_eq!(default_timeout(Some(&[1])), Duration::from_secs(70));
        assert_eq!(default_timeout(Some(&[1, 2, 3])), Duration::from_secs(90));
        assert_eq!(default_timeout(None), Duration::from_secs(300));
    }

    #[test]
    fn args_omit_unset_options() {
        assert_eq!(
            export_args(&options(), Path::new("/d")),
            json!({
                "dest": "/d", "format": "JPEG", "quality": 90, "colorSpace": "sRGB",
                "metadata": "all", "removeLocation": false, "name": "{{image_name}}",
                "onConflict": "rename", "addToCatalog": false,
            })
        );
    }

    #[test]
    fn sizes_serialise_to_the_plugin_shape() {
        assert_eq!(
            Size::Dimensions(1920, 1080).to_json(),
            json!({ "type": "dimensions", "width": 1920, "height": 1080 })
        );
        assert_eq!(
            Size::Megapixels(2.5).to_json(),
            json!({ "type": "megapixels", "value": 2.5 })
        );
    }

    #[test]
    fn all_failed_exports_report_the_first_error() {
        let failed = json!({ "photos": [
            { "file": null, "error": "first" }, { "file": null, "error": "second" } ] });
        assert_eq!(
            first_error(&failed).as_deref(),
            Some("No photo was exported (2 failed). First error: first")
        );
        let single = json!({ "photos": [{ "file": null, "error": "only" }] });
        assert_eq!(first_error(&single).as_deref(), Some("only"));
        let partial = json!({ "photos": [{ "file": "/a.jpg" }, { "file": null, "error": "x" }] });
        assert_eq!(first_error(&partial), None);
    }
}
