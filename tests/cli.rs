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

    fn target(&self) -> std::path::PathBuf {
        self.modules.path().join("lrcli.lrdevplugin")
    }
}

#[tokio::test]
async fn plugin_install_links_the_plugin_and_writes_config() {
    let env = Env::new();
    let out = env.run(&["plugin", "install"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout.contains("lrcli restart"));

    let link = fs::read_link(env.target()).unwrap();
    assert!(link.ends_with("plugin/lrcli.lrdevplugin"), "{link:?}");
    assert!(link.join("Info.lua").exists());

    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(env.home.path().join("config.json")).unwrap()).unwrap();
    assert_eq!(config["version"], 1);
    let broker_path = fs::canonicalize(env!("CARGO_BIN_EXE_lrcli")).unwrap();
    assert_eq!(config["brokerPath"], broker_path.to_str().unwrap());
}

#[tokio::test]
async fn plugin_install_twice_is_a_no_op() {
    let env = Env::new();
    env.run(&["plugin", "install"]).await;
    let out = env.run(&["--json", "plugin", "install"]).await;
    assert_eq!(out.code, 0);
    assert_eq!(out.json()["action"], "unchanged");
}

#[tokio::test]
async fn plugin_install_refuses_a_foreign_target_unless_forced() {
    let env = Env::new();
    fs::create_dir(env.target()).unwrap();
    let out = env.run(&["--json", "plugin", "install"]).await;
    assert_eq!(out.code, 1);
    assert_eq!(out.json()["error"], "plugin_target_exists");
    assert!(!env.home.path().join("config.json").exists());

    let out = env.run(&["--json", "plugin", "install", "--force"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.json()["action"], "replaced");
    assert!(fs::symlink_metadata(env.target()).unwrap().is_symlink());
}

#[tokio::test]
async fn plugin_uninstall_removes_the_link_and_config_only() {
    let env = Env::new();
    env.run(&["plugin", "install"]).await;
    let out = env.run(&["--json", "plugin", "uninstall"]).await;
    assert_eq!(out.code, 0);
    assert_eq!(out.json()["action"], "removed");
    assert!(fs::symlink_metadata(env.target()).is_err());
    assert!(!env.home.path().join("config.json").exists());

    let out = env.run(&["--json", "plugin", "uninstall"]).await;
    assert_eq!(out.json()["action"], "absent");

    fs::create_dir(env.target()).unwrap();
    let out = env.run(&["--json", "plugin", "uninstall"]).await;
    assert_eq!(out.code, 1);
    assert!(env.target().is_dir());
}

#[tokio::test]
async fn plugin_path_prints_source_target_and_config() {
    let env = Env::new();
    let out = env.run(&["--json", "plugin", "path"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    let body = out.json();
    assert!(
        body["source"]
            .as_str()
            .unwrap()
            .ends_with("plugin/lrcli.lrdevplugin")
    );
    assert_eq!(body["target"], env.target().to_str().unwrap());
    assert_eq!(
        body["config"],
        env.home.path().join("config.json").to_str().unwrap()
    );
    let out = env.run(&["plugin", "path"]).await;
    assert!(
        out.stdout.contains("Source: ")
            && out.stdout.contains("Target: ")
            && out.stdout.contains("Config: ")
    );
}
