use serde_json::Value;

use lrcli::error::Error;

const RESTART_HINT: &str = "Restart Lightroom Classic, then open File > Plug-in Manager and select lrcli once so the plugin starts.";

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
        assert!(plugin_install(&installed).contains("Restart Lightroom Classic"));
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
