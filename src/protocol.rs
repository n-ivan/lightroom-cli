use serde::Deserialize;
use serde_json::{Map, Value, json};

pub const PROTOCOL_VERSION: i64 = 1;

pub fn client_name() -> String {
    format!("lrcli/{}", env!("CARGO_PKG_VERSION"))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ports {
    pub version: u32,
    pub command_port: u16,
    pub reply_port: u16,
    pub token: String,
}

impl Ports {
    pub fn parse(text: &str) -> Result<Self, String> {
        let ports: Ports = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if ports.version != 1 {
            return Err(format!("unsupported ports.json version {}", ports.version));
        }
        Ok(ports)
    }

    pub fn fingerprint(&self) -> String {
        fingerprint(&self.token)
    }
}

pub fn fingerprint(token: &str) -> String {
    token.chars().take(8).collect()
}

#[derive(Debug, PartialEq)]
pub struct Request {
    pub id: i64,
    pub cmd: String,
    pub args: Value,
}

pub fn request(id: i64, cmd: &str, args: Value) -> Value {
    json!({ "id": id, "cmd": cmd, "args": args })
}

pub fn parse_request(line: &str) -> Result<Request, String> {
    let value: Value =
        serde_json::from_str(line).map_err(|e| format!("Request is not valid JSON ({e})"))?;
    let object = value
        .as_object()
        .ok_or("Request is not a JSON object. Send one object per line with integer \"id\" and string \"cmd\".")?;
    let id = object
        .get("id")
        .and_then(Value::as_i64)
        .ok_or("Missing or invalid \"id\". It must be an integer.")?;
    let cmd = object
        .get("cmd")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
        .ok_or("Missing or invalid \"cmd\". It must be a non-empty string.")?;
    let args = match object.get("args") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => return Err("\"args\" must be a JSON object when present.".into()),
    };
    Ok(Request {
        id,
        cmd: cmd.to_string(),
        args,
    })
}

pub fn error_reply(id: Value, code: &str, message: &str) -> Value {
    json!({ "id": id, "ok": false, "error": code, "message": message })
}

pub fn ok_reply(id: i64) -> Value {
    json!({ "id": id, "ok": true })
}

pub fn encode_line(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).expect("serializing a Value cannot fail");
    bytes.push(b'\n');
    bytes
}

pub fn reply_id(reply: &Value) -> Option<i64> {
    reply.get("id").and_then(Value::as_i64)
}

pub fn set_id(reply: &mut Value, id: i64) {
    if let Some(object) = reply.as_object_mut() {
        object.insert("id".into(), json!(id));
    }
}

pub fn strip_id(mut reply: Value) -> Value {
    if let Some(object) = reply.as_object_mut() {
        object.remove("id");
    }
    reply
}

pub fn is_ok(reply: &Value) -> bool {
    reply.get("ok").and_then(Value::as_bool) == Some(true)
}

pub fn error_parts(reply: &Value) -> (String, String) {
    let text = |key: &str, default: &str| {
        reply
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or(default)
            .to_string()
    };
    (
        text("error", "unknown_error"),
        text("message", "The request failed without a message."),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_request_and_defaults_args() {
        let r = parse_request(r#"{"id":7,"cmd":"ping"}"#).unwrap();
        assert_eq!(
            r,
            Request {
                id: 7,
                cmd: "ping".into(),
                args: json!({})
            }
        );
    }

    #[test]
    fn rejects_malformed_requests() {
        assert!(parse_request("nope").is_err());
        assert!(parse_request("[1]").is_err());
        assert!(parse_request(r#"{"cmd":"x"}"#).is_err());
        assert!(parse_request(r#"{"id":1.5,"cmd":"x"}"#).is_err());
        assert!(parse_request(r#"{"id":1,"cmd":""}"#).is_err());
        assert!(parse_request(r#"{"id":1,"cmd":"x","args":[]}"#).is_err());
    }

    #[test]
    fn lines_are_newline_terminated_compact_json() {
        assert_eq!(encode_line(&json!({"a":1})), b"{\"a\":1}\n");
    }

    #[test]
    fn id_helpers_remap_and_strip() {
        let mut reply = json!({"id": 99, "ok": true});
        assert_eq!(reply_id(&reply), Some(99));
        set_id(&mut reply, 3);
        assert_eq!(reply_id(&reply), Some(3));
        assert_eq!(strip_id(reply), json!({"ok": true}));
    }

    #[test]
    fn ports_file_parses_and_fingerprints() {
        let text = r#"{"version":1,"commandPort":1,"replyPort":2,"token":"0123456789abcdef","pluginVersion":"0.1.0","updatedAt":5}"#;
        let ports = Ports::parse(text).unwrap();
        assert_eq!((ports.command_port, ports.reply_port), (1, 2));
        assert_eq!(ports.fingerprint(), "01234567");
        assert!(
            Ports::parse(r#"{"version":2,"commandPort":1,"replyPort":2,"token":"x"}"#).is_err()
        );
        assert!(Ports::parse("{").is_err());
    }
}
