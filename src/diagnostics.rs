use serde_json::{Value, json};

use crate::clock;
use crate::paths::Paths;

const MAX_LINES: usize = 5;
const MAX_AGE_SECS: i64 = 10 * 60;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Diagnostics {
    pub plugin: Vec<String>,
    pub broker: Vec<String>,
}

impl Diagnostics {
    pub fn is_empty(&self) -> bool {
        self.plugin.is_empty() && self.broker.is_empty()
    }

    pub fn to_json(&self) -> Value {
        json!({ "plugin": self.plugin, "broker": self.broker })
    }
}

/// Recent lines of both logs, to show why Lightroom couldn't be reached.
pub fn recent(paths: &Paths) -> Diagnostics {
    let now = clock::now_secs() as i64;
    let read = |path| std::fs::read_to_string(path).unwrap_or_default();
    Diagnostics {
        plugin: recent_lines(&read(paths.plugin_log()), now),
        broker: recent_lines(&read(paths.log()), now),
    }
}

fn recent_lines(text: &str, now: i64) -> Vec<String> {
    let mut lines: Vec<String> = text
        .lines()
        .filter(|line| timestamp(line).is_some_and(|t| now - t <= MAX_AGE_SECS))
        .map(str::to_string)
        .collect();
    lines.drain(..lines.len().saturating_sub(MAX_LINES));
    lines
}

fn digits(text: &str, range: std::ops::Range<usize>) -> Option<i64> {
    let part = text.get(range)?;
    part.bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| part.parse().ok())?
}

/// Both logs start with `YYYY-MM-DDTHH:MM:SSZ`.
fn timestamp(line: &str) -> Option<i64> {
    let bytes = line.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return None;
    }
    let days = clock::days_from_civil(
        digits(line, 0..4)?,
        digits(line, 5..7)?,
        digits(line, 8..10)?,
    );
    Some(
        days * 86_400
            + digits(line, 11..13)? * 3600
            + digits(line, 14..16)? * 60
            + digits(line, 17..19)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000; // 2023-11-14T22:13:20Z

    #[test]
    fn lines_are_filtered_by_age_and_capped() {
        let text = "2023-11-14T22:00:00Z too old\n\
                    garbage line\n\
                    2023-11-14T22:04:00Z a\n\
                    2023-11-14T22:05:00Z b\n\
                    2023-11-14T22:06:00Z c\n\
                    2023-11-14T22:07:00Z d\n\
                    2023-11-14T22:08:00Z e\n\
                    2023-11-14T22:09:00Z f\n";
        let lines = recent_lines(text, NOW);
        assert_eq!(
            lines,
            [
                "2023-11-14T22:05:00Z b",
                "2023-11-14T22:06:00Z c",
                "2023-11-14T22:07:00Z d",
                "2023-11-14T22:08:00Z e",
                "2023-11-14T22:09:00Z f",
            ]
        );
    }

    #[test]
    fn future_lines_and_empty_logs_are_handled() {
        assert!(recent_lines("", NOW).is_empty());
        assert_eq!(recent_lines("2023-11-14T22:30:00Z later", NOW).len(), 1);
    }
}
