use serde_json::{Value, json};

use super::Context;
use crate::error::Error;
use crate::protocol;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The plugin version when it differs from this CLI's. The broker is spawned
/// from the plugin's own `bin/` copy, so the plugin's version is the broker's.
pub fn mismatched_plugin_version(hello: &Value) -> Option<&str> {
    hello
        .pointer("/plugin/version")
        .and_then(Value::as_str)
        .filter(|plugin| *plugin != VERSION)
}

pub fn mismatch_warning(plugin_version: &str) -> String {
    format!(
        "lrcli {VERSION} talks to plugin {plugin_version}; run `lrcli plugin install && lrcli restart`"
    )
}

pub async fn hello(ctx: &Context) -> Result<Value, Error> {
    let mut client = ctx.connect().await?;
    client
        .request(
            "hello",
            json!({ "protocol": protocol::PROTOCOL_VERSION, "client": protocol::client_name() }),
        )
        .await
}

pub async fn run(ctx: &Context) -> Result<Value, Error> {
    let mut reply = hello(ctx).await?;
    if mismatched_plugin_version(&reply).is_some() {
        reply["versionMismatch"] = json!(true);
    }
    Ok(reply)
}
