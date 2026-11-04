use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::oneshot;
use tokio::time::{Instant, sleep, timeout_at};

use crate::error::Error;
use crate::protocol::{self, Ports};

const RETRY_INTERVAL: Duration = Duration::from_millis(100);

pub async fn read_ports(path: &Path, wait: Duration) -> Result<Ports, Error> {
    let deadline = Instant::now() + wait;
    loop {
        let problem = match std::fs::read_to_string(path) {
            Ok(text) => match Ports::parse(&text) {
                Ok(ports) => return Ok(ports),
                Err(e) => e,
            },
            Err(e) => e.to_string(),
        };
        if Instant::now() >= deadline {
            return Err(Error::failed(
                "ports_unreadable",
                format!(
                    "Cannot read {}: {problem}. Is the lrcli plugin running in Lightroom Classic?",
                    path.display()
                ),
            ));
        }
        sleep(RETRY_INTERVAL).await;
    }
}

pub struct Link {
    pub replies: OwnedReadHalf,
    pub commands: OwnedWriteHalf,
    // Dropping a write half sends FIN, and LrC's send-mode socket handles a
    // half-closed peer badly, so this stays open for the life of the link.
    pub reply_writer: OwnedWriteHalf,
}

async fn connect_port(port: u16, deadline: Instant) -> Result<TcpStream, Error> {
    loop {
        match TcpStream::connect(("127.0.0.1", port)).await {
            Ok(stream) => return Ok(stream),
            Err(e) if e.kind() == ErrorKind::ConnectionRefused && Instant::now() < deadline => {
                sleep(RETRY_INTERVAL).await;
            }
            Err(e) => {
                return Err(Error::io(
                    "plugin_unreachable",
                    format!("Cannot connect to the plugin on port {port}"),
                    e,
                ));
            }
        }
    }
}

/// The plugin queues replies until its reply socket has a client, so the
/// reply socket must be connected before the command socket.
pub async fn connect(ports: &Ports, wait: Duration) -> Result<Link, Error> {
    let deadline = Instant::now() + wait;
    let replies = connect_port(ports.reply_port, deadline).await?;
    let commands = connect_port(ports.command_port, deadline).await?;
    let (replies, reply_writer) = replies.into_split();
    Ok(Link {
        replies,
        commands: commands.into_split().1,
        reply_writer,
    })
}

#[derive(Debug)]
pub struct LinkClosed;

type Pending = HashMap<i64, oneshot::Sender<Value>>;

/// Multiplexes requests from many callers onto the single plugin connection,
/// assigning its own ids and routing replies back by id.
pub struct Upstream {
    commands: tokio::sync::Mutex<OwnedWriteHalf>,
    _reply_writer: OwnedWriteHalf,
    pending: Mutex<Option<Pending>>,
    next_id: AtomicI64,
}

impl Upstream {
    /// The receiver resolves with a reason once the reply socket ends.
    pub fn start(link: Link) -> (Arc<Self>, oneshot::Receiver<String>) {
        let upstream = Arc::new(Self {
            commands: tokio::sync::Mutex::new(link.commands),
            _reply_writer: link.reply_writer,
            pending: Mutex::new(Some(HashMap::new())),
            next_id: AtomicI64::new(1),
        });
        let (closed_tx, closed_rx) = oneshot::channel();
        let reader = upstream.clone();
        tokio::spawn(async move {
            let reason = reader.read_replies(link.replies).await;
            reader.close();
            let _ = closed_tx.send(reason);
        });
        (upstream, closed_rx)
    }

    async fn read_replies(&self, stream: OwnedReadHalf) -> String {
        let mut lines = BufReader::new(stream).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => self.deliver(&line),
                Ok(None) => return "reply socket closed".into(),
                Err(e) => return format!("reply socket error: {e}"),
            }
        }
    }

    fn deliver(&self, line: &str) {
        let Ok(reply) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let Some(id) = protocol::reply_id(&reply) else {
            return;
        };
        let sender = self
            .pending
            .lock()
            .unwrap()
            .as_mut()
            .and_then(|p| p.remove(&id));
        if let Some(sender) = sender {
            let _ = sender.send(reply);
        }
    }

    /// Fails every in-flight and future request with `LinkClosed`.
    pub fn close(&self) {
        self.pending.lock().unwrap().take();
    }

    pub async fn request(&self, cmd: &str, args: Value) -> Result<Value, LinkClosed> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        match self.pending.lock().unwrap().as_mut() {
            Some(pending) => pending.insert(id, tx),
            None => return Err(LinkClosed),
        };
        let line = protocol::encode_line(&protocol::request(id, cmd, args));
        let written = {
            let mut commands = self.commands.lock().await;
            match commands.write_all(&line).await {
                Ok(()) => commands.flush().await,
                Err(e) => Err(e),
            }
        };
        if written.is_err() {
            if let Some(pending) = self.pending.lock().unwrap().as_mut() {
                pending.remove(&id);
            }
            return Err(LinkClosed);
        }
        rx.await.map_err(|_| LinkClosed)
    }

    pub async fn request_by(
        &self,
        cmd: &str,
        args: Value,
        wait: Duration,
    ) -> Result<Value, LinkClosed> {
        timeout_at(Instant::now() + wait, self.request(cmd, args))
            .await
            .unwrap_or(Err(LinkClosed))
    }
}
