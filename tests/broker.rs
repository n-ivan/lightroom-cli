mod common;

use std::time::Duration;

use common::{
    FakePlugin, Options, TOKEN, run, spawn_broker, start_broker, wait_exit, wait_for_socket,
};
use lrcli::client::Client;
use lrcli::error::Kind;
use serde_json::json;
use tokio::time::sleep;

async fn client(plugin: &FakePlugin, timeout_secs: u64) -> Client {
    Client::connect(
        &plugin.socket(),
        Duration::from_secs(2),
        Duration::from_secs(timeout_secs),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn status_prints_human_text_through_a_real_broker() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let out = run(plugin.home(), &["status"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout.contains("Lightroom Classic 15.0 [fake]"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("Plugin:  0.1.0 (protocol 1)"));
    assert!(out.stdout.contains("Catalog: /fake/catalog.lrcat"));
    assert!(out.stdout.contains("Broker:  0.1.0 (pid "));
}

#[tokio::test]
async fn status_json_omits_the_id_and_adds_broker_info() {
    let (plugin, broker) = start_broker(Options::default()).await;
    let out = run(plugin.home(), &["--json", "status"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    let reply = out.json();
    assert!(reply.get("id").is_none());
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["lightroom"]["version"], "15.0 [fake]");
    assert_eq!(reply["broker"]["pid"], broker.id().unwrap());
}

#[tokio::test]
async fn status_honours_the_lrcli_output_env() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_lrcli"));
    let output = command
        .env("LRCLI_HOME", plugin.home())
        .env("LRCLI_OUTPUT", "json")
        .arg("status")
        .output()
        .await
        .unwrap();
    let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["ok"], true);
}

#[tokio::test]
async fn fifty_sequential_status_calls_reuse_one_upstream_connection() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    for _ in 0..50 {
        let out = run(plugin.home(), &["status"]).await;
        assert_eq!(out.code, 0, "{}", out.stderr);
    }
    assert_eq!(plugin.connections(), (1, 1));
    assert_eq!(
        plugin
            .stats
            .hellos
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn concurrent_clients_get_their_own_replies() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let mut tasks = Vec::new();
    for i in 0..8u64 {
        let mut client = client(&plugin, 10).await;
        tasks.push(tokio::spawn(async move {
            let delay = (8 - i) * 60;
            let reply = client
                .request("echo", json!({ "delay_ms": delay, "value": i }))
                .await
                .unwrap();
            assert_eq!(reply["value"], i);
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(plugin.connections(), (1, 1));
}

#[tokio::test]
async fn pipelined_replies_arriving_out_of_order_are_routed_by_id() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let slow = tokio::spawn({
        let mut c = client(&plugin, 10).await;
        async move {
            c.request("echo", json!({ "delay_ms": 400, "value": "slow" }))
                .await
                .unwrap()
        }
    });
    sleep(Duration::from_millis(50)).await;
    let fast = client(&plugin, 10)
        .await
        .request("echo", json!({ "value": "fast" }))
        .await
        .unwrap();
    assert_eq!(fast["value"], "fast");
    assert_eq!(slow.await.unwrap()["value"], "slow");
}

#[tokio::test]
async fn plugin_errors_pass_through_with_exit_code_1() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let out = run(plugin.home(), &["--json", "raw", "nope"]).await;
    assert_eq!(out.code, 1);
    assert_eq!(out.json()["error"], "unknown_command");
    let out = run(plugin.home(), &["raw", "nope"]).await;
    assert!(
        out.stderr.starts_with("error: Unknown command"),
        "{}",
        out.stderr
    );
    assert!(out.stdout.is_empty());
}

#[tokio::test]
async fn broker_exits_and_removes_its_socket_when_the_plugin_closes() {
    let (plugin, mut broker) = start_broker(Options::default()).await;
    plugin.close_link();
    assert!(wait_exit(&mut broker, 5).await.success());
    assert!(!plugin.socket().exists());
    let out = run(plugin.home(), &["status"]).await;
    assert_eq!(out.code, 3);
}

#[tokio::test]
async fn pending_requests_get_connection_lost_when_the_link_drops() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let mut c = client(&plugin, 10).await;
    let pending = tokio::spawn(async move { c.request("hang", json!({})).await });
    sleep(Duration::from_millis(200)).await;
    plugin.close_link();
    let error = pending.await.unwrap().unwrap_err();
    assert_eq!(error.code, "connection_lost");
    assert_eq!(error.kind, Kind::Unreachable);
}

#[tokio::test]
async fn broker_exits_when_the_ports_token_changes() {
    let (plugin, mut broker) = start_broker(Options::default()).await;
    plugin.rewrite_ports("ffffffffffffffffffffffffffffffff");
    assert!(wait_exit(&mut broker, 6).await.success());
    assert!(!plugin.socket().exists());
}

#[tokio::test]
async fn second_broker_with_the_same_token_exits_zero_and_the_first_keeps_serving() {
    let (plugin, _first) = start_broker(Options::default()).await;
    let mut second = spawn_broker(plugin.home());
    assert!(wait_exit(&mut second, 5).await.success());
    let out = run(plugin.home(), &["status"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(plugin.connections(), (1, 1));
}

#[tokio::test]
async fn broker_for_a_new_session_replaces_the_old_one() {
    let (old_plugin, mut old_broker) = start_broker(Options::default()).await;
    let new_token = "abcdefabcdefabcdefabcdefabcdefab";
    let new_plugin = FakePlugin::start(Options {
        token: new_token.into(),
        protocol: 1,
    })
    .await;
    std::fs::copy(
        new_plugin.home().join("ports.json"),
        old_plugin.home().join("ports.json"),
    )
    .unwrap();
    let mut new_broker = spawn_broker(old_plugin.home());
    assert!(wait_exit(&mut old_broker, 6).await.success());
    wait_for_socket(old_plugin.home()).await;
    let out = run(old_plugin.home(), &["--json", "status"]).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.json()["broker"]["pid"], new_broker.id().unwrap());
    assert_eq!(new_plugin.connections(), (1, 1));
    let _ = new_broker.kill().await;
}

#[tokio::test]
async fn stale_socket_file_is_cleaned_up() {
    let plugin = FakePlugin::start(Options::default()).await;
    // A datagram socket leaves the same stale file but never listens: a listener
    // can leak into a child spawned by a parallel test (macOS sets CLOEXEC after
    // socket()), which would keep the "stale" socket accepting connections.
    drop(std::os::unix::net::UnixDatagram::bind(plugin.socket()).unwrap());
    assert!(plugin.socket().exists());
    let _broker = spawn_broker(plugin.home());
    wait_for_socket(plugin.home()).await;
    let out = run(plugin.home(), &["status"]).await;
    let log = std::fs::read_to_string(plugin.home().join("broker.log")).unwrap_or_default();
    assert_eq!(out.code, 0, "{}\nbroker.log:\n{log}", out.stderr);
}

#[tokio::test]
async fn broker_shutdown_command_stops_the_broker() {
    let (plugin, mut broker) = start_broker(Options::default()).await;
    client(&plugin, 5)
        .await
        .request("broker_shutdown", json!({}))
        .await
        .unwrap();
    assert!(wait_exit(&mut broker, 5).await.success());
    assert!(!plugin.socket().exists());
}

#[tokio::test]
async fn broker_info_reports_the_token_fingerprint() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let info = client(&plugin, 5)
        .await
        .request("broker_info", json!({}))
        .await
        .unwrap();
    assert_eq!(info["token_fingerprint"], &TOKEN[..8]);
    assert_eq!(info["state"], "ready");
}

#[tokio::test]
async fn wrong_token_makes_status_exit_4_while_the_broker_keeps_running() {
    let plugin = FakePlugin::start(Options::default()).await;
    plugin.rewrite_ports("ffffffffffffffffffffffffffffffff");
    let mut broker = spawn_broker(plugin.home());
    wait_for_socket(plugin.home()).await;
    let out = run(plugin.home(), &["--json", "status"]).await;
    assert_eq!(out.code, 4);
    assert_eq!(out.json()["error"], "unauthorized");
    let out = run(plugin.home(), &["status"]).await;
    assert_eq!(out.code, 4);
    assert!(
        out.stderr.starts_with("error: Invalid or missing token"),
        "{}",
        out.stderr
    );
    assert!(broker.try_wait().unwrap().is_none());
}

#[tokio::test]
async fn protocol_mismatch_makes_status_exit_4() {
    let (plugin, _broker) = start_broker(Options {
        protocol: 2,
        ..Options::default()
    })
    .await;
    let out = run(plugin.home(), &["--json", "status"]).await;
    assert_eq!(out.code, 4);
    assert_eq!(out.json()["error"], "protocol_mismatch");
}

#[tokio::test]
async fn request_timeout_exits_5() {
    let (plugin, _broker) = start_broker(Options::default()).await;
    let out = run(
        plugin.home(),
        &["--timeout", "0.5", "--json", "raw", "hang"],
    )
    .await;
    assert_eq!(out.code, 5);
    assert_eq!(out.json()["error"], "timeout");
}
