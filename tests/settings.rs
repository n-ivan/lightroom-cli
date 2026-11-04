mod common;

use common::{Options, run, start_broker, write_grey_jpeg};
use serde_json::{Value, json};

fn settings_reply() -> Value {
    json!({
        "photo": 149,
        "fileType": "raw",
        "panels": [
            { "id": "adjustPanel", "title": "Basic", "keys": [
                { "key": "Temperature", "min": 2000, "max": 50000, "value": 5500 },
                { "key": "Exposure2012", "min": -5, "max": 5, "value": 0.5 },
            ]},
            { "id": "crop", "title": "Crop", "keys": [
                { "key": "CropLeft", "min": 0, "max": 1 },
            ]},
        ],
    })
}

#[tokio::test]
async fn settings_sends_panel_and_photo_and_prints_aligned_rows() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("settings", settings_reply());
    let out = run(
        plugin.home(),
        &["settings", "--panel", "basic", "--photo", "149"],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    let args = plugin.last_args("settings");
    assert_eq!(args, json!({ "photos": [149], "panel": "basic" }));
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(lines[0], "Photo 149 (raw file)");
    assert_eq!(lines[2], "Basic");
    assert_eq!(lines[3], "  Temperature   2000..50000  [5500]");
    assert_eq!(lines[4], "  Exposure2012  -5..5        [0.5]");
    assert_eq!(lines[6], "Crop");
    assert_eq!(lines[7], "  CropLeft      0..1");
}

#[tokio::test]
async fn settings_without_arguments_sends_none_and_notes_non_raw_ranges() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned(
        "settings",
        json!({ "fileType": null, "panels": [{ "id": "adjustPanel", "title": "Basic", "keys": [
            { "key": "Temperature", "min": 2000, "max": 50000, "nonRaw": { "min": -100, "max": 100 } },
            { "key": "IncrementalTint", "min": -100, "max": 100, "nonRawOnly": true },
        ]}]}),
    );
    let out = run(plugin.home(), &["settings"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(plugin.last_args("settings"), json!({}));
    assert!(
        out.stdout.contains("(non-raw -100..100)") && out.stdout.contains("(non-raw only)"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("Photo"));
}

#[tokio::test]
async fn settings_json_passes_the_reply_through() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("settings", settings_reply());
    let out = run(plugin.home(), &["--json", "settings"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    let reply = out.json();
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["fileType"], "raw");
    assert_eq!(reply["panels"][0]["keys"][1]["key"], "Exposure2012");
}

#[tokio::test]
async fn settings_panel_errors_come_from_the_plugin() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned(
        "settings",
        json!({ "error": "bad_request", "message": "Unknown panel \"zzz\". Panels: adjustPanel (Basic)." }),
    );
    let out = run(plugin.home(), &["settings", "--panel", "zzz"]).await;
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("Unknown panel"), "{}", out.stderr);
    let json_out = run(plugin.home(), &["--json", "settings", "--panel", "zzz"]).await;
    assert_eq!(json_out.json()["error"], "bad_request");
}

fn gradient(path: &std::path::Path, width: u32, height: u32) {
    let image = image::RgbImage::from_fn(width, height, |x, _| image::Rgb([(x % 256) as u8; 3]));
    image
        .save_with_format(path, image::ImageFormat::Jpeg)
        .unwrap();
}

#[tokio::test]
async fn compare_writes_a_side_by_side_jpeg_next_to_after() {
    let temp = tempfile::TempDir::new().unwrap();
    let (before, after) = (temp.path().join("b.jpg"), temp.path().join("a.jpg"));
    write_grey_jpeg(&before);
    gradient(&after, 30, 12);
    let out = run(
        temp.path(),
        &[
            "--json",
            "compare",
            before.to_str().unwrap(),
            after.to_str().unwrap(),
        ],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    let reply = out.json();
    assert_eq!(reply["ok"], true);
    let file = temp.path().join("a-compare.jpg");
    assert_eq!(reply["file"], file.to_str().unwrap());
    // 10x6 and 30x12 meet at height 6: widths 10 and 15.
    let composite = image::open(&file).unwrap();
    assert_eq!((composite.width(), composite.height()), (10 + 8 + 15, 6));
    assert_eq!(reply["before"]["stats"]["width"], 10);
    assert_eq!(reply["after"]["stats"]["width"], 30);

    let human = run(
        temp.path(),
        &[
            "compare",
            before.to_str().unwrap(),
            after.to_str().unwrap(),
            "--out",
            temp.path().join("x.jpg").to_str().unwrap(),
        ],
    )
    .await;
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(
        human
            .stdout
            .starts_with(temp.path().join("x.jpg").to_str().unwrap())
    );
    assert!(temp.path().join("x.jpg").exists());
}

#[tokio::test]
async fn compare_reports_missing_and_undecodable_files() {
    let temp = tempfile::TempDir::new().unwrap();
    let missing = temp.path().join("nope.jpg");
    let junk = temp.path().join("junk.jpg");
    std::fs::write(&junk, "not a jpeg").unwrap();
    let (m, j) = (missing.to_str().unwrap(), junk.to_str().unwrap());
    let out = run(temp.path(), &["--json", "compare", m, j]).await;
    assert_eq!(out.code, 1);
    assert_eq!(out.json()["error"], "io_error");
    let out = run(temp.path(), &["--json", "compare", j, j]).await;
    assert_eq!(out.json()["error"], "compare_failed");
}
