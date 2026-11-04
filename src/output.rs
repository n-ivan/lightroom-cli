use serde_json::Value;

use lrcli::error::Error;

const RESTART_HINT: &str = "Restart Lightroom Classic, then open File > Plug-in Manager and select lrcli once so the plugin starts.";

pub type Formatter = fn(&Value) -> String;

pub fn emit(result: Result<Value, Error>, json: bool, human: Formatter) -> u8 {
    match result {
        Ok(value) => {
            if json {
                println!("{value}");
            } else {
                println!("{}", human(&value));
            }
            0
        }
        Err(error) => {
            emit_error(&error, json);
            error.exit_code()
        }
    }
}

pub fn emit_error(error: &Error, json: bool) {
    if json {
        println!("{}", error_json(error));
    } else {
        eprintln!("error: {}", error.message);
    }
}

fn error_json(error: &Error) -> Value {
    serde_json::json!({ "ok": false, "error": error.code, "message": error.message })
}

fn text<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or("unknown")
}

fn number(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_i64)
        .map_or_else(|| "unknown".to_string(), |n| n.to_string())
}

pub fn status(value: &Value) -> String {
    format!(
        "Lightroom Classic {}\nPlugin:  {} (protocol {})\nCatalog: {}\nBroker:  {} (pid {})",
        text(value, "/lightroom/version"),
        text(value, "/plugin/version"),
        number(value, "/protocol"),
        text(value, "/catalog/path"),
        text(value, "/broker/version"),
        number(value, "/broker/pid"),
    )
}

pub fn plugin_install(value: &Value) -> String {
    let (source, target) = (text(value, "/source"), text(value, "/target"));
    match text(value, "/action") {
        "unchanged" => format!(
            "Already installed: {target} -> {source}\nBroker path: {}",
            text(value, "/brokerPath")
        ),
        action => {
            let verb = if action == "replaced" {
                "Replaced"
            } else {
                "Installed"
            };
            format!(
                "{verb} {target} -> {source}\nBroker path: {}\n{RESTART_HINT}",
                text(value, "/brokerPath")
            )
        }
    }
}

pub fn plugin_uninstall(value: &Value) -> String {
    let target = text(value, "/target");
    match text(value, "/action") {
        "removed" => format!("Removed {target}\nRestart Lightroom Classic to unload the plugin."),
        _ => format!("Not installed: nothing at {target}"),
    }
}

pub fn plugin_path(value: &Value) -> String {
    format!(
        "Source: {}\nTarget: {}\nConfig: {}",
        text(value, "/source"),
        text(value, "/target"),
        text(value, "/config")
    )
}

pub fn raw(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("serializing a Value cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_lists_every_field() {
        let reply = json!({
            "ok": true, "protocol": 1,
            "plugin": { "version": "0.1.0" },
            "lightroom": { "version": "15.0" },
            "catalog": { "path": "/c.lrcat" },
            "broker": { "version": "0.1.0", "pid": 42 },
        });
        assert_eq!(
            status(&reply),
            "Lightroom Classic 15.0\nPlugin:  0.1.0 (protocol 1)\nCatalog: /c.lrcat\nBroker:  0.1.0 (pid 42)"
        );
    }

    #[test]
    fn status_tolerates_missing_fields() {
        assert!(status(&json!({ "ok": true })).contains("unknown"));
    }

    #[test]
    fn errors_render_as_json_without_an_id() {
        let error = Error::failed("boom", "it broke");
        assert_eq!(
            error_json(&error),
            json!({ "ok": false, "error": "boom", "message": "it broke" })
        );
    }

    #[test]
    fn install_output_mentions_restart_only_when_something_changed() {
        let installed =
            json!({ "action": "installed", "source": "/s", "target": "/t", "brokerPath": "/b" });
        assert!(plugin_install(&installed).contains("Restart Lightroom Classic"));
        let unchanged =
            json!({ "action": "unchanged", "source": "/s", "target": "/t", "brokerPath": "/b" });
        assert!(!plugin_install(&unchanged).contains("Restart"));
    }
}
