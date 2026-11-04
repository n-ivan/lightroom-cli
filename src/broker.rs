use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tokio::time::{Instant, interval_at, sleep};

use crate::client::Client;
use crate::error::Error;
use crate::log::Log;
use crate::paths::Paths;
use crate::plugin_link::{self, Upstream};
use crate::protocol::{self, Ports, Request};

const STARTUP_WAIT: Duration = Duration::from_secs(5);
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const PING_INTERVAL: Duration = Duration::from_secs(30);
const PING_TIMEOUT: Duration = Duration::from_secs(10);
const TOKEN_POLL_INTERVAL: Duration = Duration::from_secs(2);
const PEER_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const SUPERSEDE_WAIT: Duration = Duration::from_secs(3);
const FLUSH_GRACE: Duration = Duration::from_millis(100);

const VERSION: &str = env!("CARGO_PKG_VERSION");

enum State {
    Ready(Value),
    Failed { code: String, message: String },
}

struct Shared {
    upstream: Arc<Upstream>,
    state: State,
    fingerprint: String,
    exit: mpsc::UnboundedSender<String>,
}

enum Peer {
    None,
    SameSession,
    Superseded,
}

pub async fn run(paths: &Paths) -> Result<(), Error> {
    paths.ensure_dir()?;
    let log = Log::open(&paths.log());
    let result = serve(paths, &log).await;
    match &result {
        Ok(()) => log.line("broker exited"),
        Err(e) => log.line(format_args!("broker failed: {}: {}", e.code, e.message)),
    }
    result
}

async fn serve(paths: &Paths, log: &Log) -> Result<(), Error> {
    let ports = plugin_link::read_ports(&paths.ports(), STARTUP_WAIT).await?;
    log.line(format_args!(
        "broker {VERSION} pid {} starting, plugin token {}",
        std::process::id(),
        ports.fingerprint()
    ));

    if let Peer::SameSession = replace_existing(&paths.socket(), &ports, log).await? {
        log.line("a broker for this Lightroom session is already running");
        return Ok(());
    }

    let link = plugin_link::connect(&ports, STARTUP_WAIT).await?;
    let (upstream, mut closed) = Upstream::start(link);
    let state = handshake(&upstream, &ports.token, log).await?;

    let socket = paths.socket();
    let listener = match UnixListener::bind(&socket) {
        Ok(listener) => listener,
        Err(e) if e.kind() == ErrorKind::AddrInUse => {
            log.line("another broker bound the socket first");
            return Ok(());
        }
        Err(e) => {
            return Err(Error::io(
                "socket_bind",
                format!("Cannot listen on {}", socket.display()),
                e,
            ));
        }
    };
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
        Error::io(
            "socket_bind",
            format!("Cannot restrict {}", socket.display()),
            e,
        )
    })?;

    let (exit_tx, mut exit_rx) = mpsc::unbounded_channel::<String>();
    let shared = Arc::new(Shared {
        upstream: upstream.clone(),
        state,
        fingerprint: ports.fingerprint(),
        exit: exit_tx.clone(),
    });
    tokio::spawn(accept_loop(listener, shared));
    tokio::spawn(ping_loop(upstream.clone(), exit_tx.clone()));
    tokio::spawn(watch_token(paths.ports(), ports.token.clone(), exit_tx));

    let mut sigterm = signal(SignalKind::terminate())
        .map_err(|e| Error::io("signal", "Cannot install the SIGTERM handler", e))?;
    let reason = tokio::select! {
        reason = exit_rx.recv() => reason.unwrap_or_default(),
        reason = &mut closed => format!("plugin link closed: {}", reason.unwrap_or_default()),
        _ = sigterm.recv() => "SIGTERM".to_string(),
        _ = tokio::signal::ctrl_c() => "SIGINT".to_string(),
    };
    log.line(format_args!("shutting down: {reason}"));

    upstream.close();
    // Gives handlers a moment to write connection_lost replies before the process ends.
    sleep(FLUSH_GRACE).await;
    let _ = std::fs::remove_file(&socket);
    Ok(())
}

async fn handshake(upstream: &Upstream, token: &str, log: &Log) -> Result<State, Error> {
    let args = json!({ "token": token, "protocol": protocol::PROTOCOL_VERSION, "client": protocol::client_name() });
    let reply = upstream
        .request_by("hello", args, HELLO_TIMEOUT)
        .await
        .map_err(|_| Error::failed("hello_failed", "The plugin did not answer hello."))?;
    if protocol::is_ok(&reply) {
        log.line("plugin accepted hello");
        return Ok(State::Ready(reply));
    }
    let (code, message) = protocol::error_parts(&reply);
    match code.as_str() {
        "unauthorized" | "protocol_mismatch" => {
            log.line(format_args!("plugin rejected hello: {code}"));
            Ok(State::Failed { code, message })
        }
        _ => Err(Error::failed(&code, message)),
    }
}

/// If a live broker owns the socket: leave it be when it serves this plugin
/// instance, otherwise ask it to quit and wait for the socket to go away.
async fn replace_existing(socket: &Path, ports: &Ports, log: &Log) -> Result<Peer, Error> {
    if !socket.exists() {
        return Ok(Peer::None);
    }
    let Ok(mut client) = Client::connect(socket, PEER_PROBE_TIMEOUT, PEER_PROBE_TIMEOUT).await
    else {
        log.line("removing stale broker socket");
        let _ = std::fs::remove_file(socket);
        return Ok(Peer::None);
    };
    let info = client
        .request("broker_info", json!({}))
        .await
        .map_err(|_| {
            Error::failed(
                "socket_in_use",
                format!(
                    "{} is held by a process that is not an lrcli broker.",
                    socket.display()
                ),
            )
        })?;
    if info.get("token_fingerprint").and_then(Value::as_str) == Some(ports.fingerprint().as_str()) {
        return Ok(Peer::SameSession);
    }
    log.line("asking the broker for a previous Lightroom session to quit");
    let _ = client.request("broker_shutdown", json!({})).await;
    drop(client);

    let deadline = Instant::now() + SUPERSEDE_WAIT;
    while Instant::now() < deadline {
        if !socket.exists() || UnixStream::connect(socket).await.is_err() {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    if UnixStream::connect(socket).await.is_ok() {
        return Err(Error::failed(
            "socket_in_use",
            "A broker from a previous Lightroom session did not quit. Kill it and retry.",
        ));
    }
    let _ = std::fs::remove_file(socket);
    Ok(Peer::Superseded)
}

async fn accept_loop(listener: UnixListener, shared: Arc<Shared>) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(serve_connection(shared.clone(), stream));
            }
            Err(_) => sleep(Duration::from_millis(100)).await,
        }
    }
}

async fn serve_connection(shared: Arc<Shared>, stream: UnixStream) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let (reply, shutdown) = shared.handle(&line).await;
        if writer
            .write_all(&protocol::encode_line(&reply))
            .await
            .is_err()
        {
            break;
        }
        if shutdown {
            let _ = shared.exit.send("broker_shutdown requested".to_string());
            break;
        }
    }
}

impl Shared {
    async fn handle(&self, line: &str) -> (Value, bool) {
        let request = match protocol::parse_request(line) {
            Ok(request) => request,
            Err(message) => {
                return (
                    protocol::error_reply(Value::Null, "bad_request", &message),
                    false,
                );
            }
        };
        let id = request.id;
        match request.cmd.as_str() {
            "broker_shutdown" => (protocol::ok_reply(id), true),
            "broker_info" => (self.info(id), false),
            _ => (self.handle_plugin_command(request).await, false),
        }
    }

    fn info(&self, id: i64) -> Value {
        let state = match self.state {
            State::Ready(_) => "ready",
            State::Failed { .. } => "failed",
        };
        json!({
            "id": id, "ok": true, "version": VERSION, "pid": std::process::id(),
            "token_fingerprint": self.fingerprint, "state": state,
        })
    }

    async fn handle_plugin_command(&self, request: Request) -> Value {
        let id = request.id;
        let hello = match &self.state {
            State::Failed { code, message } => {
                return protocol::error_reply(json!(id), code, message);
            }
            State::Ready(hello) => hello,
        };
        if request.cmd == "hello" {
            let mut reply = hello.clone();
            protocol::set_id(&mut reply, id);
            reply["broker"] = json!({ "version": VERSION, "pid": std::process::id() });
            return reply;
        }
        match self.upstream.request(&request.cmd, request.args).await {
            Ok(mut reply) => {
                protocol::set_id(&mut reply, id);
                reply
            }
            Err(_) => protocol::error_reply(
                json!(id),
                "connection_lost",
                "The broker lost its connection to Lightroom Classic.",
            ),
        }
    }
}

async fn ping_loop(upstream: Arc<Upstream>, exit: mpsc::UnboundedSender<String>) {
    let mut ticks = interval_at(Instant::now() + PING_INTERVAL, PING_INTERVAL);
    loop {
        ticks.tick().await;
        if upstream
            .request_by("ping", json!({}), PING_TIMEOUT)
            .await
            .is_err()
        {
            let _ = exit.send("plugin did not answer ping".to_string());
            return;
        }
    }
}

async fn watch_token(
    ports_path: std::path::PathBuf,
    token: String,
    exit: mpsc::UnboundedSender<String>,
) {
    let mut ticks = interval_at(Instant::now() + TOKEN_POLL_INTERVAL, TOKEN_POLL_INTERVAL);
    loop {
        ticks.tick().await;
        let current = std::fs::read_to_string(&ports_path)
            .ok()
            .and_then(|text| Ports::parse(&text).ok());
        if current.is_some_and(|ports| ports.token != token) {
            let _ = exit.send("ports.json has a new token".to_string());
            return;
        }
    }
}
