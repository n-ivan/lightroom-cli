use serde_json::{Value, json};

use super::Context;
use crate::error::Error;

pub async fn run(ctx: &Context, cmd: &str, args: Option<&str>) -> Result<Value, Error> {
    let args = match args {
        Some(text) => serde_json::from_str::<Value>(text)
            .ok()
            .filter(Value::is_object)
            .ok_or_else(|| {
                Error::usage("ARGS must be a JSON object, for example '{\"key\":\"value\"}'.")
            })?,
        None => json!({}),
    };
    ctx.connect().await?.request(cmd, args).await
}
