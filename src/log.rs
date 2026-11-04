use std::fmt::Display;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: u64 = 1 << 20;

pub struct Log {
    file: Mutex<Option<File>>,
}

impl Log {
    /// A log that can't be opened silently drops lines; the broker must keep
    /// working when its log directory is odd.
    pub fn open(path: &Path) -> Self {
        let truncate = std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_BYTES);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .truncate(false)
            .open(path)
            .ok()
            .inspect(|f| {
                if truncate {
                    let _ = f.set_len(0);
                }
            });
        Self {
            file: Mutex::new(file),
        }
    }

    pub fn line(&self, message: impl Display) {
        if let Some(file) = self.file.lock().unwrap().as_mut() {
            let _ = writeln!(file, "{} {message}", format_timestamp(unix_now()));
        }
    }
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub fn format_timestamp(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc_timestamps() {
        assert_eq!(format_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_timestamp(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(format_timestamp(1_709_164_800), "2024-02-29T00:00:00Z");
    }
}
