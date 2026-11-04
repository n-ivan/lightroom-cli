use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::time::timeout;

use crate::error::Error;
use crate::protocol;

/// A connection to the local broker. Requests are strictly sequential.
pub struct Client {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    next_id: i64,
    timeout: Duration,
}

impl Client {
    pub async fn connect(
        socket: &Path,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, Error> {
        let stream = match timeout(connect_timeout, UnixStream::connect(socket)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(_)) | Err(_) => return Err(Error::unreachable("unreachable")),
        };
        let (reader, writer) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(reader),
            writer,
            next_id: 1,
            timeout: request_timeout,
        })
    }

    /// Returns the reply without its `id`; `ok:false` replies become errors.
    pub async fn request(&mut self, cmd: &str, args: Value) -> Result<Value, Error> {
        match timeout(self.timeout, self.exchange(cmd, args)).await {
            Ok(result) => result,
            Err(_) => Err(Error::timeout(self.timeout.as_secs_f64())),
        }
    }

    async fn exchange(&mut self, cmd: &str, args: Value) -> Result<Value, Error> {
        let id = self.next_id;
        self.next_id += 1;
        let line = protocol::encode_line(&protocol::request(id, cmd, args));
        self.writer
            .write_all(&line)
            .await
            .map_err(|_| Error::unreachable("connection_lost"))?;
        let mut reply_line = String::new();
        let read = self
            .reader
            .read_line(&mut reply_line)
            .await
            .map_err(|_| Error::unreachable("connection_lost"))?;
        if read == 0 {
            return Err(Error::unreachable("connection_lost"));
        }
        let reply: Value = serde_json::from_str(&reply_line).map_err(|e| {
            Error::failed(
                "bad_reply",
                format!("The broker sent an unreadable reply: {e}"),
            )
        })?;
        if protocol::is_ok(&reply) {
            Ok(protocol::strip_id(reply))
        } else {
            let (code, message) = protocol::error_parts(&reply);
            Err(Error::from_reply(&code, &message))
        }
    }
}
