use std::fs::DirBuilder;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use super::Context;
use crate::error::Error;
use crate::stats;

pub const DEFAULT_SIZE: u32 = 1600;
const KEEP_BATCHES: usize = 20;
const BASE_TIMEOUT_SECS: u64 = 30;
const PER_PHOTO_TIMEOUT_SECS: u64 = 5;
const SELECTION_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone)]
pub struct PreviewOptions {
    pub ids: Option<Vec<i64>>,
    pub size: u32,
    pub fast: bool,
    pub histogram: bool,
    pub stats: bool,
}

/// Exports take 1-2 s per photo, so the request timeout grows with the batch.
/// With no ids the selection size is unknown, so assume a generous batch.
pub fn default_timeout(ids: Option<&[i64]>) -> Duration {
    Duration::from_secs(match ids {
        Some(ids) => BASE_TIMEOUT_SECS + PER_PHOTO_TIMEOUT_SECS * ids.len() as u64,
        None => SELECTION_TIMEOUT_SECS,
    })
}

pub fn preview_args(options: &PreviewOptions, dir: &Path) -> Value {
    let mut args = Map::new();
    if let Some(ids) = &options.ids {
        args.insert("photos".into(), json!(ids));
    }
    args.insert("size".into(), json!(options.size));
    args.insert(
        "mode".into(),
        json!(if options.fast { "thumbnail" } else { "export" }),
    );
    args.insert("dir".into(), json!(dir));
    Value::Object(args)
}

/// UTC, so batch directory names sort chronologically regardless of time zone changes.
pub fn batch_name(unix_secs: u64, pid: u32) -> String {
    let days = (unix_secs / 86_400) as i64;
    let rest = unix_secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}-{pid}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

fn is_batch_name(name: &str) -> bool {
    let parts: Vec<&str> = name.split('-').collect();
    let all_digits = |part: &str, len: Option<usize>| {
        !part.is_empty()
            && part.bytes().all(|b| b.is_ascii_digit())
            && len.is_none_or(|len| part.len() == len)
    };
    matches!(parts[..], [date, time, pid]
        if all_digits(date, Some(8)) && all_digits(time, Some(6)) && all_digits(pid, None))
}

/// Best effort: a failed cleanup must never fail the command.
pub fn prune(previews: &Path, current: &str) {
    let Ok(entries) = std::fs::read_dir(previews) else {
        return;
    };
    let mut batches: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| is_batch_name(name))
        .collect();
    batches.sort_unstable_by(|a, b| b.cmp(a));
    for name in batches.iter().skip(KEEP_BATCHES).filter(|n| *n != current) {
        let _ = std::fs::remove_dir_all(previews.join(name));
    }
}

fn create_dir(path: &Path) -> Result<(), Error> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|e| Error::io("io_error", format!("Cannot create {}", path.display()), e))
}

fn create_batch_dir(previews: &Path) -> Result<(String, PathBuf), Error> {
    create_dir(previews)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let name = batch_name(now, std::process::id());
    let dir = previews.join(&name);
    create_dir(&dir)?;
    let dir = std::path::absolute(&dir)
        .map_err(|e| Error::io("io_error", format!("Cannot resolve {}", dir.display()), e))?;
    Ok((name, dir))
}

fn add_stats(photo: &mut Value, histogram: bool) {
    let Some(file) = photo.get("file").and_then(Value::as_str) else {
        return;
    };
    let analysis = std::fs::read(file)
        .map_err(|e| Error::io("io_error", format!("Cannot read {file}"), e))
        .and_then(|bytes| stats::analyze(&bytes, histogram));
    match analysis {
        Ok(stats) => photo["stats"] = stats,
        Err(e) => photo["statsError"] = json!(e.message),
    }
}

pub fn first_error(reply: &Value) -> Option<String> {
    let photos = reply.get("photos")?.as_array()?;
    if photos
        .iter()
        .any(|p| p.get("file").is_some_and(Value::is_string))
    {
        return None;
    }
    Some(
        photos
            .iter()
            .find_map(|p| p.get("error").and_then(Value::as_str))
            .unwrap_or("Lightroom returned no previews.")
            .to_string(),
    )
}

pub async fn run(ctx: &Context, options: &PreviewOptions) -> Result<Value, Error> {
    let previews = ctx.paths.dir().join("previews");
    let (batch, dir) = create_batch_dir(&previews)?;
    let mut reply = ctx
        .connect()
        .await?
        .request("preview", preview_args(options, &dir))
        .await?;
    prune(&previews, &batch);

    if options.stats
        && let Some(photos) = reply.get_mut("photos").and_then(Value::as_array_mut)
    {
        for photo in photos {
            add_stats(photo, options.histogram);
        }
    }
    reply["dir"] = json!(dir);
    if let Some(message) = first_error(&reply) {
        return Err(Error::failed("preview_failed", message));
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_scales_with_the_batch() {
        assert_eq!(default_timeout(Some(&[1])), Duration::from_secs(35));
        assert_eq!(default_timeout(Some(&[1, 2, 3])), Duration::from_secs(45));
        assert_eq!(default_timeout(None), Duration::from_secs(120));
    }

    #[test]
    fn batch_names_are_utc_timestamps_with_the_pid() {
        assert_eq!(batch_name(0, 7), "19700101-000000-7");
        assert_eq!(batch_name(1_709_251_199, 42), "20240229-235959-42");
        assert_eq!(batch_name(1_793_825_647, 1), "20261104-205407-1");
    }

    #[test]
    fn only_batch_shaped_names_are_recognised() {
        assert!(is_batch_name("20240229-235959-42"));
        for name in [
            "notes",
            "20240229-235959",
            "2024-02-29-1",
            "20240229-235959-x",
        ] {
            assert!(!is_batch_name(name), "{name}");
        }
    }

    #[test]
    fn prune_keeps_the_newest_batches_and_unrelated_entries() {
        let temp = tempfile::TempDir::new().unwrap();
        for i in 0..25 {
            std::fs::create_dir(temp.path().join(format!("20240101-0000{i:02}-1"))).unwrap();
        }
        std::fs::create_dir(temp.path().join("keep-me")).unwrap();
        std::fs::write(temp.path().join("20230101-000000-1"), "file").unwrap();
        prune(temp.path(), "20240101-000024-1");
        let mut names: Vec<String> = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names.len(), 22);
        assert!(names.contains(&"20240101-000005-1".to_string()));
        assert!(!names.contains(&"20240101-000004-1".to_string()));
        assert!(names.contains(&"keep-me".to_string()));
        assert!(names.contains(&"20230101-000000-1".to_string()));
    }

    #[test]
    fn prune_never_removes_the_current_batch() {
        let temp = tempfile::TempDir::new().unwrap();
        for i in 0..25 {
            std::fs::create_dir(temp.path().join(format!("20240101-0000{i:02}-1"))).unwrap();
        }
        prune(temp.path(), "20240101-000000-1");
        assert!(temp.path().join("20240101-000000-1").exists());
    }

    #[test]
    fn args_omit_photos_for_the_selection() {
        let options = PreviewOptions {
            ids: None,
            size: 800,
            fast: true,
            histogram: false,
            stats: true,
        };
        assert_eq!(
            preview_args(&options, Path::new("/d")),
            json!({ "size": 800, "mode": "thumbnail", "dir": "/d" })
        );
    }

    #[test]
    fn all_failed_previews_report_the_first_error() {
        let failed = json!({ "photos": [
            { "file": null, "error": "first" }, { "file": null, "error": "second" } ] });
        assert_eq!(first_error(&failed).as_deref(), Some("first"));
        let partial = json!({ "photos": [{ "file": "/a.jpg" }, { "file": null, "error": "x" }] });
        assert_eq!(first_error(&partial), None);
    }
}
