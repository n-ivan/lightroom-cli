mod common;

use std::path::Path;

use common::{Options, run, run_stdin, start_broker};
use serde_json::{Value, json};

fn preview_reply(mode: &str) -> Value {
    let mut ok = json!({ "id": 149, "filename": "DSC08011-2.ARW", "file": "{dir}/lrcli-a.jpg", "error": null });
    let failed = json!({ "id": 150, "filename": "v.mov", "file": null, "error": "Videos cannot be previewed." });
    if mode == "thumbnail" {
        ok["possiblyStale"] = json!(true);
    }
    json!({ "target": "ids", "mode": mode, "size": 1600, "photos": [ok, failed] })
}

fn batch_dirs(home: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(home.join("previews"))
        .map(|entries| {
            entries
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[tokio::test]
async fn preview_sends_export_args_and_merges_stats() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("preview", preview_reply("export"));
    let out = run(
        plugin.home(),
        &[
            "--json", "preview", "--photo", "149", "--photo", "150", "--size", "800",
        ],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);

    let args = plugin.last_args("preview");
    assert_eq!(args["photos"], json!([149, 150]));
    assert_eq!(args["size"], 800);
    assert_eq!(args["mode"], "export");
    let dir = args["dir"].as_str().unwrap();
    assert!(Path::new(dir).is_absolute(), "{dir}");
    assert!(dir.starts_with(plugin.home().join("previews").to_str().unwrap()));

    let reply = out.json();
    assert_eq!(reply["dir"], dir);
    assert!(reply.get("id").is_none());
    let photos = reply["photos"].as_array().unwrap();
    assert_eq!(photos[0]["file"], format!("{dir}/lrcli-a.jpg"));
    let stats = &photos[0]["stats"];
    assert_eq!(
        (stats["width"].as_u64(), stats["height"].as_u64()),
        (Some(10), Some(6))
    );
    let mean = stats["meanLuminance"].as_f64().unwrap();
    assert!((mean - 0.5).abs() < 0.03, "{mean}");
    assert_eq!(stats["clipping"]["highlights"]["luma"], 0.0);
    assert!(stats.get("histograms").is_none());
    assert!(photos[1].get("stats").is_none());
}

#[tokio::test]
async fn preview_defaults_to_the_selection_and_size_1600() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("preview", preview_reply("export"));
    let out = run(plugin.home(), &["preview"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    let args = plugin.last_args("preview");
    assert!(args.get("photos").is_none());
    assert_eq!(args["size"], 1600);
}

#[tokio::test]
async fn preview_reads_ids_from_stdin() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("preview", preview_reply("export"));
    let out = run_stdin(plugin.home(), &["preview", "--photos", "-"], "149\n150\n").await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(plugin.last_args("preview")["photos"], json!([149, 150]));
}

#[tokio::test]
async fn preview_fast_uses_thumbnail_mode_and_says_so() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("preview", preview_reply("thumbnail"));
    let out = run(plugin.home(), &["preview", "--fast", "--photo", "149"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(plugin.last_args("preview")["mode"], "thumbnail");
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert!(
        lines[0].starts_with("DSC08011-2.ARW (149)  ")
            && lines[0].contains("/lrcli-a.jpg  10×6  mean 0.5"),
        "{}",
        out.stdout
    );
    assert_eq!(lines[1], "v.mov (150)  error: Videos cannot be previewed.");
    assert_eq!(lines[2], "(fast preview: may not reflect recent edits)");
}

#[tokio::test]
async fn preview_no_stats_and_histogram_flags() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("preview", preview_reply("export"));
    let out = run(
        plugin.home(),
        &["--json", "preview", "--no-stats", "--photo", "149"],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.json()["photos"][0].get("stats").is_none());

    let out = run(
        plugin.home(),
        &["--json", "preview", "--histogram", "--photo", "149"],
    )
    .await;
    let histograms = &out.json()["photos"][0]["stats"]["histograms"];
    assert_eq!(histograms["luma"].as_array().unwrap().len(), 256);
    assert_eq!(histograms["b"].as_array().unwrap().len(), 256);
}

#[tokio::test]
async fn preview_exits_1_with_the_first_error_when_every_photo_failed() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned(
        "preview",
        json!({ "target": "ids", "mode": "export", "size": 1600, "photos": [
            { "id": 1, "filename": "a", "file": null, "error": "render failed" },
            { "id": 2, "filename": "b", "file": null, "error": "other" } ] }),
    );
    let out = run(plugin.home(), &["preview", "--photo", "1", "--photo", "2"]).await;
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("render failed"), "{}", out.stderr);
}

#[tokio::test]
async fn preview_plugin_errors_exit_1() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned(
        "preview",
        json!({ "error": "photo_not_found", "message": "Photos not found in the catalog: 9." }),
    );
    let out = run(plugin.home(), &["preview", "--photo", "9"]).await;
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("not found"), "{}", out.stderr);
}

#[tokio::test]
async fn preview_validates_size_locally() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    assert_eq!(
        run(plugin.home(), &["preview", "--size", "10"]).await.code,
        2
    );
    assert_eq!(
        run(plugin.home(), &["preview", "--size", "5000"])
            .await
            .code,
        2
    );
}

#[tokio::test]
async fn preview_prunes_old_batches_but_keeps_the_new_one() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("preview", preview_reply("export"));
    for i in 0..25 {
        std::fs::create_dir_all(
            plugin
                .home()
                .join(format!("previews/20200101-0000{i:02}-1")),
        )
        .unwrap();
    }
    let out = run(plugin.home(), &["--json", "preview", "--photo", "149"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    let current = out.json()["dir"].as_str().unwrap().to_string();
    let names = batch_dirs(plugin.home());
    assert_eq!(names.len(), 20, "{names:?}");
    assert!(
        current.ends_with(names.last().unwrap()),
        "{current} {names:?}"
    );
    assert!(Path::new(&current).is_dir());
}
