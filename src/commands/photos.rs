use std::io::Read;

use serde_json::{Map, Value, json};

use super::Context;
use crate::error::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rating {
    pub op: &'static str,
    pub value: u8,
}

impl Rating {
    fn to_json(&self) -> Value {
        json!({ "op": self.op, "value": self.value })
    }
}

/// Accepts `3` (equal to 3) or an operator followed by 0-5: `>=3`, `<=2`, `>3`, `<3`, `!=0`, `==4`.
pub fn parse_rating(text: &str) -> Result<Rating, String> {
    let text = text.trim();
    let malformed = || {
        format!(
            "invalid rating \"{text}\". Use a number from 0 to 5, optionally after one of ==, !=, >, <, >=, <= (for example 3, >=3, !=0)"
        )
    };
    let split = text
        .find(|c: char| c.is_ascii_digit())
        .ok_or_else(malformed)?;
    let (op_text, number) = text.split_at(split);
    let op = match op_text.trim() {
        "" | "==" | "=" => "==",
        "!=" => "!=",
        ">" => ">",
        "<" => "<",
        ">=" => ">=",
        "<=" => "<=",
        _ => return Err(malformed()),
    };
    let value: u8 = number.parse().map_err(|_| malformed())?;
    if value > 5 {
        return Err(malformed());
    }
    Ok(Rating { op, value })
}

pub fn parse_date(text: &str) -> Result<String, String> {
    let malformed = || format!("invalid date \"{text}\". Use YYYY-MM-DD, for example 2024-05-31");
    let parts: Vec<&str> = text.split('-').collect();
    let [year, month, day] = parts[..] else {
        return Err(malformed());
    };
    let shape_ok = [(year, 4), (month, 2), (day, 2)]
        .iter()
        .all(|(part, len)| part.len() == *len && part.bytes().all(|b| b.is_ascii_digit()));
    if !shape_ok {
        return Err(malformed());
    }
    let numbers: Option<Vec<u32>> = parts.iter().map(|p| p.parse().ok()).collect();
    let Some([year, month, day]) = numbers
        .as_deref()
        .and_then(|n| <[u32; 3]>::try_from(n).ok())
    else {
        return Err(malformed());
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return Err(malformed()),
    };
    if day == 0 || day > days {
        return Err(malformed());
    }
    Ok(text.to_string())
}

fn id_of(value: &Value) -> Option<i64> {
    match value {
        Value::Number(_) => value.as_i64(),
        Value::Object(object) => object.get("id").and_then(Value::as_i64),
        _ => None,
    }
}

fn ids_from_array(items: &[Value]) -> Result<Vec<i64>, Error> {
    items
        .iter()
        .map(|item| {
            id_of(item).ok_or_else(|| {
                Error::usage(format!(
                    "Cannot read a photo id from {item}. Expected integers or objects with an \"id\"."
                ))
            })
        })
        .collect()
}

/// Reads one id per line, or JSON: an array of ids or objects with "id",
/// or an object with a "photos" or "ids" array (the output of `search --json`).
pub fn parse_stdin_ids(input: &str) -> Result<Vec<i64>, Error> {
    let trimmed = input.trim();
    let ids = if trimmed.starts_with('[') || trimmed.starts_with('{') {
        let value: Value = serde_json::from_str(trimmed)
            .map_err(|e| Error::usage(format!("Stdin is not valid JSON ({e}).")))?;
        let items = match &value {
            Value::Array(items) => Some(items),
            Value::Object(object) => ["photos", "ids"]
                .iter()
                .find_map(|key| object.get(*key).and_then(Value::as_array)),
            _ => None,
        };
        let items = items.ok_or_else(|| {
            Error::usage(
                "Stdin JSON must be an array of ids, or an object with a \"photos\" or \"ids\" array.",
            )
        })?;
        ids_from_array(items)?
    } else {
        trimmed
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| {
                line.parse::<i64>().map_err(|_| {
                    Error::usage(format!(
                        "Cannot read a photo id from stdin line \"{line}\". Expected one integer id per line, or JSON."
                    ))
                })
            })
            .collect::<Result<_, _>>()?
    };
    if ids.is_empty() {
        return Err(Error::usage(
            "No photo ids on stdin. Pipe ids one per line, or the JSON output of `lrcli search --json`.",
        ));
    }
    Ok(ids)
}

pub fn read_stdin_ids() -> Result<Vec<i64>, Error> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| Error::io("io_error", "Cannot read stdin", e))?;
    parse_stdin_ids(&input)
}

#[derive(Debug, Default)]
pub struct SearchOptions {
    pub rating: Option<Rating>,
    pub flag: Option<String>,
    pub label: Option<String>,
    pub keywords: Vec<String>,
    pub text: Option<String>,
    pub filename: Option<String>,
    pub folder: Option<String>,
    pub collection: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub camera: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

pub fn search_args(options: &SearchOptions) -> Value {
    let mut filters = Map::new();
    if let Some(rating) = &options.rating {
        filters.insert("rating".into(), rating.to_json());
    }
    if !options.keywords.is_empty() {
        filters.insert("keywords".into(), json!(options.keywords));
    }
    let strings = [
        ("flag", &options.flag),
        ("label", &options.label),
        ("text", &options.text),
        ("filename", &options.filename),
        ("folder", &options.folder),
        ("collection", &options.collection),
        ("since", &options.since),
        ("until", &options.until),
        ("camera", &options.camera),
    ];
    for (name, value) in strings {
        if let Some(value) = value {
            filters.insert(name.into(), json!(value));
        }
    }
    let mut args = Map::new();
    args.insert("filters".into(), Value::Object(filters));
    if let Some(limit) = options.limit {
        args.insert("limit".into(), json!(limit));
    }
    if let Some(offset) = options.offset {
        args.insert("offset".into(), json!(offset));
    }
    Value::Object(args)
}

pub fn ids_only(reply: &Value) -> Value {
    let photos = reply
        .get("photos")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let ids: Vec<&Value> = photos.iter().filter_map(|p| p.get("id")).collect();
    let total = reply
        .get("total")
        .cloned()
        .unwrap_or_else(|| json!(ids.len()));
    json!({ "ok": true, "ids": ids, "total": total })
}

pub async fn search(ctx: &Context, options: &SearchOptions, ids: bool) -> Result<Value, Error> {
    let reply = ctx
        .connect()
        .await?
        .request("search", search_args(options))
        .await?;
    Ok(if ids { ids_only(&reply) } else { reply })
}

pub async fn select(ctx: &Context, photos: &[i64]) -> Result<Value, Error> {
    ctx.connect()
        .await?
        .request("select", json!({ "photos": photos }))
        .await
}

pub async fn selection(ctx: &Context, ids: bool) -> Result<Value, Error> {
    let reply = ctx.connect().await?.request("selection", json!({})).await?;
    Ok(if ids { ids_only(&reply) } else { reply })
}

pub fn info_args(photos: Option<&[i64]>, develop: bool) -> Value {
    let mut args = Map::new();
    if let Some(photos) = photos {
        args.insert("photos".into(), json!(photos));
    }
    args.insert("develop".into(), json!(develop));
    Value::Object(args)
}

pub async fn info(ctx: &Context, photos: Option<&[i64]>, develop: bool) -> Result<Value, Error> {
    ctx.connect()
        .await?
        .request("info", info_args(photos, develop))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rating_forms() {
        let cases = [
            ("3", "==", 3),
            ("0", "==", 0),
            ("==4", "==", 4),
            (">=3", ">=", 3),
            ("<=2", "<=", 2),
            (">3", ">", 3),
            ("<3", "<", 3),
            ("!=0", "!=", 0),
            (" >= 5 ", ">=", 5),
        ];
        for (text, op, value) in cases {
            assert_eq!(parse_rating(text), Ok(Rating { op, value }), "{text}");
        }
    }

    #[test]
    fn rating_rejects_malformed_input() {
        for text in ["", "6", "-1", "abc", ">=", "=>3", "3.5", ">>3", "3>", "10"] {
            assert!(parse_rating(text).is_err(), "{text}");
        }
    }

    #[test]
    fn dates() {
        assert!(parse_date("2024-02-29").is_ok());
        for text in [
            "2023-02-29",
            "2024-13-01",
            "2024-1-1",
            "24-01-01",
            "2024/01/01",
            "",
            "2024-01-00",
            "2024-01-+1",
        ] {
            assert!(parse_date(text).is_err(), "{text}");
        }
    }

    #[test]
    fn stdin_lines() {
        assert_eq!(parse_stdin_ids("1\n 2 \n\n3\n").unwrap(), vec![1, 2, 3]);
        assert_eq!(parse_stdin_ids("42").unwrap(), vec![42]);
    }

    #[test]
    fn stdin_json_shapes() {
        for input in [
            "[1,2]",
            r#"[{"id":1},{"id":2}]"#,
            r#"{"ok":true,"total":2,"photos":[{"id":1,"filename":"a"},{"id":2}]}"#,
            r#"{"ids":[1,2],"total":2}"#,
        ] {
            assert_eq!(parse_stdin_ids(input).unwrap(), vec![1, 2], "{input}");
        }
    }

    #[test]
    fn stdin_errors_are_usage_errors() {
        for input in [
            "",
            "  \n",
            "[]",
            "abc",
            "1\nx\n",
            "[1,\"a\"]",
            "{\"foo\":1}",
            "[1,",
            r#"{"photos":[]}"#,
        ] {
            let error = parse_stdin_ids(input).unwrap_err();
            assert_eq!(error.exit_code(), 2, "{input}");
        }
    }

    #[test]
    fn search_args_map_every_option() {
        let options = SearchOptions {
            rating: Some(Rating { op: ">=", value: 3 }),
            flag: Some("picked".into()),
            label: Some("red".into()),
            keywords: vec!["a".into(), "b".into()],
            text: Some("t".into()),
            filename: Some("f".into()),
            folder: Some("d".into()),
            collection: Some("c".into()),
            since: Some("2024-01-01".into()),
            until: Some("2024-02-01".into()),
            camera: Some("R5".into()),
            limit: Some(10),
            offset: Some(20),
        };
        assert_eq!(
            search_args(&options),
            json!({
                "filters": {
                    "rating": { "op": ">=", "value": 3 }, "flag": "picked", "label": "red",
                    "keywords": ["a", "b"], "text": "t", "filename": "f", "folder": "d",
                    "collection": "c", "since": "2024-01-01", "until": "2024-02-01", "camera": "R5",
                },
                "limit": 10, "offset": 20,
            })
        );
        assert_eq!(
            search_args(&SearchOptions::default()),
            json!({ "filters": {} })
        );
    }

    #[test]
    fn ids_only_keeps_total() {
        let reply = json!({ "ok": true, "total": 9, "photos": [{ "id": 4 }, { "id": 5 }] });
        assert_eq!(
            ids_only(&reply),
            json!({ "ok": true, "ids": [4, 5], "total": 9 })
        );
        assert_eq!(
            ids_only(&json!({ "photos": [] })),
            json!({ "ok": true, "ids": [], "total": 0 })
        );
    }
}
