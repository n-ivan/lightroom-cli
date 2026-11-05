use std::fs::OpenOptions;
use std::io::Write;

use serde_json::{Value, json};

use crate::clock;
use crate::error::Error;
use crate::paths::Paths;

/// A recorded entry together with the exact line it was read from, so it can be
/// removed without re-serialising anything else in the file.
#[derive(Debug, Clone, PartialEq)]
pub struct Recorded {
    pub raw: String,
    pub entry: Value,
}

fn quote(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if plain {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

/// The command as a user would retype it, without the executable path.
pub fn command_line<I: IntoIterator<Item = String>>(args: I) -> String {
    let quoted: Vec<String> = args.into_iter().map(|arg| quote(&arg)).collect();
    format!("lrcli {}", quoted.join(" "))
}

pub fn entry(command: &str, catalog: &str, changes: Vec<Value>) -> Value {
    json!({
        "time": clock::iso8601(clock::now_secs()),
        "command": command,
        "catalog": catalog,
        "changes": changes,
    })
}

fn io_error(paths: &Paths, e: std::io::Error) -> Error {
    Error::io(
        "history_io_error",
        format!("Cannot update {}", paths.history().display()),
        e,
    )
}

pub const MAX_ENTRIES: usize = 500;

pub fn append(paths: &Paths, entry: &Value) -> Result<(), Error> {
    append_capped(paths, entry, MAX_ENTRIES)
}

fn append_capped(paths: &Paths, entry: &Value, cap: usize) -> Result<(), Error> {
    paths.ensure_dir()?;
    let mut line = entry.to_string();
    line.push('\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.history())
        .and_then(|mut file| file.write_all(line.as_bytes()))
        .map_err(|e| io_error(paths, e))?;
    trim(paths, cap)
}

fn trim(paths: &Paths, cap: usize) -> Result<(), Error> {
    let text = std::fs::read_to_string(paths.history()).map_err(|e| io_error(paths, e))?;
    let excess = text.lines().count().saturating_sub(cap);
    if excess == 0 {
        return Ok(());
    }
    let kept: Vec<&str> = text.lines().skip(excess).collect();
    write_atomically(paths, &format!("{}\n", kept.join("\n")))
}

fn write_atomically(paths: &Paths, text: &str) -> Result<(), Error> {
    let tmp = paths.history().with_extension("jsonl.tmp");
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, paths.history()))
        .map_err(|e| io_error(paths, e))
}

/// Oldest first. Lines that are not JSON objects are ignored.
pub fn load(paths: &Paths) -> Result<Vec<Recorded>, Error> {
    let text = match std::fs::read_to_string(paths.history()) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_error(paths, e)),
    };
    Ok(text
        .lines()
        .filter_map(|line| {
            let entry: Value = serde_json::from_str(line).ok()?;
            entry.is_object().then(|| Recorded {
                raw: line.to_string(),
                entry,
            })
        })
        .collect())
}

/// The most recent `steps` entries recorded for `catalog`, newest first.
pub fn latest_for_catalog(records: &[Recorded], catalog: &str, steps: usize) -> Vec<Recorded> {
    records
        .iter()
        .rev()
        .filter(|r| r.entry["catalog"].as_str() == Some(catalog))
        .take(steps)
        .cloned()
        .collect()
}

/// Reloads the file first so entries appended since `load` are kept.
pub fn remove(paths: &Paths, done: &[Recorded]) -> Result<(), Error> {
    let text = match std::fs::read_to_string(paths.history()) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(io_error(paths, e)),
    };
    let mut lines: Vec<&str> = text.lines().collect();
    for record in done {
        if let Some(index) = lines.iter().rposition(|line| *line == record.raw) {
            lines.remove(index);
        }
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    write_atomically(paths, &out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::TempDir::new().unwrap();
        let paths = Paths::at(dir.path().to_path_buf());
        (dir, paths)
    }

    fn record(paths: &Paths, command: &str, catalog: &str) {
        append(paths, &entry(command, catalog, vec![json!({ "id": 1 })])).unwrap();
    }

    #[test]
    fn only_the_newest_entries_are_kept_past_the_cap() {
        let (_dir, paths) = setup();
        for n in 1..=5 {
            append_capped(&paths, &entry(&format!("c{n}"), "/c", vec![]), 3).unwrap();
        }
        let commands: Vec<String> = load(&paths)
            .unwrap()
            .iter()
            .map(|r| r.entry["command"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(commands, ["c3", "c4", "c5"]);
        assert!(!paths.history().with_extension("jsonl.tmp").exists());
    }

    #[test]
    fn missing_history_is_empty() {
        let (_dir, paths) = setup();
        assert!(load(&paths).unwrap().is_empty());
    }

    #[test]
    fn appended_entries_load_oldest_first() {
        let (_dir, paths) = setup();
        record(&paths, "lrcli rate 3", "/a.lrcat");
        record(&paths, "lrcli rate 4", "/a.lrcat");
        let records = load(&paths).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].entry["command"], "lrcli rate 3");
        assert_eq!(records[1].entry["changes"], json!([{ "id": 1 }]));
        assert!(records[0].entry["time"].as_str().unwrap().ends_with('Z'));
    }

    #[test]
    fn latest_skips_other_catalogs() {
        let (_dir, paths) = setup();
        record(&paths, "one", "/a.lrcat");
        record(&paths, "two", "/b.lrcat");
        record(&paths, "three", "/a.lrcat");
        let records = load(&paths).unwrap();
        let picked = latest_for_catalog(&records, "/a.lrcat", 5);
        let commands: Vec<&str> = picked
            .iter()
            .map(|r| r.entry["command"].as_str().unwrap())
            .collect();
        assert_eq!(commands, ["three", "one"]);
        assert_eq!(latest_for_catalog(&records, "/a.lrcat", 1).len(), 1);
        assert!(latest_for_catalog(&records, "/c.lrcat", 1).is_empty());
    }

    #[test]
    fn remove_drops_only_the_given_entries() {
        let (_dir, paths) = setup();
        record(&paths, "one", "/a.lrcat");
        record(&paths, "two", "/b.lrcat");
        record(&paths, "three", "/a.lrcat");
        let records = load(&paths).unwrap();
        let picked = latest_for_catalog(&records, "/a.lrcat", 1);
        remove(&paths, &picked).unwrap();
        let left: Vec<String> = load(&paths)
            .unwrap()
            .iter()
            .map(|r| r.entry["command"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(left, ["one", "two"]);
    }

    #[test]
    fn malformed_lines_are_ignored_but_survive_a_rewrite() {
        let (_dir, paths) = setup();
        std::fs::write(paths.history(), "not json\n").unwrap();
        record(&paths, "one", "/a.lrcat");
        let records = load(&paths).unwrap();
        assert_eq!(records.len(), 1);
        remove(&paths, &records).unwrap();
        assert_eq!(
            std::fs::read_to_string(paths.history()).unwrap(),
            "not json\n"
        );
    }

    #[test]
    fn command_lines_are_shell_quoted() {
        let args = ["tag", "add", "Parent > Child", "it's", "--photo", "5"].map(String::from);
        assert_eq!(
            command_line(args),
            "lrcli tag add 'Parent > Child' 'it'\\''s' --photo 5"
        );
        assert_eq!(
            command_line(["apply", "Exposure2012+=0.5"].map(String::from)),
            "lrcli apply Exposure2012+=0.5"
        );
    }
}
