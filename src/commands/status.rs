use serde_json::{Value, json};

use super::Context;
use crate::error::Error;
use crate::protocol;

pub async fn run(ctx: &Context) -> Result<Value, Error> {
    let mut client = ctx.connect().await?;
    client
        .request(
            "hello",
            json!({ "protocol": protocol::PROTOCOL_VERSION, "client": protocol::client_name() }),
        )
        .await
}
