mod common;

use std::path::Path;

use common::{FakePlugin, Options, run, start_broker};
use serde_json::{Value, json};

const CATALOG: &str = "/fake/catalog.lrcat";

fn history(plugin: &FakePlugin) -> Vec<Value> {
    history_at(plugin.home())
}

fn history_at(home: &Path) -> Vec<Value> {
    std::fs::read_to_string(home.join("history.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn requests(plugin: &FakePlugin, cmd: &str) -> usize {
    plugin
        .stats
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|(c, _)| c == cmd)
        .count()
}

fn apply_reply() -> Value {
    json!({ "target": "ids", "photos": [
        { "id": 149, "filename": "a.cr3",
          "before": { "Exposure2012": 1, "Contrast2012": 3 },
          "after": { "Exposure2012": 1.5, "Contrast2012": 10 } },
        { "id": 150, "filename": "b.mov", "error": "Videos have no develop settings to edit." },
    ]})
}

#[tokio::test]
async fn apply_sends_set_and_add_and_records_before_values() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("apply", apply_reply());
    let out = run(
        plugin.home(),
        &[
            "apply",
            "--photo",
            "149",
            "--photo",
            "150",
            "Exposure2012+=0.5",
            "Contrast2012=15",
            "WhiteBalance=Auto",
            "Highlights2012=-20",
            "Shadows2012-=5",
        ],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        plugin.last_args("apply"),
        json!({
            "photos": [149, 150],
            "set": { "Contrast2012": 15, "WhiteBalance": "Auto", "Highlights2012": -20 },
            "add": { "Exposure2012": 0.5, "Shadows2012": -5 },
        })
    );
    assert!(out.stdout.contains("a.cr3 (id 149)"), "{}", out.stdout);
    assert!(
        out.stdout.contains("Exposure2012: 1 → 1.5"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("Contrast2012: 3 → 10 (requested 15)"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("b.mov (id 150)  error:"),
        "{}",
        out.stdout
    );

    let entries = history(&plugin);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["catalog"], CATALOG);
    assert!(
        entries[0]["command"]
            .as_str()
            .unwrap()
            .starts_with("lrcli apply --photo 149 --photo 150 Exposure2012+=0.5")
    );
    assert_eq!(
        entries[0]["changes"],
        json!([{
            "id": 149,
            "develop": { "Exposure2012": 1, "Contrast2012": 3 },
            "developAfter": { "Exposure2012": 1.5, "Contrast2012": 10 },
        }])
    );
}

#[tokio::test]
async fn apply_without_photos_targets_the_selection_and_json_includes_requested() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("apply", apply_reply());
    let out = run(plugin.home(), &["--json", "apply", "Exposure2012-=0.25"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        plugin.last_args("apply"),
        json!({ "set": {}, "add": { "Exposure2012": -0.25 } })
    );
    let reply = out.json();
    assert_eq!(
        reply["photos"][0]["requested"],
        json!({ "Exposure2012": 0.75 })
    );
}

#[tokio::test]
async fn malformed_settings_are_usage_errors() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    for bad in ["Exposure2012", "=5", "Key+=abc"] {
        let out = run(plugin.home(), &["apply", "--photo", "1", bad]).await;
        assert_eq!(out.code, 2, "{bad}: {}", out.stderr);
    }
    let out = run(plugin.home(), &["apply", "--photo", "1"]).await;
    assert_eq!(out.code, 2);
    let out = run(
        plugin.home(),
        &["apply", "--photo", "1", "Exposure2012=1", "Exposure2012+=1"],
    )
    .await;
    assert_eq!(out.code, 2, "{}", out.stderr);
    assert_eq!(requests(&plugin, "apply"), 0);
}

#[tokio::test]
async fn plugin_errors_pass_through_and_nothing_is_recorded() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned(
        "apply",
        json!({ "error": "bad_request", "message": "Unknown develop setting \"Exposur\"." }),
    );
    let out = run(plugin.home(), &["apply", "--photo", "1", "Exposur=1"]).await;
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains("Unknown develop setting"),
        "{}",
        out.stderr
    );
    let json_out = run(
        plugin.home(),
        &["--json", "apply", "--photo", "1", "Exposur=1"],
    )
    .await;
    assert_eq!(json_out.json()["error"], "bad_request");
    assert!(history(&plugin).is_empty());
}

#[tokio::test]
async fn a_write_that_failed_for_every_photo_exits_1_without_history() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned(
        "apply",
        json!({ "photos": [{ "id": 1, "filename": "v.mov", "error": "Videos have no develop settings to edit." }] }),
    );
    let out = run(plugin.home(), &["apply", "--photo", "1", "Tint=5"]).await;
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("Videos"), "{}", out.stderr);
    assert!(history(&plugin).is_empty());
}

#[tokio::test]
async fn apply_without_changes_records_nothing() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned(
        "apply",
        json!({ "photos": [{ "id": 1, "filename": "a", "before": { "Tint": 5 }, "after": { "Tint": 5 } }] }),
    );
    let out = run(plugin.home(), &["apply", "--photo", "1", "Tint=5"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(history(&plugin).is_empty());
}

#[tokio::test]
async fn apply_preview_chains_a_preview_of_the_edited_photos() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned(
        "apply",
        json!({ "photos": [
            { "id": 149, "filename": "a.cr3", "before": { "Tint": 1 }, "after": { "Tint": 2 } },
            { "id": 150, "filename": "v.mov", "error": "Videos have no develop settings to edit." },
        ]}),
    );
    plugin.canned(
        "preview",
        json!({ "target": "ids", "mode": "export", "size": 1600, "photos": [
            { "id": 149, "filename": "a.cr3", "file": "{dir}/lrcli-a.jpg", "error": null },
        ]}),
    );
    let out = run(
        plugin.home(),
        &[
            "--json",
            "apply",
            "--photo",
            "149",
            "--photo",
            "150",
            "Tint=2",
            "--preview",
        ],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    let args = plugin.last_args("preview");
    assert_eq!(args["photos"], json!([149]));
    assert_eq!(args["size"], 1600);
    assert_eq!(args["mode"], "export");
    let reply = out.json();
    assert_eq!(reply["preview"]["photos"][0]["id"], 149);
    assert!(reply["preview"]["photos"][0]["stats"].is_object());

    let human = run(
        plugin.home(),
        &["apply", "--photo", "149", "Tint=2", "--preview"],
    )
    .await;
    assert!(human.stdout.contains("lrcli-a.jpg"), "{}", human.stdout);
}

#[tokio::test]
async fn apply_without_preview_flag_does_not_render() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("apply", apply_reply());
    run(plugin.home(), &["apply", "--photo", "149", "Tint=2"]).await;
    assert_eq!(requests(&plugin, "preview"), 0);
}

#[tokio::test]
async fn preview_failure_after_a_write_is_reported_not_fatal() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("apply", apply_reply());
    plugin.canned(
        "preview",
        json!({ "photos": [{ "id": 149, "filename": "a.cr3", "file": null, "error": "render failed" }] }),
    );
    let out = run(
        plugin.home(),
        &["--json", "apply", "--photo", "149", "Tint=2", "--preview"],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.json()["previewError"]["message"], "render failed");
    assert_eq!(history(&plugin).len(), 1);
}

#[tokio::test]
async fn presets_lists_folders() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned(
        "presets",
        json!({ "folders": [{ "name": "Color", "presets": [{ "name": "Vivid", "uuid": "U1" }] }] }),
    );
    let out = run(plugin.home(), &["presets"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, "Color\n  Vivid\n");
    assert_eq!(plugin.last_args("presets"), json!({}));
}

#[tokio::test]
async fn preset_sends_name_or_uuid_and_amount() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned(
        "preset",
        json!({ "preset": { "name": "Vivid", "uuid": "U", "folder": "Color" }, "photos": [
            { "id": 149, "filename": "a.cr3", "before": { "Vibrance": 0 }, "after": { "Vibrance": 30 } },
        ]}),
    );
    let out = run(
        plugin.home(),
        &["preset", "Vivid", "--amount", "60", "--photo", "149"],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        plugin.last_args("preset"),
        json!({ "photos": [149], "name": "Vivid", "amount": 60 })
    );
    assert!(
        out.stdout.starts_with("Preset Color/Vivid"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("Vibrance: 0 → 30"), "{}", out.stdout);

    let uuid = "1C2D3E4F-0123-4567-89AB-CDEF01234567";
    run(plugin.home(), &["preset", uuid]).await;
    assert_eq!(plugin.last_args("preset"), json!({ "uuid": uuid }));
    assert_eq!(history(&plugin).len(), 2);
    assert_eq!(
        history(&plugin)[0]["changes"],
        json!([{ "id": 149, "develop": { "Vibrance": 0 }, "developAfter": { "Vibrance": 30 } }])
    );

    let out = run(plugin.home(), &["preset", "Vivid", "--amount", "201"]).await;
    assert_eq!(out.code, 2);
}

#[tokio::test]
async fn tag_sends_keywords_and_records_changes() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned(
        "tag",
        json!({ "photos": [
            { "id": 149, "filename": "a.cr3", "added": ["Places > Paris"], "removed": [] },
            { "id": 150, "filename": "b.cr3", "added": [], "removed": [] },
        ]}),
    );
    let out = run(
        plugin.home(),
        &[
            "tag",
            "add",
            "Places > Paris",
            "studio",
            "--photo",
            "149",
            "--photo",
            "150",
        ],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        plugin.last_args("tag"),
        json!({ "photos": [149, 150], "add": ["Places > Paris", "studio"] })
    );
    assert!(
        out.stdout.contains("a.cr3 (id 149): +Places > Paris"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("b.cr3 (id 150): no change"),
        "{}",
        out.stdout
    );
    assert_eq!(
        history(&plugin)[0]["changes"],
        json!([{ "id": 149, "keywordsAdded": ["Places > Paris"], "keywordsRemoved": [] }])
    );
    assert!(
        history(&plugin)[0]["command"]
            .as_str()
            .unwrap()
            .contains("'Places > Paris'")
    );

    run(plugin.home(), &["tag", "remove", "studio"]).await;
    assert_eq!(plugin.last_args("tag"), json!({ "remove": ["studio"] }));
    assert_eq!(run(plugin.home(), &["tag", "add"]).await.code, 2);
}

#[tokio::test]
async fn rate_flag_and_label_send_values_and_record_only_changes() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned(
        "rate",
        json!({ "photos": [
            { "id": 1, "filename": "a", "before": 2, "after": 4 },
            { "id": 2, "filename": "b", "before": 4, "after": 4 },
        ]}),
    );
    plugin.canned(
        "flag",
        json!({ "photos": [{ "id": 1, "filename": "a", "before": "none", "after": "picked" }] }),
    );
    plugin.canned(
        "label",
        json!({ "photos": [{ "id": 1, "filename": "a", "before": "red", "after": "none" }] }),
    );
    let out = run(
        plugin.home(),
        &["rate", "4", "--photo", "1", "--photo", "2"],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        plugin.last_args("rate"),
        json!({ "photos": [1, 2], "rating": 4 })
    );
    assert!(out.stdout.contains("a (id 1): 2 → 4"), "{}", out.stdout);
    assert!(out.stdout.contains("b (id 2): already 4"), "{}", out.stdout);

    run(plugin.home(), &["flag", "picked", "--photo", "1"]).await;
    assert_eq!(
        plugin.last_args("flag"),
        json!({ "photos": [1], "flag": "picked" })
    );
    run(plugin.home(), &["label", "none", "--photo", "1"]).await;
    assert_eq!(
        plugin.last_args("label"),
        json!({ "photos": [1], "label": "none" })
    );

    let entries = history(&plugin);
    assert_eq!(
        entries[0]["changes"],
        json!([{ "id": 1, "rating": 2, "ratingAfter": 4 }])
    );
    assert_eq!(
        entries[1]["changes"],
        json!([{ "id": 1, "flag": "none", "flagAfter": "picked" }])
    );
    assert_eq!(
        entries[2]["changes"],
        json!([{ "id": 1, "label": "red", "labelAfter": "none" }])
    );

    assert_eq!(run(plugin.home(), &["rate", "6"]).await.code, 2);
    assert_eq!(run(plugin.home(), &["flag", "unflagged"]).await.code, 2);
    assert_eq!(run(plugin.home(), &["label", "pink"]).await.code, 2);
}

fn no_native() -> Value {
    json!({ "native": false, "reason": "test" })
}

fn restore_reply() -> Value {
    json!({ "photos": [{ "id": 1, "filename": "a", "restored": ["rating"], "warnings": [] }] })
}

#[tokio::test]
async fn undo_restores_recorded_values_and_drops_the_entry() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned(
        "rate",
        json!({ "photos": [{ "id": 1, "filename": "a", "before": 2, "after": 4 }] }),
    );
    plugin.canned("restore", restore_reply());
    plugin.canned("undo_native", no_native());
    run(plugin.home(), &["rate", "4", "--photo", "1"]).await;

    let dry = run(plugin.home(), &["undo", "--dry-run"]).await;
    assert_eq!(dry.code, 0, "{}", dry.stderr);
    assert!(
        dry.stdout.contains("Would undo: lrcli rate 4 --photo 1"),
        "{}",
        dry.stdout
    );
    assert!(dry.stdout.contains("photo 1: rating"), "{}", dry.stdout);
    assert_eq!(requests(&plugin, "restore"), 0);
    assert_eq!(history(&plugin).len(), 1);

    let out = run(plugin.home(), &["undo"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        plugin.last_args("restore"),
        json!({ "changes": [{ "id": 1, "rating": 2 }] })
    );
    assert!(
        out.stdout.contains("Undid: lrcli rate 4 --photo 1"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("a (id 1): restored rating"),
        "{}",
        out.stdout
    );
    assert!(history(&plugin).is_empty());

    let again = run(plugin.home(), &["undo"]).await;
    assert_eq!(again.code, 1);
    assert!(
        again.stderr.contains("No recorded changes"),
        "{}",
        again.stderr
    );
}

#[tokio::test]
async fn undo_steps_run_newest_first_and_skip_other_catalogs() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let lines = [
        json!({ "time": "t1", "command": "first", "catalog": CATALOG, "changes": [{ "id": 1, "rating": 1 }] }),
        json!({ "time": "t2", "command": "other", "catalog": "/other.lrcat", "changes": [{ "id": 9, "rating": 9 }] }),
        json!({ "time": "t3", "command": "second", "catalog": CATALOG, "changes": [{ "id": 1, "rating": 2 }] }),
        json!({ "time": "t4", "command": "third", "catalog": CATALOG, "changes": [{ "id": 1, "rating": 3 }] }),
    ];
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(plugin.home().join("history.jsonl"), text).unwrap();
    plugin.canned("restore", restore_reply());
    plugin.canned("undo_native", no_native());

    let out = run(plugin.home(), &["--json", "undo", "--steps", "2"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    let reply = out.json();
    let commands: Vec<&str> = reply["undone"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["command"].as_str().unwrap())
        .collect();
    assert_eq!(commands, ["third", "second"]);
    assert_eq!(requests(&plugin, "restore"), 2);
    assert_eq!(
        plugin.last_args("restore"),
        json!({ "changes": [{ "id": 1, "rating": 2 }] })
    );
    let left: Vec<String> = history(&plugin)
        .iter()
        .map(|e| e["command"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(left, ["first", "other"]);
}

#[tokio::test]
async fn undo_failure_keeps_the_entry() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let line = json!({ "time": "t", "command": "c", "catalog": CATALOG, "changes": [{ "id": 1, "rating": 1 }] });
    std::fs::write(plugin.home().join("history.jsonl"), format!("{line}\n")).unwrap();
    plugin.canned("undo_native", no_native());
    plugin.canned(
        "restore",
        json!({ "error": "write_busy", "message": "Lightroom couldn't get write access" }),
    );
    let out = run(plugin.home(), &["undo"]).await;
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("write access"), "{}", out.stderr);
    assert_eq!(history(&plugin).len(), 1);
}

#[tokio::test]
async fn native_undo_success_pops_the_entry_without_restoring() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let older = json!({ "time": "t1", "command": "older", "catalog": CATALOG, "changes": [
        { "id": 1, "rating": 1, "ratingAfter": 2, "keywordsAdded": ["K"], "keywordsRemoved": [] }] });
    let newer = json!({ "time": "t2", "command": "newer", "catalog": CATALOG, "changes": [
        { "id": 1, "develop": { "A": 1, "B": null }, "developAfter": { "A": 2, "B": 3 },
          "rating": 2, "ratingAfter": 4, "keywordsAdded": [], "keywordsRemoved": ["J"] },
        { "id": 2, "flag": "none" }] });
    std::fs::write(
        plugin.home().join("history.jsonl"),
        format!("{older}\n{newer}\n"),
    )
    .unwrap();
    plugin.canned("undo_native", json!({ "native": true }));

    let out = run(plugin.home(), &["undo"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        plugin.last_args("undo_native"),
        json!({
            "expect": [
                { "id": 1, "develop": { "A": 1, "B": null }, "rating": 2, "keywordsPresent": ["J"] },
                { "id": 2, "flag": "none" },
            ],
            "keep": [
                { "id": 1, "rating": 2, "keywordsPresent": ["K"] },
            ],
        })
    );
    assert_eq!(requests(&plugin, "restore"), 0);
    assert!(
        out.stdout.contains("undone with Lightroom's undo"),
        "{}",
        out.stdout
    );
    let left = history(&plugin);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0]["command"], "older");
}

#[tokio::test]
async fn failed_native_undo_falls_back_to_restore_and_warns_about_uncleared_keys() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let line = json!({ "time": "t", "command": "c", "catalog": CATALOG, "changes": [
        { "id": 1, "develop": { "A": 1, "Temperature": null }, "developAfter": { "A": 2, "Temperature": 4000 } }] });
    std::fs::write(plugin.home().join("history.jsonl"), format!("{line}\n")).unwrap();
    plugin.canned(
        "undo_native",
        json!({ "native": false, "reason": "photo 1: A is 2, expected 1" }),
    );
    plugin.canned(
        "restore",
        json!({ "photos": [{ "id": 1, "filename": "a", "restored": ["develop"], "warnings": [] }] }),
    );
    let out = run(plugin.home(), &["undo"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        plugin.last_args("restore"),
        json!({ "changes": [{ "id": 1, "develop": { "A": 1, "Temperature": null } }] })
    );
    assert!(out.stdout.contains("restored values"), "{}", out.stdout);
    assert!(
        out.stdout
            .contains("warning: Temperature had no value before"),
        "{}",
        out.stdout
    );
    assert!(history(&plugin).is_empty());
}

#[tokio::test]
async fn dry_run_undo_sends_nothing_to_lightroom_beyond_status() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let line = json!({ "time": "t", "command": "c", "catalog": CATALOG, "changes": [{ "id": 1, "rating": 1, "ratingAfter": 2 }] });
    std::fs::write(plugin.home().join("history.jsonl"), format!("{line}\n")).unwrap();
    let out = run(plugin.home(), &["undo", "--dry-run"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(requests(&plugin, "undo_native"), 0);
    assert_eq!(requests(&plugin, "restore"), 0);
}

#[tokio::test]
async fn history_lists_newest_first_without_lightroom() {
    let home = tempfile::TempDir::new().unwrap();
    let lines = [
        json!({ "time": "t1", "command": "lrcli rate 1", "catalog": "/c", "changes": [{ "id": 1, "rating": 0 }] }),
        json!({ "time": "t2", "command": "lrcli rate 2", "catalog": "/c", "changes": [{ "id": 1, "rating": 1 }, { "id": 2, "rating": 1 }] }),
    ];
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(home.path().join("history.jsonl"), text).unwrap();
    let out = run(home.path(), &["history"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        out.stdout,
        "t2  lrcli rate 2  (2 photos)\nt1  lrcli rate 1  (1 photo)\n"
    );
    let limited = run(home.path(), &["--json", "history", "--limit", "1"]).await;
    assert_eq!(limited.json()["entries"].as_array().unwrap().len(), 1);
    let empty = tempfile::TempDir::new().unwrap();
    assert_eq!(
        run(empty.path(), &["history"]).await.stdout,
        "No recorded changes.\n"
    );
}

fn write_and_preview_order(plugin: &FakePlugin) -> Vec<String> {
    plugin
        .stats
        .commands
        .lock()
        .unwrap()
        .iter()
        .filter(|c| matches!(c.as_str(), "preview" | "apply" | "preset"))
        .cloned()
        .collect()
}

fn compare_fixtures(plugin: &FakePlugin) {
    plugin.canned(
        "apply",
        json!({ "photos": [
            { "id": 149, "filename": "a.cr3", "before": { "Tint": 1 }, "after": { "Tint": 2 } },
        ]}),
    );
    plugin.canned(
        "preview",
        json!({ "target": "ids", "mode": "export", "size": 1600, "photos": [
            { "id": 149, "filename": "a.cr3", "file": "{dir}/lrcli-a.jpg", "error": null },
        ]}),
    );
}

#[tokio::test]
async fn apply_compare_renders_before_and_after_around_the_write() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    compare_fixtures(&plugin);
    let out = run(
        plugin.home(),
        &["--json", "apply", "--photo", "149", "Tint=2", "--compare"],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        write_and_preview_order(&plugin),
        ["preview", "apply", "preview"]
    );
    let compare = &out.json()["photos"][0]["compare"];
    let file = compare["file"].as_str().unwrap();
    assert!(file.ends_with("lrcli-a-compare.jpg"), "{file}");
    assert_ne!(compare["before"]["file"], compare["after"]["file"]);
    let composite = image::open(file).unwrap();
    assert_eq!((composite.width(), composite.height()), (10 + 8 + 10, 6));
    assert_eq!(compare["before"]["stats"]["width"], 10);
    assert!(compare["after"]["stats"]["meanLuminance"].is_number());

    let human = run(
        plugin.home(),
        &["apply", "--photo", "149", "Tint=2", "--compare"],
    )
    .await;
    assert!(
        human.stdout.contains("compare: ") && human.stdout.contains("mean 0.50 → 0.50"),
        "{}",
        human.stdout
    );
}

#[tokio::test]
async fn compare_wins_over_preview_when_both_are_given() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    compare_fixtures(&plugin);
    let out = run(
        plugin.home(),
        &[
            "--json",
            "apply",
            "--photo",
            "149",
            "Tint=2",
            "--preview",
            "--compare",
        ],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(requests(&plugin, "preview"), 2);
    assert!(out.json()["photos"][0]["compare"].is_object());
}

#[tokio::test]
async fn preset_compare_renders_around_the_write() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    compare_fixtures(&plugin);
    plugin.canned(
        "preset",
        json!({ "preset": { "name": "Vivid", "uuid": "u", "folder": "Color" }, "photos": [
            { "id": 149, "filename": "a.cr3", "before": { "Tint": 1 }, "after": { "Tint": 2 } },
        ]}),
    );
    let out = run(
        plugin.home(),
        &["--json", "preset", "Vivid", "--photo", "149", "--compare"],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        write_and_preview_order(&plugin),
        ["preview", "preset", "preview"]
    );
    assert!(out.json()["photos"][0]["compare"]["file"].is_string());
}

#[tokio::test]
async fn failed_write_skips_the_after_render() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    compare_fixtures(&plugin);
    plugin.canned(
        "apply",
        json!({ "photos": [{ "id": 149, "filename": "a.cr3", "error": "nope" }] }),
    );
    let out = run(
        plugin.home(),
        &["apply", "--photo", "149", "Tint=2", "--compare"],
    )
    .await;
    assert_eq!(out.code, 1);
    assert_eq!(write_and_preview_order(&plugin), ["preview", "apply"]);
}

#[tokio::test]
async fn compare_failure_after_a_write_is_reported_not_fatal() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    compare_fixtures(&plugin);
    plugin.canned(
        "preview",
        json!({ "photos": [{ "id": 149, "filename": "a.cr3", "file": null, "error": "render failed" }] }),
    );
    let out = run(
        plugin.home(),
        &["--json", "apply", "--photo", "149", "Tint=2", "--compare"],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    let reply = out.json();
    assert_eq!(reply["compareError"]["message"], "render failed");
    assert_eq!(requests(&plugin, "preview"), 1);
    assert_eq!(history(&plugin).len(), 1);
}
