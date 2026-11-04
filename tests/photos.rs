mod common;

use common::{Options, run, run_stdin, start_broker};
use serde_json::{Value, json};

fn photo(id: i64, name: &str) -> Value {
    json!({
        "id": id, "filename": name, "path": format!("/p/{name}"),
        "captureTime": "2024-05-31T10:00:00Z", "rating": 4, "flag": "picked",
        "label": "red", "width": 6000, "height": 4000,
    })
}

fn search_reply(total: u64, offset: u64) -> Value {
    json!({ "total": total, "offset": offset, "limit": 2,
        "photos": [photo(1, "a.cr3"), photo(2, "b.cr3")] })
}

#[tokio::test]
async fn search_maps_every_option_to_plugin_args() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("search", search_reply(2, 0));
    let out = run(
        plugin.home(),
        &[
            "search",
            "--rating",
            ">=3",
            "--flag",
            "picked",
            "--label",
            "red",
            "--keyword",
            "a",
            "--keyword",
            "b",
            "--text",
            "t",
            "--filename",
            "f",
            "--folder",
            "d",
            "--collection",
            "c",
            "--since",
            "2024-01-01",
            "--until",
            "2024-02-01",
            "--camera",
            "R5",
            "--limit",
            "2",
            "--offset",
            "4",
        ],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        plugin.last_args("search"),
        json!({
            "filters": {
                "rating": { "op": ">=", "value": 3 }, "flag": "picked", "label": "red",
                "keywords": ["a", "b"], "text": "t", "filename": "f", "folder": "d",
                "collection": "c", "since": "2024-01-01", "until": "2024-02-01", "camera": "R5",
            },
            "limit": 2, "offset": 4,
        })
    );
}

#[tokio::test]
async fn search_prints_a_table_with_footer_and_next_page_hint() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("search", search_reply(5, 0));
    let out = run(plugin.home(), &["search", "--rating", "4"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        plugin.last_args("search"),
        json!({ "filters": { "rating": { "op": "==", "value": 4 } } })
    );
    assert!(out.stdout.starts_with("ID  FILENAME"), "{}", out.stdout);
    assert!(out.stdout.contains("a.cr3     ★★★★☆"), "{}", out.stdout);
    assert!(
        out.stdout
            .trim_end()
            .ends_with("Showing 1–2 of 5. More: add --offset 2"),
        "{}",
        out.stdout
    );
}

#[tokio::test]
async fn search_ids_prints_ids_in_human_and_json_modes() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("search", search_reply(7, 0));
    let out = run(plugin.home(), &["search", "--ids"]).await;
    assert_eq!(out.stdout, "1\n2\n");
    let out = run(plugin.home(), &["--json", "search", "--ids"]).await;
    assert_eq!(out.json(), json!({ "ok": true, "ids": [1, 2], "total": 7 }));
}

#[tokio::test]
async fn search_json_passes_the_reply_through() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("search", search_reply(2, 0));
    let out = run(plugin.home(), &["--json", "search"]).await;
    let reply = out.json();
    assert_eq!(reply["total"], 2);
    assert_eq!(reply["photos"][1]["filename"], "b.cr3");
    assert_eq!(plugin.last_args("search"), json!({ "filters": {} }));
}

#[tokio::test]
async fn select_sends_ids_from_flags_and_reports_the_source_switch() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned(
        "select",
        json!({ "selected": [3, 4], "sourceChanged": true }),
    );
    let out = run(plugin.home(), &["select", "--photo", "3", "--photo", "4"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(plugin.last_args("select"), json!({ "photos": [3, 4] }));
    assert_eq!(
        out.stdout.trim(),
        "Selected 2 photos (switched source to All Photographs)"
    );
}

#[tokio::test]
async fn select_reads_ids_piped_from_stdin() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("select", json!({ "selected": [5], "sourceChanged": false }));
    let out = run_stdin(plugin.home(), &["select", "--photos", "-"], "5\n").await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout.trim(), "Selected 1 photo");
    assert_eq!(plugin.last_args("select"), json!({ "photos": [5] }));
}

#[tokio::test]
async fn selection_lists_photos_and_ids() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("selection", json!({ "photos": [photo(9, "z.cr3")] }));
    let out = run(plugin.home(), &["selection"]).await;
    assert!(out.stdout.contains("z.cr3"), "{}", out.stdout);
    assert!(!out.stdout.contains("Showing"));
    assert_eq!(plugin.last_args("selection"), json!({}));
    let out = run(plugin.home(), &["selection", "--ids"]).await;
    assert_eq!(out.stdout, "9\n");

    plugin.canned("selection", json!({ "photos": [] }));
    let out = run(plugin.home(), &["selection"]).await;
    assert_eq!(out.code, 0);
    assert_eq!(out.stdout.trim(), "Nothing is selected in Lightroom.");
}

fn info_reply() -> Value {
    let mut p = photo(8, "i.cr3");
    p["title"] = json!("Title");
    p["keywords"] = json!(["k1", "k2"]);
    p["exif"] = json!({ "camera": "R5", "iso": 100 });
    p["develop"] = json!({ "Exposure2012": 0.5 });
    json!({ "target": "ids", "photos": [p] })
}

#[tokio::test]
async fn info_human_omits_develop_unless_asked() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("info", info_reply());
    let out = run(plugin.home(), &["info", "--photo", "8"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        plugin.last_args("info"),
        json!({ "photos": [8], "develop": false })
    );
    assert!(out.stdout.contains("Keywords: k1, k2"), "{}", out.stdout);

    run(plugin.home(), &["info", "--photo", "8", "--develop"]).await;
    assert_eq!(
        plugin.last_args("info"),
        json!({ "photos": [8], "develop": true })
    );
}

#[tokio::test]
async fn info_without_ids_targets_the_selection_and_json_requests_develop() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("info", info_reply());
    let out = run(plugin.home(), &["--json", "info"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(plugin.last_args("info"), json!({ "develop": true }));
    assert_eq!(out.json()["photos"][0]["develop"]["Exposure2012"], 0.5);
}

#[tokio::test]
async fn info_accepts_search_json_on_stdin() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("info", info_reply());
    let piped = json!({ "ok": true, "total": 2, "photos": [photo(1, "a"), photo(2, "b")] });
    let out = run_stdin(
        plugin.home(),
        &["info", "--photos", "-"],
        &piped.to_string(),
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        plugin.last_args("info"),
        json!({ "photos": [1, 2], "develop": false })
    );
}

#[tokio::test]
async fn plugin_errors_pass_through_with_exit_1() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    for (cmd, args, code, message) in [
        (
            "info",
            &["info"][..],
            "no_photo_selected",
            "Nothing is selected in Lightroom. Pass --photo <id> (see `lrcli search`).",
        ),
        (
            "info",
            &["info", "--photo", "99"],
            "photo_not_found",
            "Photos not found in the catalog: 99.",
        ),
        (
            "select",
            &["select", "--photo", "1"],
            "select_failed",
            "Lightroom did not select all requested photos.",
        ),
        (
            "search",
            &["search", "--folder", "x"],
            "bad_request",
            "Filter \"folder\" must be a non-empty string.",
        ),
    ] {
        plugin.canned(cmd, json!({ "error": code, "message": message }));
        let out = run(plugin.home(), args).await;
        assert_eq!(out.code, 1, "{args:?}");
        assert_eq!(out.stderr.trim(), format!("error: {message}"));
        let mut json_args = vec!["--json"];
        json_args.extend_from_slice(args);
        let body = run(plugin.home(), &json_args).await.json();
        assert_eq!(body["error"], code);
    }
}

#[tokio::test]
async fn usage_errors_exit_2_without_contacting_the_plugin() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let cases: Vec<&[&str]> = vec![
        &["search", "--rating", "9"],
        &["search", "--rating", "abc"],
        &["search", "--flag", "maybe"],
        &["search", "--label", "pink"],
        &["search", "--since", "2024-1-1"],
        &["search", "--limit", "501"],
        &["select"],
        &["select", "--photo", "x"],
        &["info", "--photo", "1", "--photos", "-"],
        &["info", "--photos", "file.txt"],
    ];
    for args in cases {
        let out = run(plugin.home(), args).await;
        assert_eq!(out.code, 2, "{args:?}: {}", out.stderr);
    }
    for (input, label) in [("", "empty"), ("abc\n", "garbage"), ("[]", "empty array")] {
        let out = run_stdin(plugin.home(), &["select", "--photos", "-"], input).await;
        assert_eq!(out.code, 2, "{label}: {}", out.stderr);
    }
    let out = run_stdin(plugin.home(), &["--json", "info", "--photos", "-"], "").await;
    assert_eq!(out.code, 2);
    assert_eq!(out.json()["error"], "usage");
    let requests = plugin.stats.requests.lock().unwrap();
    assert!(
        requests.iter().all(|(cmd, _)| cmd == "hello"),
        "{requests:?}"
    );
}

#[tokio::test]
async fn help_for_the_catalog_commands_has_examples() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    for cmd in ["search", "select", "selection", "info"] {
        let out = run(plugin.home(), &[cmd, "--help"]).await;
        assert_eq!(out.code, 0);
        assert!(out.stdout.contains("Examples:"), "{cmd}");
    }
}
