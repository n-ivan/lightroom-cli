mod common;

use common::{Options, run, run_fake_app, start_broker};
use tempfile::TempDir;

#[tokio::test]
async fn start_is_a_no_op_when_the_plugin_already_answers() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let out = run_fake_app(plugin.home(), &["start"], "running").await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout.contains("already running"), "{}", out.stdout);
    assert!(out.stdout.contains("/fake/catalog.lrcat"));

    let out = run_fake_app(plugin.home(), &["--json", "start"], "stopped").await;
    assert_eq!(out.code, 0);
    let body = out.json();
    assert_eq!(body["started"], false);
    assert_eq!(body["catalog"]["path"], "/fake/catalog.lrcat");
}

#[tokio::test]
async fn start_with_a_different_catalog_asks_for_restart() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let dir = TempDir::new().unwrap();
    let catalog = dir.path().join("other.lrcat");
    std::fs::write(&catalog, "").unwrap();
    let out = run_fake_app(
        plugin.home(),
        &["start", "--catalog", catalog.to_str().unwrap()],
        "running",
    )
    .await;
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains("lrcli restart --catalog"),
        "{}",
        out.stderr
    );
}

#[tokio::test]
async fn start_rejects_bad_catalog_paths() {
    let home = TempDir::new().unwrap();
    for path in ["/nonexistent/x.lrcat", "/etc/hosts"] {
        let out = run_fake_app(home.path(), &["start", "--catalog", path], "stopped").await;
        assert_eq!(out.code, 2, "{path}: {}", out.stderr);
    }
}

#[tokio::test]
async fn start_times_out_with_exit_3_when_the_plugin_never_answers() {
    let home = TempDir::new().unwrap();
    let out = run_fake_app(
        home.path(),
        &["--connect-timeout", "1", "start", "--wait", "1"],
        "running",
    )
    .await;
    assert_eq!(out.code, 3);
    assert!(
        out.stderr.contains("didn't answer within 1 s"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("lrcli plugin install"));
}

#[tokio::test]
async fn stop_reports_not_running_and_never_force_kills() {
    let home = TempDir::new().unwrap();
    let out = run_fake_app(home.path(), &["stop"], "stopped").await;
    assert_eq!(out.code, 0);
    assert!(out.stdout.contains("not running"));
    assert_eq!(
        run_fake_app(home.path(), &["--json", "stop"], "stopped")
            .await
            .json()["stopped"],
        false
    );

    let out = run_fake_app(home.path(), &["stop", "--wait", "1"], "running").await;
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains("didn't quit within 1 s"),
        "{}",
        out.stderr
    );
}

#[tokio::test]
async fn lifecycle_commands_document_themselves() {
    let home = TempDir::new().unwrap();
    for cmd in ["start", "stop", "restart"] {
        let out = run(home.path(), &[cmd, "--help"]).await;
        assert_eq!(out.code, 0);
        assert!(out.stdout.contains("Examples:"), "{cmd}");
    }
}

#[tokio::test]
async fn version_mismatch_warns_on_stderr_for_every_command_and_flags_status() {
    let (plugin, _broker) = start_broker(Options {
        plugin_version: "9.9.9".into(),
        ..Options::default()
    })
    .await;
    let warning = "lrcli 0.1.0 talks to plugin 9.9.9; run `lrcli plugin install && lrcli restart`";

    let out = run(plugin.home(), &["status"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stderr.contains(warning), "{}", out.stderr);
    assert!(out.stdout.contains("Warning:"), "{}", out.stdout);

    let out = run(plugin.home(), &["--json", "status"]).await;
    assert!(out.stderr.contains(warning), "{}", out.stderr);
    assert_eq!(out.json()["versionMismatch"], true);

    let out = run(plugin.home(), &["--json", "selection"]).await;
    assert!(out.stderr.contains(warning), "{}", out.stderr);
    assert!(!out.stdout.contains("talks to plugin"));
}

#[tokio::test]
async fn matching_versions_print_no_warning() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let out = run(plugin.home(), &["--json", "status"]).await;
    assert!(out.stderr.is_empty(), "{}", out.stderr);
    assert!(out.json().get("versionMismatch").is_none());
}
