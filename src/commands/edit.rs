use std::path::Path;
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::preview::{self, PreviewOptions};
use super::{Context, status};
use crate::compare;
use crate::error::Error;
use crate::history::{self, Recorded};
use crate::settings::{self, Setting};

/// Differences below this are Lightroom rounding, not clamping.
const REQUESTED_TOLERANCE: f64 = 0.005;

pub struct Run<'a> {
    pub command_line: &'a str,
    pub preview: bool,
    pub compare: bool,
    /// The user's `--timeout`, if any; otherwise previews get a batch-sized default.
    pub timeout: Option<Duration>,
}

fn targets(ids: Option<&[i64]>) -> Map<String, Value> {
    let mut args = Map::new();
    if let Some(ids) = ids {
        args.insert("photos".into(), json!(ids));
    }
    args
}

fn photos_of(reply: &Value) -> &[Value] {
    reply
        .get("photos")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
}

fn first_failure(reply: &Value) -> Option<String> {
    let photos = photos_of(reply);
    if photos.is_empty() || photos.iter().any(|p| p.get("error").is_none()) {
        return None;
    }
    photos[0]
        .get("error")
        .and_then(Value::as_str)
        .map(String::from)
}

async fn catalog_path(ctx: &Context) -> Result<String, Error> {
    let hello = status::run(ctx).await?;
    hello
        .pointer("/catalog/path")
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| Error::failed("no_catalog", "Lightroom did not report an open catalog."))
}

fn develop_changes(reply: &Value) -> Vec<Value> {
    photos_of(reply)
        .iter()
        .filter_map(|photo| {
            let before = photo.get("before")?.as_object()?;
            let after = photo.get("after")?.as_object()?;
            let mut recorded_before = Map::new();
            let mut recorded_after = Map::new();
            for (key, value) in before {
                let now = after.get(key).unwrap_or(&Value::Null);
                if now != value {
                    recorded_before.insert(key.clone(), value.clone());
                    recorded_after.insert(key.clone(), now.clone());
                }
            }
            (!recorded_before.is_empty()).then(|| {
                json!({ "id": photo["id"], "develop": recorded_before, "developAfter": recorded_after })
            })
        })
        .collect()
}

fn scalar_changes(reply: &Value, field: &'static str) -> Vec<Value> {
    let after_field = format!("{field}After");
    photos_of(reply)
        .iter()
        .filter(|photo| photo.get("before") != photo.get("after") && photo.get("error").is_none())
        .map(|photo| json!({ "id": photo["id"], field: photo["before"], after_field.as_str(): photo["after"] }))
        .collect()
}

fn keyword_changes(reply: &Value) -> Vec<Value> {
    let non_empty = |photo: &Value, key: &str| {
        photo
            .get(key)
            .and_then(Value::as_array)
            .is_some_and(|list| !list.is_empty())
    };
    photos_of(reply)
        .iter()
        .filter(|photo| non_empty(photo, "added") || non_empty(photo, "removed"))
        .map(|photo| {
            json!({
                "id": photo["id"],
                "keywordsAdded": photo["added"],
                "keywordsRemoved": photo["removed"],
            })
        })
        .collect()
}

fn annotate_requested(reply: &mut Value, settings: &[Setting]) {
    let Some(photos) = reply.get_mut("photos").and_then(Value::as_array_mut) else {
        return;
    };
    for photo in photos {
        let Some(before) = photo.get("before").cloned() else {
            continue;
        };
        let mut requested = Map::new();
        for setting in settings {
            match setting {
                Setting::Set(key, value) => {
                    requested.insert(key.clone(), value.clone());
                }
                Setting::Add(key, delta) => {
                    if let Some(now) = before.get(key).and_then(Value::as_f64) {
                        requested.insert(key.clone(), json!(now + delta));
                    }
                }
            }
        }
        photo["requested"] = Value::Object(requested);
    }
}

/// True when Lightroom stored something other than what was asked for.
pub fn differs_from_requested(after: &Value, requested: &Value) -> bool {
    match (after.as_f64(), requested.as_f64()) {
        (Some(a), Some(r)) => (a - r).abs() > REQUESTED_TOLERANCE,
        _ => after != requested,
    }
}

fn error_json(e: &Error) -> Value {
    json!({ "error": e.code, "message": e.message })
}

fn succeeded_ids(reply: &Value) -> Vec<i64> {
    photos_of(reply)
        .iter()
        .filter(|p| p.get("error").is_none())
        .filter_map(|p| p.get("id").and_then(Value::as_i64))
        .collect()
}

async fn render(
    ctx: &Context,
    run: &Run<'_>,
    ids: Option<Vec<i64>>,
    stats: bool,
) -> Result<Value, Error> {
    let preview_ctx = Context {
        paths: ctx.paths.clone(),
        timeout: run
            .timeout
            .unwrap_or_else(|| preview::default_timeout(ids.as_deref())),
        connect_timeout: ctx.connect_timeout,
    };
    let options = PreviewOptions {
        ids,
        size: preview::DEFAULT_SIZE,
        fast: false,
        histogram: false,
        stats,
    };
    preview::run(&preview_ctx, &options).await
}

async fn attach_preview(ctx: &Context, run: &Run<'_>, reply: &mut Value) {
    let ids = succeeded_ids(reply);
    if ids.is_empty() {
        return;
    }
    match render(ctx, run, Some(ids), true).await {
        Ok(result) => reply["preview"] = result,
        Err(e) => reply["previewError"] = error_json(&e),
    }
}

fn preview_file<'a>(preview: &'a Value, id: &Value) -> Option<&'a str> {
    photos_of(preview)
        .iter()
        .find(|p| p["id"] == *id)?
        .get("file")?
        .as_str()
}

async fn render_before(
    ctx: &Context,
    run: &Run<'_>,
    ids: Option<&[i64]>,
) -> Option<Result<Value, Error>> {
    if !run.compare {
        return None;
    }
    Some(render(ctx, run, ids.map(<[i64]>::to_vec), false).await)
}

/// Renders the after previews and composes each photo's before/after image.
/// Problems are reported in the reply; the write has already happened.
async fn attach_compare(
    ctx: &Context,
    run: &Run<'_>,
    reply: &mut Value,
    before: Result<Value, Error>,
) {
    let before = match before {
        Ok(before) => before,
        Err(e) => {
            reply["compareError"] = error_json(&e);
            return;
        }
    };
    let ids = succeeded_ids(reply);
    if ids.is_empty() {
        return;
    }
    let after = match render(ctx, run, Some(ids), false).await {
        Ok(after) => after,
        Err(e) => {
            reply["compareError"] = error_json(&e);
            return;
        }
    };
    let Some(photos) = reply.get_mut("photos").and_then(Value::as_array_mut) else {
        return;
    };
    for photo in photos.iter_mut().filter(|p| p.get("error").is_none()) {
        let id = photo["id"].clone();
        let Some((before_file, after_file)) =
            preview_file(&before, &id).zip(preview_file(&after, &id))
        else {
            photo["compareError"] =
                json!("Lightroom produced no before/after preview of this photo.");
            continue;
        };
        let after_path = Path::new(after_file);
        match compare::compare_files(
            Path::new(before_file),
            after_path,
            &compare::default_output(after_path),
        ) {
            Ok(result) => photo["compare"] = result,
            Err(e) => photo["compareError"] = json!(e.message),
        }
    }
}

async fn attach_previews(
    ctx: &Context,
    run: &Run<'_>,
    reply: &mut Value,
    before: Option<Result<Value, Error>>,
) {
    if let Some(before) = before {
        attach_compare(ctx, run, reply, before).await;
    } else if run.preview {
        attach_preview(ctx, run, reply).await;
    }
}

/// Sends a write command, records what changed for `undo`, and optionally
/// renders previews. A history failure is reported in the reply rather than
/// failing a command whose edit already happened.
async fn write(
    ctx: &Context,
    run: &Run<'_>,
    cmd: &str,
    args: Map<String, Value>,
    changes_of: fn(&Value) -> Vec<Value>,
) -> Result<Value, Error> {
    let catalog = catalog_path(ctx).await?;
    let mut reply = ctx
        .connect()
        .await?
        .request(cmd, Value::Object(args))
        .await?;
    if let Some(message) = first_failure(&reply) {
        return Err(Error::failed("write_failed", message));
    }
    let changes = changes_of(&reply);
    if !changes.is_empty() {
        let entry = history::entry(run.command_line, &catalog, changes);
        if let Err(e) = history::append(&ctx.paths, &entry) {
            reply["historyWarning"] = json!(format!("{} Undo will not cover this change.", e));
        }
    }
    Ok(reply)
}

pub async fn apply(
    ctx: &Context,
    run: &Run<'_>,
    ids: Option<&[i64]>,
    settings: &[Setting],
) -> Result<Value, Error> {
    let (set, add) = settings::to_args(settings)?;
    let mut args = targets(ids);
    args.insert("set".into(), Value::Object(set));
    args.insert("add".into(), Value::Object(add));
    let before = render_before(ctx, run, ids).await;
    let mut reply = write(ctx, run, "apply", args, develop_changes).await?;
    annotate_requested(&mut reply, settings);
    attach_previews(ctx, run, &mut reply, before).await;
    Ok(reply)
}

pub async fn presets(ctx: &Context) -> Result<Value, Error> {
    ctx.connect().await?.request("presets", json!({})).await
}

/// Preset uuids look like 8-4-4-4-12 hex digits.
pub fn looks_like_uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(group, len)| group.len() == len && group.bytes().all(|b| b.is_ascii_hexdigit()))
}

pub async fn preset(
    ctx: &Context,
    run: &Run<'_>,
    ids: Option<&[i64]>,
    name_or_uuid: &str,
    amount: Option<u32>,
) -> Result<Value, Error> {
    let mut args = targets(ids);
    let field = if looks_like_uuid(name_or_uuid) {
        "uuid"
    } else {
        "name"
    };
    args.insert(field.into(), json!(name_or_uuid));
    if let Some(amount) = amount {
        args.insert("amount".into(), json!(amount));
    }
    let before = render_before(ctx, run, ids).await;
    let mut reply = write(ctx, run, "preset", args, develop_changes).await?;
    attach_previews(ctx, run, &mut reply, before).await;
    Ok(reply)
}

/// Builds the plugin's action list: the positional actions in order, then one
/// `reset` action for all `--reset` keys.
pub fn develop_actions(actions: &[String], reset: &[String]) -> Result<Vec<Value>, Error> {
    let mut list: Vec<Value> = actions.iter().map(|a| json!({ "action": a })).collect();
    if !reset.is_empty() {
        if let Some(blank) = reset.iter().find(|key| key.trim().is_empty()) {
            return Err(Error::usage(format!(
                "--reset needs develop setting names, got {blank:?}."
            )));
        }
        list.push(json!({ "action": "reset", "keys": reset }));
    }
    if list.is_empty() {
        return Err(Error::usage(
            "Give at least one ACTION (auto-tone, auto-wb, as-shot-wb, reset-all) or --reset KEY. See `lrcli develop --help`.",
        ));
    }
    Ok(list)
}

pub async fn develop(
    ctx: &Context,
    run: &Run<'_>,
    ids: Option<&[i64]>,
    actions: Vec<Value>,
) -> Result<Value, Error> {
    let mut args = targets(ids);
    args.insert("actions".into(), Value::Array(actions));
    let before = render_before(ctx, run, ids).await;
    let mut reply = write(ctx, run, "develop", args, develop_changes).await?;
    attach_previews(ctx, run, &mut reply, before).await;
    Ok(reply)
}

pub async fn tag(
    ctx: &Context,
    run: &Run<'_>,
    ids: Option<&[i64]>,
    add: &[String],
    remove: &[String],
) -> Result<Value, Error> {
    let mut args = targets(ids);
    if !add.is_empty() {
        args.insert("add".into(), json!(add));
    }
    if !remove.is_empty() {
        args.insert("remove".into(), json!(remove));
    }
    write(ctx, run, "tag", args, keyword_changes).await
}

pub async fn rate(
    ctx: &Context,
    run: &Run<'_>,
    ids: Option<&[i64]>,
    rating: u8,
) -> Result<Value, Error> {
    let mut args = targets(ids);
    args.insert("rating".into(), json!(rating));
    write(ctx, run, "rate", args, |r| scalar_changes(r, "rating")).await
}

pub async fn flag(
    ctx: &Context,
    run: &Run<'_>,
    ids: Option<&[i64]>,
    flag: &str,
) -> Result<Value, Error> {
    let mut args = targets(ids);
    args.insert("flag".into(), json!(flag));
    write(ctx, run, "flag", args, |r| scalar_changes(r, "flag")).await
}

pub async fn label(
    ctx: &Context,
    run: &Run<'_>,
    ids: Option<&[i64]>,
    label: &str,
) -> Result<Value, Error> {
    let mut args = targets(ids);
    args.insert("label".into(), json!(label));
    write(ctx, run, "label", args, |r| scalar_changes(r, "label")).await
}

fn describe(record: &Recorded) -> Value {
    json!({
        "time": record.entry["time"],
        "command": record.entry["command"],
        "changes": record.entry["changes"],
    })
}

const AFTER_FIELDS: [&str; 4] = ["developAfter", "ratingAfter", "flagAfter", "labelAfter"];

/// What a change's photo should still hold if nothing touched it since lrcli
/// wrote it; None for entries recorded without after-values.
fn expect_after(change: &Value) -> Option<Value> {
    let mut state = Map::new();
    for field in ["develop", "rating", "flag", "label"] {
        if let Some(value) = change.get(format!("{field}After")).filter(|v| !v.is_null()) {
            state.insert(field.into(), value.clone());
        }
    }
    for (field, source) in [
        ("keywordsPresent", "keywordsAdded"),
        ("keywordsAbsent", "keywordsRemoved"),
    ] {
        if let Some(names) = change
            .get(source)
            .filter(|v| v.as_array().is_some_and(|names| !names.is_empty()))
        {
            state.insert(field.into(), names.clone());
        }
    }
    (!state.is_empty()).then_some(Value::Object(state))
}

fn changes_in(record: &Recorded) -> &[Value] {
    record.entry["changes"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
}

fn restore_changes(record: &Recorded) -> Vec<Value> {
    changes_in(record)
        .iter()
        .map(|recorded| {
            let mut change = recorded.clone();
            if let Some(fields) = change.as_object_mut() {
                for field in AFTER_FIELDS {
                    fields.remove(field);
                }
            }
            if let Some(expected) = expect_after(recorded) {
                change["expectAfter"] = expected;
            }
            change
        })
        .collect()
}

/// Develop keys with no value before the change: restoring cannot clear them.
fn uncleared_keys(change: &Value) -> Vec<String> {
    change["develop"]
        .as_object()
        .map(|develop| {
            develop
                .iter()
                .filter(|(_, value)| value.is_null())
                .map(|(key, _)| key.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn warn_uncleared(record: &Recorded, reply: &mut Value) {
    let Some(photos) = reply.get_mut("photos").and_then(Value::as_array_mut) else {
        return;
    };
    for change in changes_in(record) {
        let keys = uncleared_keys(change);
        if keys.is_empty() {
            continue;
        }
        let Some(photo) = photos.iter_mut().find(|p| p["id"] == change["id"]) else {
            continue;
        };
        let warning = format!(
            "{} had no value before and could not be cleared; it keeps its current value.",
            keys.join(", ")
        );
        match photo.get_mut("warnings").and_then(Value::as_array_mut) {
            Some(warnings) => warnings.push(json!(warning)),
            None => photo["warnings"] = json!([warning]),
        }
    }
}

fn after_earlier_steps(e: Error, done: usize) -> Error {
    if done == 0 {
        return e;
    }
    Error::new(
        e.kind,
        &e.code,
        format!(
            "{} ({done} earlier step(s) were already undone.)",
            e.message
        ),
    )
}

/// Per photo, the line telling why `undo` left it alone; None if it had no conflict.
pub fn conflict_line(photo: &Value) -> Option<String> {
    let reasons: Vec<&str> = photo
        .get("conflict")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    if reasons.is_empty() {
        return None;
    }
    Some(format!(
        "{} (id {}): skipped: changed since lrcli edited it ({}). Re-run with --force to restore anyway.",
        photo["filename"].as_str().unwrap_or("?"),
        photo["id"],
        reasons.join("; ")
    ))
}

fn conflicts_error(photos: &Value, record: &Recorded, done: usize) -> Option<Error> {
    let lines: Vec<String> = photos
        .as_array()?
        .iter()
        .filter_map(conflict_line)
        .collect();
    if lines.is_empty() {
        return None;
    }
    let mut message = format!(
        "Not undone: {}\n{}\nNo photo of this step was restored and the step is still in the history.",
        record.entry["command"].as_str().unwrap_or("unknown"),
        lines.join("\n")
    );
    if done > 0 {
        message.push_str(&format!(" {done} earlier step(s) were already undone."));
    }
    Some(Error::failed("undo_conflict", message))
}

fn restore_request(record: &Recorded, force: bool, dry_run: bool) -> Value {
    let mut request = json!({ "changes": restore_changes(record) });
    if force {
        request["force"] = json!(true);
    }
    if dry_run {
        request["dryRun"] = json!(true);
    }
    request
}

pub async fn undo(ctx: &Context, steps: usize, dry_run: bool, force: bool) -> Result<Value, Error> {
    let catalog = catalog_path(ctx).await?;
    let records = history::load(&ctx.paths)?;
    let all = history::latest_for_catalog(&records, &catalog, usize::MAX);
    if all.is_empty() {
        return Err(Error::failed(
            "nothing_to_undo",
            "No recorded changes to undo for the open catalog. `lrcli history` lists what was recorded.",
        ));
    }
    let picked = &all[..steps.min(all.len())];
    if dry_run {
        let mut would: Vec<Value> = picked.iter().map(describe).collect();
        // Later steps can only be checked once the newer ones are undone.
        let reply = ctx
            .connect()
            .await?
            .request("restore", restore_request(&picked[0], force, true))
            .await?;
        would[0]["photos"] = reply["photos"].clone();
        return Ok(json!({ "ok": true, "dryRun": true, "undo": would }));
    }
    let mut undone = Vec::new();
    for record in picked {
        let mut reply = ctx
            .connect()
            .await?
            .request("restore", restore_request(record, force, false))
            .await
            .map_err(|e| after_earlier_steps(e, undone.len()))?;
        if let Some(error) = conflicts_error(&reply["photos"], record, undone.len()) {
            return Err(error);
        }
        warn_uncleared(record, &mut reply);
        let mut item = describe(record);
        item["photos"] = reply["photos"].clone();
        history::remove(&ctx.paths, std::slice::from_ref(record))?;
        undone.push(item);
    }
    Ok(json!({ "ok": true, "dryRun": false, "undone": undone }))
}

pub fn history(ctx: &Context, limit: usize) -> Result<Value, Error> {
    let records = history::load(&ctx.paths)?;
    let entries: Vec<&Value> = records.iter().rev().take(limit).map(|r| &r.entry).collect();
    Ok(json!({ "ok": true, "entries": entries }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_are_told_from_names() {
        assert!(looks_like_uuid("1C2D3E4F-0123-4567-89AB-CDEF01234567"));
        assert!(!looks_like_uuid("Warm Contrast"));
        assert!(!looks_like_uuid("1C2D3E4F-0123-4567-89AB"));
    }

    #[test]
    fn clamping_is_told_from_rounding() {
        assert!(differs_from_requested(&json!(100), &json!(150)));
        assert!(!differs_from_requested(&json!(1.5), &json!(1.5000001)));
        assert!(differs_from_requested(&json!("Auto"), &json!("Custom")));
    }

    #[test]
    fn develop_history_keeps_changed_keys_with_before_and_after_values() {
        let reply = json!({ "photos": [
            { "id": 1, "before": { "A": 1, "B": 2, "C": null }, "after": { "A": 1, "B": 3, "C": 4 } },
            { "id": 2, "error": "video" },
        ]});
        assert_eq!(
            develop_changes(&reply),
            vec![json!({
                "id": 1,
                "develop": { "B": 2, "C": null },
                "developAfter": { "B": 3, "C": 4 },
            })]
        );
    }

    fn rating_record(change: Value) -> Recorded {
        Recorded {
            raw: String::new(),
            entry: json!({ "changes": [change] }),
        }
    }

    #[test]
    fn restore_changes_carry_before_values_and_expected_after_values() {
        let record = rating_record(json!({
            "id": 1,
            "develop": { "B": 2, "C": null },
            "developAfter": { "B": 3, "C": 4 },
            "rating": 2, "ratingAfter": 4,
            "keywordsAdded": ["K"], "keywordsRemoved": ["J"],
        }));
        assert_eq!(
            restore_changes(&record)[0],
            json!({
                "id": 1, "develop": { "B": 2, "C": null }, "rating": 2,
                "keywordsAdded": ["K"], "keywordsRemoved": ["J"],
                "expectAfter": {
                    "develop": { "B": 3, "C": 4 }, "rating": 4,
                    "keywordsPresent": ["K"], "keywordsAbsent": ["J"],
                },
            })
        );
        let old = rating_record(json!({ "id": 1, "rating": 2 }));
        assert_eq!(restore_changes(&old)[0], json!({ "id": 1, "rating": 2 }));
    }

    #[test]
    fn conflict_lines_list_the_differences() {
        let photo = json!({ "id": 1, "filename": "a.cr3", "conflict": ["A is 0.6, lrcli set 0.5", "rating is 2, lrcli set 3"] });
        assert_eq!(
            conflict_line(&photo).unwrap(),
            "a.cr3 (id 1): skipped: changed since lrcli edited it (A is 0.6, lrcli set 0.5; rating is 2, lrcli set 3). Re-run with --force to restore anyway."
        );
        assert!(conflict_line(&json!({ "id": 1, "conflict": [] })).is_none());
    }

    #[test]
    fn scalar_history_skips_unchanged_photos() {
        let reply = json!({ "photos": [
            { "id": 1, "before": 2, "after": 5 }, { "id": 2, "before": 3, "after": 3 },
        ]});
        assert_eq!(
            scalar_changes(&reply, "rating"),
            vec![json!({ "id": 1, "rating": 2, "ratingAfter": 5 })]
        );
    }

    #[test]
    fn keyword_history_skips_empty_changes() {
        let reply = json!({ "photos": [
            { "id": 1, "added": ["A"], "removed": [] }, { "id": 2, "added": [], "removed": [] },
        ]});
        assert_eq!(
            keyword_changes(&reply),
            vec![json!({ "id": 1, "keywordsAdded": ["A"], "keywordsRemoved": [] })]
        );
    }

    #[test]
    fn requested_values_resolve_relative_settings() {
        let mut reply = json!({ "photos": [{ "id": 1, "before": { "A": 1.0 } }] });
        annotate_requested(
            &mut reply,
            &[
                Setting::Add("A".into(), 0.5),
                Setting::Set("B".into(), json!(3)),
            ],
        );
        assert_eq!(reply["photos"][0]["requested"], json!({ "A": 1.5, "B": 3 }));
    }
}
