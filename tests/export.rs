mod common;

use common::{Options, run, run_stdin, start_broker};
use serde_json::{Value, json};

fn export_reply() -> Value {
    json!({ "target": "ids", "dest": "{dir}", "photos": [
        { "id": 149, "filename": "DSC08011-2.ARW", "file": "{dir}/DSC08011-2.jpg", "error": null },
        { "id": 150, "filename": "v.mov", "file": null, "error": "Videos cannot be exported." },
    ] })
}

#[tokio::test]
async fn export_sends_defaults_and_reports_sizes() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("export", export_reply());
    let dest = plugin.home().join("out");
    let dest_arg = dest.to_str().unwrap();
    let out = run(
        plugin.home(),
        &[
            "export", "--photo", "149", "--photo", "150", "--dest", dest_arg,
        ],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);

    let args = plugin.last_args("export");
    assert_eq!(
        args,
        json!({
            "photos": [149, 150], "dest": dest_arg, "format": "JPEG", "quality": 90,
            "colorSpace": "sRGB", "metadata": "all", "removeLocation": false,
            "name": "{{image_name}}", "onConflict": "rename", "addToCatalog": false,
        })
    );
    assert!(dest.is_dir());

    let lines: Vec<&str> = out.stdout.lines().collect();
    let file = std::fs::metadata(dest.join("DSC08011-2.jpg"))
        .unwrap()
        .len();
    assert!(file > 0 && file < 1024);
    assert_eq!(
        lines[0],
        format!("DSC08011-2.ARW (149)  {dest_arg}/DSC08011-2.jpg  {file} B")
    );
    assert_eq!(lines[1], "v.mov (150)  error: Videos cannot be exported.");
    assert_eq!(lines[2], format!("Exported 1 of 2 to {dest_arg}"));
}

#[tokio::test]
async fn export_maps_every_flag() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("export", export_reply());
    let dest = plugin.home().join("a/b");
    let dest_arg = dest.to_str().unwrap();
    let out = run(
        plugin.home(),
        &[
            "export",
            "--dest",
            dest_arg,
            "--format",
            "tiff",
            "--short-edge",
            "1000",
            "--color-space",
            "prophoto",
            "--bit-depth",
            "16",
            "--tiff-compression",
            "lzw",
            "--sharpen",
            "glossy",
            "--sharpen-amount",
            "high",
            "--metadata",
            "all-except-camera",
            "--remove-location",
            "--name",
            "{{date_YYYYMMDD}}-{{naming_sequenceNumber_3Digits}}",
            "--on-conflict",
            "skip",
            "--subfolder",
            "print",
            "--add-to-catalog",
        ],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        plugin.last_args("export"),
        json!({
            "dest": dest_arg, "format": "TIFF", "size": { "type": "shortEdge", "value": 1000 },
            "colorSpace": "ProPhotoRGB", "bitDepth": 16,
            "tiffCompression": "compressionMethod_LZW",
            "sharpen": { "media": "glossy", "level": 3 }, "metadata": "allExceptCameraInfo",
            "removeLocation": true,
            "name": "{{date_YYYYMMDD}}-{{naming_sequenceNumber_3Digits}}",
            "onConflict": "skip", "subfolder": "print", "addToCatalog": true,
        })
    );
}

#[tokio::test]
async fn export_maps_formats_sizes_and_enums() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("export", export_reply());
    let dest = plugin.home().join("out");
    let dest_arg = dest.to_str().unwrap();
    let cases: &[(&[&str], &str, Value)] = &[
        (
            &["--long-edge", "2048", "--quality", "70"],
            "size",
            json!({ "type": "longEdge", "value": 2048 }),
        ),
        (
            &["--size", "1920x1080"],
            "size",
            json!({ "type": "dimensions", "width": 1920, "height": 1080 }),
        ),
        (
            &["--megapixels", "2.5"],
            "size",
            json!({ "type": "megapixels", "value": 2.5 }),
        ),
        (
            &["--format", "psd", "--bit-depth", "8"],
            "bitDepth",
            json!(8),
        ),
        (&["--format", "dng"], "format", json!("DNG")),
        (&["--format", "original"], "format", json!("ORIGINAL")),
        (
            &["--color-space", "adobergb"],
            "colorSpace",
            json!("AdobeRGB"),
        ),
        (
            &["--metadata", "copyright"],
            "metadata",
            json!("copyrightOnly"),
        ),
        (
            &["--metadata", "copyright-contact"],
            "metadata",
            json!("copyrightAndContactOnly"),
        ),
        (
            &["--format", "tiff", "--tiff-compression", "none"],
            "tiffCompression",
            json!("compressionMethod_None"),
        ),
        (
            &["--format", "tiff", "--tiff-compression", "zip"],
            "tiffCompression",
            json!("compressionMethod_ZIP"),
        ),
        (
            &["--sharpen", "screen"],
            "sharpen",
            json!({ "media": "screen", "level": 2 }),
        ),
        (
            &["--sharpen", "matte", "--sharpen-amount", "low"],
            "sharpen",
            json!({ "media": "matte", "level": 1 }),
        ),
        (
            &["--on-conflict", "overwrite"],
            "onConflict",
            json!("overwrite"),
        ),
    ];
    for (flags, key, expected) in cases {
        let mut argv = vec!["export", "--dest", dest_arg];
        argv.extend_from_slice(flags);
        let out = run(plugin.home(), &argv).await;
        assert_eq!(out.code, 0, "{flags:?}: {}", out.stderr);
        assert_eq!(&plugin.last_args("export")[*key], expected, "{flags:?}");
    }
    let out = run(
        plugin.home(),
        &["export", "--dest", dest_arg, "--format", "dng"],
    )
    .await;
    assert_eq!(out.code, 0);
    let args = plugin.last_args("export");
    assert!(args.get("quality").is_none() && args.get("size").is_none());
}

#[tokio::test]
async fn export_defaults_to_the_selection_and_reads_stdin_ids() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("export", export_reply());
    let dest = plugin.home().join("out");
    let dest_arg = dest.to_str().unwrap();
    let out = run(plugin.home(), &["export", "--dest", dest_arg]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(plugin.last_args("export").get("photos").is_none());

    let out = run_stdin(
        plugin.home(),
        &["export", "--photos", "-", "--dest", dest_arg],
        "149\n150\n",
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(plugin.last_args("export")["photos"], json!([149, 150]));
}

#[tokio::test]
async fn export_resolves_a_relative_dest_to_an_absolute_path() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("export", export_reply());
    let name = format!("lrcli-export-test-{}", std::process::id());
    let out = run(plugin.home(), &["export", "--dest", &name]).await;
    let created = std::env::current_dir().unwrap().join(&name);
    let is_dir = created.is_dir();
    let _ = std::fs::remove_dir_all(&created);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(is_dir);
    let dest = plugin.last_args("export")["dest"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(dest, created.to_str().unwrap());
}

#[tokio::test]
async fn export_json_adds_bytes_per_photo() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("export", export_reply());
    let dest = plugin.home().join("out");
    let out = run(
        plugin.home(),
        &[
            "--json",
            "export",
            "--photo",
            "149",
            "--dest",
            dest.to_str().unwrap(),
        ],
    )
    .await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    let reply = out.json();
    assert!(reply.get("id").is_none());
    assert_eq!(reply["dest"], dest.to_str().unwrap());
    let size = std::fs::metadata(dest.join("DSC08011-2.jpg"))
        .unwrap()
        .len();
    assert_eq!(reply["photos"][0]["bytes"], size);
    assert!(reply["photos"][1].get("bytes").is_none());
}

#[tokio::test]
async fn export_exits_1_when_every_photo_failed() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned(
        "export",
        json!({ "target": "ids", "dest": "/x", "photos": [
            { "id": 1, "filename": "a", "file": null, "error": "render failed" },
            { "id": 2, "filename": "b", "file": null, "error": "other" } ] }),
    );
    let dest = plugin.home().join("out");
    let out = run(plugin.home(), &["export", "--dest", dest.to_str().unwrap()]).await;
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("render failed"), "{}", out.stderr);
}

#[tokio::test]
async fn export_plugin_errors_exit_1() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned(
        "export",
        json!({ "error": "bad_request", "message": "\"quality\" must be an integer." }),
    );
    let dest = plugin.home().join("out");
    let out = run(plugin.home(), &["export", "--dest", dest.to_str().unwrap()]).await;
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("quality"), "{}", out.stderr);
}

#[tokio::test]
async fn export_usage_errors_exit_2_without_contacting_the_plugin() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    plugin.canned("export", export_reply());
    let dest = plugin.home().join("out");
    let d = dest.to_str().unwrap();
    let cases: &[&[&str]] = &[
        &["export"],
        &["export", "--dest", d, "--format", "tiff", "--quality", "80"],
        &["export", "--dest", d, "--bit-depth", "8"],
        &[
            "export",
            "--dest",
            d,
            "--format",
            "psd",
            "--tiff-compression",
            "zip",
        ],
        &[
            "export",
            "--dest",
            d,
            "--format",
            "jpeg",
            "--bit-depth",
            "16",
        ],
        &["export", "--dest", d, "--quality", "101"],
        &[
            "export",
            "--dest",
            d,
            "--bit-depth",
            "12",
            "--format",
            "tiff",
        ],
        &[
            "export",
            "--dest",
            d,
            "--long-edge",
            "100",
            "--short-edge",
            "100",
        ],
        &[
            "export",
            "--dest",
            d,
            "--long-edge",
            "100",
            "--size",
            "10x10",
        ],
        &[
            "export",
            "--dest",
            d,
            "--megapixels",
            "5",
            "--size",
            "10x10",
        ],
        &["export", "--dest", d, "--size", "10"],
        &["export", "--dest", d, "--long-edge", "0"],
        &["export", "--dest", d, "--megapixels", "0"],
        &["export", "--dest", d, "--sharpen-amount", "high"],
        &["export", "--dest", d, "--format", "png"],
        &["export", "--dest", d, "--on-conflict", "ask"],
    ];
    for argv in cases {
        let out = run(plugin.home(), argv).await;
        assert_eq!(out.code, 2, "{argv:?}: {}", out.stderr);
    }
    assert!(
        !plugin
            .stats
            .commands
            .lock()
            .unwrap()
            .iter()
            .any(|c| c == "export")
    );
}

#[tokio::test]
async fn export_fails_locally_when_dest_is_a_file() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let file = plugin.home().join("not-a-dir");
    std::fs::write(&file, "x").unwrap();
    let out = run(plugin.home(), &["export", "--dest", file.to_str().unwrap()]).await;
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("Cannot create"), "{}", out.stderr);
}
