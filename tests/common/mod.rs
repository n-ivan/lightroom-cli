#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, UnixStream};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, watch};
use tokio::time::{sleep, timeout};

pub const TOKEN: &str = "0123456789abcdef0123456789abcdef";

pub struct Options {
    pub token: String,
    pub protocol: i64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            token: TOKEN.into(),
            protocol: 1,
        }
    }
}

#[derive(Default)]
pub struct Stats {
    pub reply_conns: AtomicUsize,
    pub command_conns: AtomicUsize,
    pub hellos: AtomicUsize,
    pub pings: AtomicUsize,
    pub commands: Mutex<Vec<String>>,
    pub requests: Mutex<Vec<(String, Value)>>,
    pub canned: Mutex<HashMap<String, Value>>,
}

/// Emulates the plugin's LrSocket pair: replies go out on one listener,
/// commands come in on the other.
pub struct FakePlugin {
    pub home: TempDir,
    pub stats: Arc<Stats>,
    pub reply_port: u16,
    pub command_port: u16,
    close: watch::Sender<bool>,
}

impl FakePlugin {
    pub async fn start(options: Options) -> Self {
        let home = TempDir::new().unwrap();
        let reply_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let command_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let reply_port = reply_listener.local_addr().unwrap().port();
        let command_port = command_listener.local_addr().unwrap().port();
        let stats = Arc::new(Stats::default());
        let token = options.token.clone();
        let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<Value>();
        let (close, close_rx) = watch::channel(false);

        let (s, mut closing) = (stats.clone(), close_rx.clone());
        tokio::spawn(async move {
            let (mut stream, _) = reply_listener.accept().await.unwrap();
            s.reply_conns.fetch_add(1, Ordering::SeqCst);
            loop {
                tokio::select! {
                    Some(reply) = reply_rx.recv() => {
                        let mut line = reply.to_string();
                        line.push('\n');
                        if stream.write_all(line.as_bytes()).await.is_err() { break; }
                    }
                    _ = closing.changed() => break,
                }
            }
        });

        let (s, mut closing) = (stats.clone(), close_rx);
        tokio::spawn(async move {
            let (stream, _) = command_listener.accept().await.unwrap();
            s.command_conns.fetch_add(1, Ordering::SeqCst);
            let mut lines = BufReader::new(stream).lines();
            let mut authenticated = false;
            loop {
                let line = tokio::select! {
                    line = lines.next_line() => match line { Ok(Some(l)) => l, _ => break },
                    _ = closing.changed() => break,
                };
                let request: Value = serde_json::from_str(&line).unwrap();
                let id = request["id"].clone();
                let cmd = request["cmd"].as_str().unwrap().to_string();
                let args = request["args"].clone();
                s.commands.lock().unwrap().push(cmd.clone());
                s.requests.lock().unwrap().push((cmd.clone(), args.clone()));
                let reply = match cmd.as_str() {
                    "hello" => {
                        s.hellos.fetch_add(1, Ordering::SeqCst);
                        if args["token"] != options.token.as_str() {
                            error(&id, "unauthorized", "Invalid or missing token.")
                        } else if args["protocol"] != options.protocol {
                            error(&id, "protocol_mismatch", "Protocol mismatch. Update lrcli.")
                        } else {
                            authenticated = true;
                            json!({
                                "id": id, "ok": true, "protocol": options.protocol,
                                "plugin": { "version": "0.1.0" },
                                "lightroom": { "version": "15.0 [fake]" },
                                "catalog": { "path": "/fake/catalog.lrcat" },
                            })
                        }
                    }
                    _ if !authenticated => error(&id, "unauthorized", "Send hello first."),
                    "ping" => {
                        s.pings.fetch_add(1, Ordering::SeqCst);
                        json!({ "id": id, "ok": true })
                    }
                    "echo" => {
                        let delay = Duration::from_millis(args["delay_ms"].as_u64().unwrap_or(0));
                        let (tx, value) = (reply_tx.clone(), args["value"].clone());
                        tokio::spawn(async move {
                            sleep(delay).await;
                            let _ = tx.send(json!({ "id": id, "ok": true, "value": value }));
                        });
                        continue;
                    }
                    "hang" => continue,
                    other if s.canned.lock().unwrap().contains_key(other) => {
                        let body = s.canned.lock().unwrap()[other].clone();
                        canned_reply(&id, materialize_files(body, &args))
                    }
                    other => error(
                        &id,
                        "unknown_command",
                        &format!("Unknown command \"{other}\"."),
                    ),
                };
                let _ = reply_tx.send(reply);
            }
        });

        let plugin = Self {
            home,
            stats,
            reply_port,
            command_port,
            close,
        };
        plugin.rewrite_ports(&token);
        plugin
    }

    pub fn home(&self) -> &Path {
        self.home.path()
    }

    pub fn socket(&self) -> PathBuf {
        self.home().join("broker.sock")
    }

    pub fn rewrite_ports(&self, token: &str) {
        let ports = json!({
            "version": 1, "commandPort": self.command_port, "replyPort": self.reply_port,
            "token": token, "pluginVersion": "0.1.0", "updatedAt": 0,
        });
        let tmp = self.home().join("ports.json.tmp");
        std::fs::write(&tmp, ports.to_string()).unwrap();
        std::fs::rename(tmp, self.home().join("ports.json")).unwrap();
    }

    /// `body` is an object merged into an ok reply, or `{"error", "message"}` for a failure.
    pub fn canned(&self, cmd: &str, body: Value) {
        self.stats
            .canned
            .lock()
            .unwrap()
            .insert(cmd.to_string(), body);
    }

    /// Args of the most recent request for `cmd`.
    pub fn last_args(&self, cmd: &str) -> Value {
        self.stats
            .requests
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(c, _)| c == cmd)
            .unwrap_or_else(|| panic!("plugin never received {cmd}"))
            .1
            .clone()
    }

    pub fn close_link(&self) {
        let _ = self.close.send(true);
    }

    pub fn connections(&self) -> (usize, usize) {
        (
            self.stats.reply_conns.load(Ordering::SeqCst),
            self.stats.command_conns.load(Ordering::SeqCst),
        )
    }
}

/// Replaces `{dir}` in a canned body with the request's `dir` and writes a
/// tiny mid-grey JPEG at every `photos[].file`, as the real plugin would.
fn materialize_files(body: Value, args: &Value) -> Value {
    let Some(dir) = args["dir"].as_str() else {
        return body;
    };
    let body: Value = serde_json::from_str(&body.to_string().replace("{dir}", dir)).unwrap();
    for photo in body["photos"].as_array().into_iter().flatten() {
        if let Some(file) = photo["file"].as_str() {
            write_grey_jpeg(Path::new(file));
        }
    }
    body
}

pub fn write_grey_jpeg(path: &Path) {
    let image = image::RgbImage::from_pixel(10, 6, image::Rgb([128, 128, 128]));
    image
        .save_with_format(path, image::ImageFormat::Jpeg)
        .unwrap();
}

fn canned_reply(id: &Value, body: Value) -> Value {
    let mut reply = body;
    reply["id"] = id.clone();
    reply["ok"] = json!(reply.get("error").is_none());
    reply
}

fn error(id: &Value, code: &str, message: &str) -> Value {
    json!({ "id": id, "ok": false, "error": code, "message": message })
}

pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn json(&self) -> Value {
        serde_json::from_str(self.stdout.trim())
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", self.stdout))
    }
}

fn lrcli(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lrcli"));
    command
        .env("LRCLI_HOME", home)
        .env_remove("LRCLI_OUTPUT")
        .env_remove("LRCLI_MODULES_DIR")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    command
}

pub async fn run(home: &Path, args: &[&str]) -> Out {
    run_env(home, args, &[]).await
}

pub async fn run_env(home: &Path, args: &[&str], env: &[(&str, &Path)]) -> Out {
    let mut command = lrcli(home);
    command.args(args);
    for (key, value) in env {
        command.env(key, value);
    }
    let output = timeout(Duration::from_secs(20), command.output())
        .await
        .expect("lrcli hung")
        .unwrap();
    Out {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

pub async fn run_fake_app(home: &Path, args: &[&str], app: &str) -> Out {
    let mut command = lrcli(home);
    command.args(args).env("LRCLI_FAKE_APP", app);
    let output = timeout(Duration::from_secs(20), command.output())
        .await
        .expect("lrcli hung")
        .unwrap();
    Out {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

pub async fn run_stdin(home: &Path, args: &[&str], input: &str) -> Out {
    let mut command = lrcli(home);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(input.as_bytes()).await.unwrap();
    drop(stdin);
    let output = timeout(Duration::from_secs(20), child.wait_with_output())
        .await
        .expect("lrcli hung")
        .unwrap();
    Out {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

pub fn spawn_broker(home: &Path) -> Child {
    lrcli(home)
        .arg("broker")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

pub async fn wait_for_socket(home: &Path) {
    let socket = home.join("broker.sock");
    for _ in 0..100 {
        if UnixStream::connect(&socket).await.is_ok() {
            return;
        }
        sleep(Duration::from_millis(50)).await;
    }
    let log = std::fs::read_to_string(home.join("broker.log")).unwrap_or_default();
    panic!("broker socket never came up. broker.log:\n{log}");
}

pub async fn wait_exit(child: &mut Child, secs: u64) -> ExitStatus {
    timeout(Duration::from_secs(secs), child.wait())
        .await
        .expect("process did not exit in time")
        .unwrap()
}

pub async fn start_broker(options: Options) -> (FakePlugin, Child) {
    let plugin = FakePlugin::start(options).await;
    let broker = spawn_broker(plugin.home());
    wait_for_socket(plugin.home()).await;
    (plugin, broker)
}
