use serde_json::Value;

use lrcli::commands::edit::differs_from_requested;
use lrcli::error::Error;

const RESTART_HINT: &str = "Run `lrcli restart` so Lightroom Classic loads the plugin.";

pub type Formatter = fn(&Value) -> String;

pub fn emit(result: Result<Value, Error>, json: bool, human: Formatter) -> u8 {
    match result {
        Ok(value) => {
            if json {
                println!("{value}");
            } else {
                println!("{}", human(&value));
            }
            0
        }
        Err(error) => {
            emit_error(&error, json);
            error.exit_code()
        }
    }
}

pub fn emit_error(error: &Error, json: bool) {
    if json {
        println!("{}", error_json(error));
    } else {
        eprintln!("error: {}", error.message);
    }
}

fn error_json(error: &Error) -> Value {
    serde_json::json!({ "ok": false, "error": error.code, "message": error.message })
}

fn text<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or("unknown")
}

fn number(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_i64)
        .map_or_else(|| "unknown".to_string(), |n| n.to_string())
}

pub fn status(value: &Value) -> String {
    format!(
        "Lightroom Classic {}\nPlugin:  {} (protocol {})\nCatalog: {}\nBroker:  {} (pid {})",
        text(value, "/lightroom/version"),
        text(value, "/plugin/version"),
        number(value, "/protocol"),
        text(value, "/catalog/path"),
        text(value, "/broker/version"),
        number(value, "/broker/pid"),
    )
}

pub fn start(value: &Value) -> String {
    let headline = if value["started"] == true {
        "Lightroom Classic started."
    } else {
        "Lightroom Classic is already running."
    };
    format!("{headline}\n{}", status(value))
}

pub fn stop(value: &Value) -> String {
    if value["stopped"] == true {
        "Lightroom Classic quit.".to_string()
    } else {
        "Lightroom Classic is not running.".to_string()
    }
}

pub fn plugin_install(value: &Value) -> String {
    let (source, target) = (text(value, "/source"), text(value, "/target"));
    match text(value, "/action") {
        "unchanged" => format!(
            "Already installed: {target} -> {source}\nBroker path: {}",
            text(value, "/brokerPath")
        ),
        action => {
            let verb = if action == "replaced" {
                "Replaced"
            } else {
                "Installed"
            };
            format!(
                "{verb} {target} -> {source}\nBroker path: {}\n{RESTART_HINT}",
                text(value, "/brokerPath")
            )
        }
    }
}

pub fn plugin_uninstall(value: &Value) -> String {
    let target = text(value, "/target");
    match text(value, "/action") {
        "removed" => format!("Removed {target}\nRestart Lightroom Classic to unload the plugin."),
        _ => format!("Not installed: nothing at {target}"),
    }
}

pub fn plugin_path(value: &Value) -> String {
    format!(
        "Source: {}\nTarget: {}\nConfig: {}",
        text(value, "/source"),
        text(value, "/target"),
        text(value, "/config")
    )
}

fn str_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn photos_of(value: &Value) -> &[Value] {
    value
        .get("photos")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
}

fn stars(value: &Value) -> String {
    let rating = value
        .get("rating")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(5) as usize;
    format!("{}{}", "★".repeat(rating), "☆".repeat(5 - rating))
}

fn flag_text(value: &Value) -> &str {
    match str_at(value, "flag") {
        Some("unflagged") | None => "-",
        Some(flag) => flag,
    }
}

fn captured_date(value: &Value) -> &str {
    str_at(value, "captureTime").map_or("-", |t| t.get(..10).unwrap_or(t))
}

pub fn photo_table(photos: &[Value]) -> String {
    let header = ["ID", "FILENAME", "RATING", "FLAG", "LABEL", "CAPTURED"];
    let rows: Vec<[String; 6]> = photos
        .iter()
        .map(|p| {
            [
                p.get("id")
                    .map_or_else(|| "?".to_string(), Value::to_string),
                str_at(p, "filename").unwrap_or("-").to_string(),
                stars(p),
                flag_text(p).to_string(),
                str_at(p, "label").unwrap_or("-").to_string(),
                captured_date(p).to_string(),
            ]
        })
        .collect();
    let widths: Vec<usize> = (0..header.len())
        .map(|i| {
            rows.iter()
                .map(|row| row[i].chars().count())
                .chain([header[i].chars().count()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: &[&str]| {
        let padded: Vec<String> = cells
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        padded.join("  ").trim_end().to_string()
    };
    let mut lines = vec![line(&header)];
    lines.extend(rows.iter().map(|row| {
        let cells: Vec<&str> = row.iter().map(String::as_str).collect();
        line(&cells)
    }));
    lines.join("\n")
}

pub fn search_footer(value: &Value) -> String {
    let count = photos_of(value).len() as u64;
    let total = value.get("total").and_then(Value::as_u64).unwrap_or(count);
    let offset = value.get("offset").and_then(Value::as_u64).unwrap_or(0);
    if count == 0 {
        return format!("No photos in range (total {total}).");
    }
    let last = offset + count;
    let mut footer = format!("Showing {}–{last} of {total}", offset + 1);
    if last < total {
        footer.push_str(&format!(". More: add --offset {last}"));
    }
    footer
}

pub fn search(value: &Value) -> String {
    if value.get("total").and_then(Value::as_u64) == Some(0) {
        return "No photos match.".to_string();
    }
    format!(
        "{}\n\n{}",
        photo_table(photos_of(value)),
        search_footer(value)
    )
}

pub fn selection(value: &Value) -> String {
    if photos_of(value).is_empty() {
        return "Nothing is selected in Lightroom.".to_string();
    }
    photo_table(photos_of(value))
}

pub fn ids(value: &Value) -> String {
    value
        .get("ids")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn select(value: &Value) -> String {
    let count = value
        .get("selected")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let noun = if count == 1 { "photo" } else { "photos" };
    let mut text = format!("Selected {count} {noun}");
    if value.get("sourceChanged").and_then(Value::as_bool) == Some(true) {
        text.push_str(" (switched source to All Photographs)");
    }
    text
}

fn info_block(photo: &Value) -> String {
    let mut lines = vec![format!(
        "{} (id {})",
        str_at(photo, "filename").unwrap_or("?"),
        photo
            .get("id")
            .map_or_else(|| "?".to_string(), Value::to_string)
    )];
    let mut field = |label: &str, text: Option<String>| {
        if let Some(text) = text {
            lines.push(format!("  {:<10}{text}", format!("{label}:")));
        }
    };
    field("Path", str_at(photo, "path").map(String::from));
    field("Captured", str_at(photo, "captureTime").map(String::from));
    let dimension = |key| photo.get(key).and_then(Value::as_u64);
    field(
        "Size",
        dimension("width")
            .zip(dimension("height"))
            .map(|(w, h)| format!("{w} x {h}")),
    );
    field("Format", str_at(photo, "fileFormat").map(String::from));
    field(
        "Rating",
        Some(format!(
            "{}  flag: {}  label: {}",
            stars(photo),
            flag_text(photo),
            str_at(photo, "label").unwrap_or("-")
        )),
    );
    field("Title", str_at(photo, "title").map(String::from));
    field("Caption", str_at(photo, "caption").map(String::from));
    let keywords: Vec<&str> = photo
        .get("keywords")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter_map(Value::as_str)
        .collect();
    field(
        "Keywords",
        (!keywords.is_empty()).then(|| keywords.join(", ")),
    );
    if let Some(exif) = photo.get("exif") {
        let part = |key| match exif.get(key) {
            Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        };
        let join = |parts: Vec<Option<String>>| {
            let parts: Vec<String> = parts.into_iter().flatten().collect();
            (!parts.is_empty()).then(|| parts.join(", "))
        };
        field("Camera", join(vec![part("camera"), part("lens")]));
        field(
            "Exposure",
            join(vec![
                part("shutter"),
                part("aperture"),
                part("focalLength"),
                part("iso"),
                part("exposureBias"),
            ]),
        );
    }
    if let Some(gps) = photo.get("gps").filter(|g| g.is_object()) {
        let coordinate = |key| gps.get(key).and_then(Value::as_f64);
        field(
            "GPS",
            coordinate("latitude")
                .zip(coordinate("longitude"))
                .map(|(lat, lon)| format!("{lat}, {lon}")),
        );
    }
    if let Some(develop) = photo.get("develop").and_then(Value::as_object) {
        lines.push("  Develop settings:".to_string());
        lines.extend(
            develop
                .iter()
                .map(|(key, setting)| format!("    {key}: {setting}")),
        );
    }
    lines.join("\n")
}

pub fn info(value: &Value) -> String {
    photos_of(value)
        .iter()
        .map(info_block)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn max_percent(stats: &Value, kind: &str) -> Option<f64> {
    let channels = stats.pointer(&format!("/clipping/{kind}"))?;
    ["r", "g", "b"]
        .iter()
        .filter_map(|c| channels.get(*c).and_then(Value::as_f64))
        .reduce(f64::max)
}

fn preview_line(photo: &Value) -> String {
    let name = str_at(photo, "filename").unwrap_or("?");
    let id = photo
        .get("id")
        .map_or_else(|| "?".to_string(), Value::to_string);
    let Some(file) = str_at(photo, "file") else {
        return format!(
            "{name} ({id})  error: {}",
            str_at(photo, "error").unwrap_or("no preview was produced")
        );
    };
    let mut line = format!("{name} ({id})  {file}");
    if let Some(stats) = photo.get("stats") {
        line.push_str(&stats_summary(stats));
    }
    line
}

fn stats_summary(stats: &Value) -> String {
    let mut text = String::new();
    let dimension = |key| stats.get(key).and_then(Value::as_u64);
    if let Some((width, height)) = dimension("width").zip(dimension("height")) {
        text.push_str(&format!("  {width}×{height}"));
    }
    if let Some(mean) = stats.get("meanLuminance").and_then(Value::as_f64) {
        text.push_str(&format!("  mean {mean:.2}"));
    }
    if let Some((high, low)) = max_percent(stats, "highlights").zip(max_percent(stats, "shadows")) {
        text.push_str(&format!("  clip hi {high:.1}% lo {low:.1}%"));
    }
    text
}

fn before_after_line(compare: &Value) -> String {
    let (before, after) = (&compare["before"]["stats"], &compare["after"]["stats"]);
    let mut parts = Vec::new();
    let mean = |stats: &Value| stats.get("meanLuminance").and_then(Value::as_f64);
    if let Some((b, a)) = mean(before).zip(mean(after)) {
        parts.push(format!("mean {b:.2} → {a:.2}"));
    }
    let clip = |stats: &Value, kind| max_percent(stats, kind);
    if let Some((b, a)) = clip(before, "highlights").zip(clip(after, "highlights")) {
        parts.push(format!("clip hi {b:.1}% → {a:.1}%"));
    }
    if let Some((b, a)) = clip(before, "shadows").zip(clip(after, "shadows")) {
        parts.push(format!("lo {b:.1}% → {a:.1}%"));
    }
    parts.join(", ")
}

pub fn compare(value: &Value) -> String {
    let side = |name: &str| {
        let side = &value[name];
        format!(
            "  {name:<6} {}{}",
            str_at(side, "file").unwrap_or("?"),
            stats_summary(&side["stats"])
        )
    };
    format!(
        "{}\n{}\n{}\n  {}",
        str_at(value, "file").unwrap_or("?"),
        side("before"),
        side("after"),
        before_after_line(value)
    )
}

pub fn preview(value: &Value) -> String {
    let mut lines: Vec<String> = photos_of(value).iter().map(preview_line).collect();
    if str_at(value, "mode") == Some("thumbnail") {
        lines.push("(fast preview: may not reflect recent edits)".to_string());
    }
    lines.join("\n")
}

fn human_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

fn export_line(photo: &Value) -> String {
    let name = str_at(photo, "filename").unwrap_or("?");
    let id = photo
        .get("id")
        .map_or_else(|| "?".to_string(), Value::to_string);
    if photo.get("skipped").and_then(Value::as_bool) == Some(true) {
        return format!("{name} ({id})  skipped: a file with that name already exists");
    }
    let Some(file) = str_at(photo, "file") else {
        return format!(
            "{name} ({id})  error: {}",
            str_at(photo, "error").unwrap_or("no file was exported")
        );
    };
    let mut line = format!("{name} ({id})  {file}");
    if let Some(bytes) = photo.get("bytes").and_then(Value::as_u64) {
        line.push_str(&format!("  {}", human_bytes(bytes)));
    }
    line
}

pub fn export(value: &Value) -> String {
    let photos = photos_of(value);
    let mut lines: Vec<String> = photos.iter().map(export_line).collect();
    let exported = photos
        .iter()
        .filter(|p| str_at(p, "file").is_some())
        .count();
    let skipped = photos
        .iter()
        .filter(|p| p.get("skipped").and_then(Value::as_bool) == Some(true))
        .count();
    let skipped_note = if skipped > 0 {
        format!(" ({skipped} skipped)")
    } else {
        String::new()
    };
    lines.push(format!(
        "Exported {exported} of {} to {}{skipped_note}",
        photos.len(),
        str_at(value, "dest").unwrap_or("?")
    ));
    lines.join("\n")
}

fn photo_label(photo: &Value) -> String {
    format!(
        "{} (id {})",
        str_at(photo, "filename").unwrap_or("?"),
        photo
            .get("id")
            .map_or_else(|| "?".to_string(), Value::to_string)
    )
}

fn photo_error(photo: &Value) -> Option<String> {
    str_at(photo, "error").map(|error| format!("{}  error: {error}", photo_label(photo)))
}

fn display_value(value: &Value) -> String {
    match value {
        Value::Null => "-".to_string(),
        Value::String(text) => text.clone(),
        Value::Number(number) => match number.as_f64() {
            Some(float) if float.fract() != 0.0 => format!("{float:.4}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string(),
            _ => number.to_string(),
        },
        other => other.to_string(),
    }
}

fn develop_block(photo: &Value) -> String {
    if let Some(error) = photo_error(photo) {
        return error;
    }
    let mut lines = vec![photo_label(photo)];
    let after = photo.get("after").and_then(Value::as_object);
    for (key, after) in after.into_iter().flatten() {
        let before = photo
            .pointer(&format!("/before/{key}"))
            .unwrap_or(&Value::Null);
        // Table values such as Look (the profile) run to hundreds of characters;
        // --json carries them in full.
        let mut line =
            if before.is_object() || after.is_object() || before.is_array() || after.is_array() {
                format!("  {key}: (changed; see --json)")
            } else {
                format!(
                    "  {key}: {} → {}",
                    display_value(before),
                    display_value(after)
                )
            };
        if let Some(requested) = photo.pointer(&format!("/requested/{key}"))
            && differs_from_requested(after, requested)
        {
            line.push_str(&format!(" (requested {})", display_value(requested)));
        }
        lines.push(line);
    }
    if lines.len() == 1 {
        lines.push("  (no changes)".to_string());
    }
    if let Some(compare) = photo.get("compare") {
        lines.push(format!(
            "  compare: {}",
            str_at(compare, "file").unwrap_or("?")
        ));
        lines.push(format!("    {}", before_after_line(compare)));
    }
    if let Some(message) = str_at(photo, "compareError") {
        lines.push(format!("  compare failed: {message}"));
    }
    lines.join("\n")
}

fn write_footer(value: &Value, lines: &mut Vec<String>) {
    if let Some(preview_reply) = value.get("preview") {
        lines.push(String::new());
        lines.push(preview(preview_reply));
    }
    if let Some(message) = value
        .pointer("/previewError/message")
        .and_then(Value::as_str)
    {
        lines.push(format!("preview failed: {message}"));
    }
    if let Some(message) = value
        .pointer("/compareError/message")
        .and_then(Value::as_str)
    {
        lines.push(format!("compare failed: {message}"));
    }
    if let Some(message) = str_at(value, "historyWarning") {
        lines.push(format!("warning: {message}"));
    }
}

pub fn apply(value: &Value) -> String {
    let mut lines: Vec<String> = photos_of(value).iter().map(develop_block).collect();
    write_footer(value, &mut lines);
    lines.join("\n")
}

pub fn preset(value: &Value) -> String {
    let mut lines = vec![format!(
        "Preset {}/{}",
        text(value, "/preset/folder"),
        text(value, "/preset/name")
    )];
    lines.extend(photos_of(value).iter().map(develop_block));
    write_footer(value, &mut lines);
    lines.join("\n")
}

pub fn module(value: &Value) -> String {
    let current = text(value, "/current");
    match str_at(value, "previous") {
        Some(previous) if previous == current => format!("Already in {current}"),
        Some(previous) => format!("Switched to {current} (was {previous})"),
        None => format!("Switched to {current}"),
    }
}

pub fn scalar(value: &Value) -> String {
    let mut lines: Vec<String> = photos_of(value)
        .iter()
        .map(|photo| {
            photo_error(photo).unwrap_or_else(|| {
                let (before, after) = (
                    display_value(photo.get("before").unwrap_or(&Value::Null)),
                    display_value(photo.get("after").unwrap_or(&Value::Null)),
                );
                if before == after {
                    format!("{}: already {after}", photo_label(photo))
                } else {
                    format!("{}: {before} → {after}", photo_label(photo))
                }
            })
        })
        .collect();
    write_footer(value, &mut lines);
    lines.join("\n")
}

pub fn tag(value: &Value) -> String {
    let names = |photo: &Value, key: &str, sign: &str| -> Vec<String> {
        photo
            .get(key)
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .filter_map(Value::as_str)
            .map(|name| format!("{sign}{name}"))
            .collect()
    };
    let mut lines: Vec<String> = photos_of(value)
        .iter()
        .map(|photo| {
            photo_error(photo).unwrap_or_else(|| {
                let mut changes = names(photo, "added", "+");
                changes.extend(names(photo, "removed", "-"));
                let summary = if changes.is_empty() {
                    "no change".to_string()
                } else {
                    changes.join(", ")
                };
                format!("{}: {summary}", photo_label(photo))
            })
        })
        .collect();
    write_footer(value, &mut lines);
    lines.join("\n")
}

pub fn presets(value: &Value) -> String {
    let folders = value
        .get("folders")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if folders.is_empty() {
        return "No develop presets.".to_string();
    }
    let mut lines = Vec::new();
    for folder in folders {
        lines.push(str_at(folder, "name").unwrap_or("?").to_string());
        for preset in folder
            .get("presets")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
        {
            lines.push(format!("  {}", str_at(preset, "name").unwrap_or("?")));
        }
    }
    lines.join("\n")
}

fn change_summary(change: &Value) -> String {
    let fields: Vec<String> = [
        ("develop", "develop"),
        ("rating", "rating"),
        ("flag", "flag"),
        ("label", "label"),
        ("keywordsAdded", "keywords added"),
        ("keywordsRemoved", "keywords removed"),
    ]
    .iter()
    .filter(|(key, _)| {
        change.get(*key).is_some_and(|v| match v {
            Value::Array(list) => !list.is_empty(),
            other => !other.is_null(),
        })
    })
    .map(|(_, label)| (*label).to_string())
    .collect();
    format!(
        "photo {}: {}",
        change
            .get("id")
            .map_or_else(|| "?".to_string(), Value::to_string),
        fields.join(", ")
    )
}

fn changes_of(item: &Value) -> &[Value] {
    item.get("changes")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
}

pub fn undo(value: &Value) -> String {
    let dry_run = value["dryRun"] == true;
    let items = value
        .get(if dry_run { "undo" } else { "undone" })
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let mut lines = Vec::new();
    for item in items {
        let verb = if dry_run { "Would undo" } else { "Undid" };
        lines.push(format!(
            "{verb}: {} ({})",
            text(item, "/command"),
            text(item, "/time")
        ));
        if !dry_run {
            lines.push(if item["method"] == "native" {
                "  undone with Lightroom's undo".to_string()
            } else {
                match str_at(item, "nativeSkipped") {
                    Some(reason) => {
                        format!("  restored values (Lightroom's undo not used: {reason})")
                    }
                    None => "  restored values".to_string(),
                }
            });
        }
        if dry_run {
            lines.extend(
                changes_of(item)
                    .iter()
                    .map(|change| format!("  {}", change_summary(change))),
            );
            continue;
        }
        for photo in photos_of(item) {
            if let Some(error) = str_at(photo, "error") {
                lines.push(format!("  {}  error: {error}", photo_label(photo)));
                continue;
            }
            let restored: Vec<&str> = photo
                .get("restored")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .filter_map(Value::as_str)
                .collect();
            let summary = if restored.is_empty() {
                "nothing to restore".to_string()
            } else {
                format!("restored {}", restored.join(", "))
            };
            lines.push(format!("  {}: {summary}", photo_label(photo)));
            for warning in photo
                .get("warnings")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .filter_map(Value::as_str)
            {
                lines.push(format!("    warning: {warning}"));
            }
        }
    }
    lines.join("\n")
}

pub fn history(value: &Value) -> String {
    let entries = value
        .get("entries")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if entries.is_empty() {
        return "No recorded changes.".to_string();
    }
    entries
        .iter()
        .map(|entry| {
            let count = changes_of(entry).len();
            let noun = if count == 1 { "photo" } else { "photos" };
            format!(
                "{}  {}  ({count} {noun})",
                text(entry, "/time"),
                text(entry, "/command")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn range_text(entry: &Value) -> String {
    format!(
        "{}..{}",
        display_value(&entry["min"]),
        display_value(&entry["max"])
    )
}

pub fn settings(value: &Value) -> String {
    let panels = value["panels"].as_array().map_or(&[][..], Vec::as_slice);
    let keys_of = |panel: &Value| panel["keys"].as_array().cloned().unwrap_or_default();
    let key_width = panels
        .iter()
        .flat_map(keys_of)
        .map(|k| str_at(&k, "key").map_or(1, |key| key.chars().count()))
        .max()
        .unwrap_or(0);
    let range_width = panels
        .iter()
        .flat_map(keys_of)
        .map(|k| range_text(&k).chars().count())
        .max()
        .unwrap_or(0);
    let mut blocks = Vec::new();
    if let Some(photo) = value.get("photo") {
        let kind = if value["fileType"] == "raw" {
            "raw"
        } else {
            "non-raw"
        };
        blocks.push(format!("Photo {photo} ({kind} file)"));
    }
    for panel in panels {
        let mut lines = vec![str_at(panel, "title").unwrap_or("?").to_string()];
        for key in keys_of(panel) {
            let mut line = format!(
                "  {:<key_width$}  {:<range_width$}",
                str_at(&key, "key").unwrap_or("?"),
                range_text(&key)
            );
            if let Some(current) = key.get("value") {
                line.push_str(&format!("  [{}]", display_value(current)));
            }
            if key["nonRawOnly"] == true {
                line.push_str("  (non-raw only)");
            } else if let Some(non_raw) = key.get("nonRaw") {
                line.push_str(&format!("  (non-raw {})", range_text(non_raw)));
            }
            lines.push(line.trim_end().to_string());
        }
        blocks.push(lines.join("\n"));
    }
    blocks.join("\n\n")
}

pub fn raw(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("serializing a Value cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_lists_every_field() {
        let reply = json!({
            "ok": true, "protocol": 1,
            "plugin": { "version": "0.1.0" },
            "lightroom": { "version": "15.0" },
            "catalog": { "path": "/c.lrcat" },
            "broker": { "version": "0.1.0", "pid": 42 },
        });
        assert_eq!(
            status(&reply),
            "Lightroom Classic 15.0\nPlugin:  0.1.0 (protocol 1)\nCatalog: /c.lrcat\nBroker:  0.1.0 (pid 42)"
        );
    }

    #[test]
    fn status_tolerates_missing_fields() {
        assert!(status(&json!({ "ok": true })).contains("unknown"));
    }

    #[test]
    fn errors_render_as_json_without_an_id() {
        let error = Error::failed("boom", "it broke");
        assert_eq!(
            error_json(&error),
            json!({ "ok": false, "error": "boom", "message": "it broke" })
        );
    }

    #[test]
    fn install_output_mentions_restart_only_when_something_changed() {
        let installed =
            json!({ "action": "installed", "source": "/s", "target": "/t", "brokerPath": "/b" });
        assert!(plugin_install(&installed).contains("lrcli restart"));
        let unchanged =
            json!({ "action": "unchanged", "source": "/s", "target": "/t", "brokerPath": "/b" });
        assert!(!plugin_install(&unchanged).contains("Restart"));
    }

    fn sample_photo() -> Value {
        json!({
            "id": 12, "filename": "IMG_1.CR3", "path": "/p/IMG_1.CR3",
            "captureTime": "2024-05-31T10:00:00Z", "rating": 3, "flag": "picked",
            "label": "red", "width": 6000, "height": 4000,
        })
    }

    #[test]
    fn table_aligns_columns_and_shows_placeholders() {
        let bare = json!({ "id": 100, "filename": "b.jpg", "rating": 0, "flag": "unflagged",
            "label": null, "captureTime": null });
        let table = photo_table(&[sample_photo(), bare]);
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines[0], "ID   FILENAME   RATING  FLAG    LABEL  CAPTURED");
        assert_eq!(
            lines[1],
            "12   IMG_1.CR3  ★★★☆☆   picked  red    2024-05-31"
        );
        assert_eq!(lines[2], "100  b.jpg      ☆☆☆☆☆   -       -      -");
    }

    #[test]
    fn search_footer_hints_the_next_page_only_when_more_exist() {
        let page =
            json!({ "total": 120, "offset": 50, "limit": 50, "photos": vec![sample_photo(); 50] });
        assert_eq!(
            search_footer(&page),
            "Showing 51–100 of 120. More: add --offset 100"
        );
        let last =
            json!({ "total": 3, "offset": 0, "limit": 50, "photos": vec![sample_photo(); 3] });
        assert_eq!(search_footer(&last), "Showing 1–3 of 3");
        assert_eq!(
            search(&json!({ "total": 0, "offset": 0, "photos": [] })),
            "No photos match."
        );
    }

    #[test]
    fn export_lines_show_sizes_errors_and_the_footer() {
        let reply = json!({ "dest": "/out", "photos": [
            { "id": 149, "filename": "DSC08011-2.ARW", "file": "/out/DSC08011-2.jpg", "bytes": 3_355_443 },
            { "id": 150, "filename": "small.ARW", "file": "/out/small.jpg", "bytes": 512 },
            { "id": 151, "filename": "v.mov", "file": null, "error": "Videos cannot be exported." },
        ] });
        let lines: Vec<String> = export(&reply).lines().map(String::from).collect();
        assert_eq!(
            lines[0],
            "DSC08011-2.ARW (149)  /out/DSC08011-2.jpg  3.2 MB"
        );
        assert_eq!(lines[1], "small.ARW (150)  /out/small.jpg  512 B");
        assert_eq!(lines[2], "v.mov (151)  error: Videos cannot be exported.");
        assert_eq!(lines[3], "Exported 2 of 3 to /out");
        assert_eq!(human_bytes(2048), "2.0 KB");
    }

    #[test]
    fn preview_lines_show_stats_errors_and_the_fast_note() {
        let reply = json!({
            "mode": "thumbnail",
            "photos": [
                { "id": 149, "filename": "DSC08011-2.ARW", "file": "/p/lrcli-x.jpg", "error": null,
                  "stats": { "width": 1600, "height": 1067, "meanLuminance": 0.42,
                    "clipping": {
                      "highlights": { "r": 0.3, "g": 0.1, "b": 0.0, "luma": 0.0 },
                      "shadows": { "r": 1.2, "g": 0.9, "b": 1.1, "luma": 0.8 } } } },
                { "id": 7, "filename": "v.mov", "file": null, "error": "Videos cannot be previewed." },
                { "id": 8, "filename": "b.cr3", "file": "/p/b.jpg", "error": null },
            ],
        });
        let text = preview(&reply);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            "DSC08011-2.ARW (149)  /p/lrcli-x.jpg  1600×1067  mean 0.42  clip hi 0.3% lo 1.2%"
        );
        assert_eq!(lines[1], "v.mov (7)  error: Videos cannot be previewed.");
        assert_eq!(lines[2], "b.cr3 (8)  /p/b.jpg");
        assert_eq!(lines[3], "(fast preview: may not reflect recent edits)");
    }

    #[test]
    fn selection_and_select_text() {
        assert_eq!(
            selection(&json!({ "photos": [] })),
            "Nothing is selected in Lightroom."
        );
        assert_eq!(select(&json!({ "selected": [1] })), "Selected 1 photo");
        assert_eq!(
            select(&json!({ "selected": [1, 2], "sourceChanged": true })),
            "Selected 2 photos (switched source to All Photographs)"
        );
        assert_eq!(ids(&json!({ "ids": [3, 4] })), "3\n4");
    }

    #[test]
    fn info_block_lists_available_fields_and_sorted_develop_settings() {
        let mut photo = sample_photo();
        photo["title"] = json!("T");
        photo["caption"] = Value::Null;
        photo["keywords"] = json!(["a", "b"]);
        photo["exif"] = json!({ "camera": "R5", "lens": null, "iso": "ISO 100", "shutter": "1/250 sec",
            "aperture": "ƒ / 2.8", "focalLength": "50 mm", "exposureBias": "0 EV" });
        photo["gps"] = json!({ "latitude": 1.5, "longitude": -2.0 });
        photo["develop"] = json!({ "Temperature": 5500, "Exposure2012": 0.5 });
        let text = info(&json!({ "target": "ids", "photos": [photo] }));
        assert!(text.starts_with("IMG_1.CR3 (id 12)\n"), "{text}");
        for expected in [
            "Size:     6000 x 4000",
            "Rating:   ★★★☆☆  flag: picked  label: red",
            "Title:    T",
            "Keywords: a, b",
            "Camera:   R5",
            "Exposure: 1/250 sec, ƒ / 2.8, 50 mm, ISO 100, 0 EV",
            "GPS:      1.5, -2",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in\n{text}");
        }
        assert!(!text.contains("Caption"));
        assert!(text.find("Exposure2012").unwrap() < text.find("Temperature").unwrap());
    }
}
