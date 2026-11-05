mod common;

use std::fs;
use std::time::{Duration, Instant};

use common::{run, run_env};
use tempfile::TempDir;

#[tokio::test]
async fn status_without_a_broker_exits_3_within_the_connect_timeout() {
    let home = TempDir::new().unwrap();
    let started = Instant::now();
    let out = run(home.path(), &["--connect-timeout", "1", "status"]).await;
    assert_eq!(out.code, 3);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(
        out.stderr.starts_with("error: Can't reach Lightroom."),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("lrcli plugin install"));
    assert!(out.stdout.is_empty());
}

#[tokio::test]
async fn unreachable_error_is_json_in_json_mode() {
    let home = TempDir::new().unwrap();
    let out = run(home.path(), &["--json", "status"]).await;
    assert_eq!(out.code, 3);
    let body = out.json();
    assert_eq!(body["ok"], false);
    assert_eq!(body["error"], "unreachable");
}

#[tokio::test]
async fn unreachable_errors_show_recent_log_lines_only() {
    let home = TempDir::new().unwrap();
    let now = lrcli::log::unix_now();
    let stamp = lrcli::log::format_timestamp;
    fs::write(
        home.path().join("plugin.log"),
        format!(
            "{} INFO stale plugin line\n{} ERROR fresh plugin line\n",
            stamp(now - 3600),
            stamp(now - 30)
        ),
    )
    .unwrap();
    fs::write(
        home.path().join("broker.log"),
        format!(
            "{} stale broker line\n{} fresh broker line\n",
            lrcli::log::format_timestamp(now - 3600),
            lrcli::log::format_timestamp(now - 30)
        ),
    )
    .unwrap();

    let human = run(home.path(), &["status"]).await;
    assert_eq!(human.code, 3);
    assert!(
        human.stderr.contains("Recent plugin log:"),
        "{}",
        human.stderr
    );
    assert!(
        human.stderr.contains("fresh plugin line"),
        "{}",
        human.stderr
    );
    assert!(
        human.stderr.contains("Recent broker log:"),
        "{}",
        human.stderr
    );
    assert!(
        human.stderr.contains("fresh broker line"),
        "{}",
        human.stderr
    );
    assert!(!human.stderr.contains("stale"), "{}", human.stderr);

    let json = run(home.path(), &["--json", "status"]).await.json();
    let plugin = json["diagnostics"]["plugin"].as_array().unwrap();
    assert_eq!(plugin.len(), 1);
    assert!(plugin[0].as_str().unwrap().contains("fresh plugin line"));
    let broker = json["diagnostics"]["broker"].as_array().unwrap();
    assert!(
        broker
            .iter()
            .all(|l| !l.as_str().unwrap().contains("stale"))
    );
    assert!(
        broker
            .iter()
            .any(|l| l.as_str().unwrap().contains("fresh broker line"))
    );
}

#[tokio::test]
async fn usage_errors_exit_2() {
    let home = TempDir::new().unwrap();
    let out = run(home.path(), &["bogus"]).await;
    assert_eq!(out.code, 2);
    assert!(out.stderr.contains("bogus"));

    let out = run(home.path(), &["--json", "bogus"]).await;
    assert_eq!(out.code, 2);
    assert_eq!(out.json()["error"], "usage");

    assert_eq!(
        run(home.path(), &["--timeout", "0", "status"]).await.code,
        2
    );
    assert_eq!(run(home.path(), &[]).await.code, 2);
}

#[tokio::test]
async fn help_documents_every_command_with_examples() {
    let home = TempDir::new().unwrap();
    for args in [
        &["--help"][..],
        &["status", "--help"],
        &["plugin", "--help"],
        &["plugin", "install", "--help"],
        &["plugin", "uninstall", "--help"],
        &["plugin", "path", "--help"],
    ] {
        let out = run(home.path(), args).await;
        assert_eq!(out.code, 0, "{args:?}");
        assert!(out.stdout.contains("lrcli"), "{args:?}");
    }
    for args in [
        &["--help"][..],
        &["status", "--help"],
        &["plugin", "install", "--help"],
    ] {
        assert!(
            run(home.path(), args).await.stdout.contains("Examples:"),
            "{args:?}"
        );
    }
    let top = run(home.path(), &["--help"]).await.stdout;
    assert!(
        !top.contains("broker\n"),
        "hidden commands must not be listed"
    );
}

struct Env {
    home: TempDir,
    modules: TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            home: TempDir::new().unwrap(),
            modules: TempDir::new().unwrap(),
        }
    }

    async fn run(&self, args: &[&str]) -> common::Out {
        run_env(
            self.home.path(),
            args,
            &[("LRCLI_MODULES_DIR", self.modules.path())],
        )
        .await
    }

    fn dev(&self) -> std::path::PathBuf {
        self.modules.path().join("lrcli.lrdevplugin")
    }

    fn release(&self) -> std::path::PathBuf {
        self.modules.path().join("lrcli.lrplugin")
    }
}

#[tokio::test]
async fn plugin_install_copies_the_plugin_and_the_binary() {
    let env = Env::new();
    let out = env.run(&["plugin", "install"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout.contains("lrcli restart"));

    let target = env.release();
    assert!(fs::symlink_metadata(&target).unwrap().is_dir());
    assert!(target.join("Info.lua").is_file());
    let broker = target.join("bin/lrcli");
    assert_eq!(
        fs::read(&broker).unwrap(),
        fs::read(env!("CARGO_BIN_EXE_lrcli")).unwrap()
    );
    assert!(!env.home.path().join("config.json").exists());

    let out = env.run(&["--json", "plugin", "install"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    let body = out.json();
    assert_eq!(
        (body["mode"].as_str(), body["action"].as_str()),
        (Some("release"), Some("upgraded"))
    );
    assert_eq!(body["brokerPath"], broker.to_str().unwrap());
}

#[tokio::test]
async fn plugin_install_refuses_a_foreign_lrplugin_unless_forced() {
    let env = Env::new();
    fs::create_dir(env.release()).unwrap();
    let out = env.run(&["--json", "plugin", "install"]).await;
    assert_eq!(out.code, 1);
    assert_eq!(out.json()["error"], "plugin_target_exists");
    assert!(!env.release().join("bin").exists());

    let out = env.run(&["plugin", "install", "--force"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(env.release().join("bin/lrcli").exists());
}

#[tokio::test]
async fn dev_install_links_the_plugin_and_writes_config() {
    let env = Env::new();
    let out = env.run(&["plugin", "install", "--dev"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout.contains("lrcli restart"));

    let link = fs::read_link(env.dev()).unwrap();
    assert!(link.ends_with("plugin/lrcli.lrdevplugin"), "{link:?}");
    assert!(link.join("Info.lua").exists());

    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(env.home.path().join("config.json")).unwrap()).unwrap();
    assert_eq!(config["version"], 1);
    let broker_path = fs::canonicalize(env!("CARGO_BIN_EXE_lrcli")).unwrap();
    assert_eq!(config["brokerPath"], broker_path.to_str().unwrap());

    let out = env.run(&["--json", "plugin", "install", "--dev"]).await;
    assert_eq!(out.json()["action"], "unchanged");
}

#[tokio::test]
async fn switching_between_dev_and_release_removes_the_other_copy() {
    let env = Env::new();
    env.run(&["plugin", "install", "--dev"]).await;
    let out = env.run(&["plugin", "install"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout.contains("Removed"), "{}", out.stdout);
    assert!(fs::symlink_metadata(env.dev()).is_err());
    assert!(env.release().is_dir());

    let out = env.run(&["--json", "plugin", "install", "--dev"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.json()["removed"], env.release().to_str().unwrap());
    assert!(!env.release().exists());
    assert!(fs::symlink_metadata(env.dev()).unwrap().is_symlink());
}

#[tokio::test]
async fn plugin_uninstall_removes_ours_and_config_only() {
    let env = Env::new();
    env.run(&["plugin", "install", "--dev"]).await;
    let out = env.run(&["--json", "plugin", "uninstall"]).await;
    assert_eq!(out.code, 0);
    assert_eq!(out.json()["removed"], serde_json::json!([env.dev()]));
    assert!(fs::symlink_metadata(env.dev()).is_err());
    assert!(!env.home.path().join("config.json").exists());

    env.run(&["plugin", "install"]).await;
    let out = env.run(&["plugin", "uninstall"]).await;
    assert_eq!(out.code, 0);
    assert!(!env.release().exists());

    let out = env.run(&["--json", "plugin", "uninstall"]).await;
    assert_eq!(out.json()["removed"], serde_json::json!([]));

    fs::create_dir(env.dev()).unwrap();
    let out = env.run(&["--json", "plugin", "uninstall"]).await;
    assert_eq!(out.code, 1);
    assert!(env.dev().is_dir());
}

#[tokio::test]
async fn plugin_path_reports_the_install_and_its_broker() {
    let env = Env::new();
    let body = env.run(&["--json", "plugin", "path"]).await.json();
    assert_eq!(body["installed"], "none");
    assert!(body["brokerPath"].is_null());
    let out = env.run(&["plugin", "path"]).await;
    assert!(out.stdout.contains("Not installed"), "{}", out.stdout);

    env.run(&["plugin", "install"]).await;
    let body = env.run(&["--json", "plugin", "path"]).await.json();
    assert_eq!(body["installed"], "release");
    assert_eq!(body["target"], env.release().to_str().unwrap());
    assert_eq!(
        body["brokerPath"],
        env.release().join("bin/lrcli").to_str().unwrap()
    );
    let out = env.run(&["plugin", "path"]).await;
    assert!(out.stdout.contains("Installed: release") && out.stdout.contains("Broker:"));

    env.run(&["plugin", "install", "--dev"]).await;
    let body = env.run(&["--json", "plugin", "path"]).await.json();
    assert_eq!(body["installed"], "dev");
    let broker = fs::canonicalize(env!("CARGO_BIN_EXE_lrcli")).unwrap();
    assert_eq!(body["brokerPath"], broker.to_str().unwrap());
}
