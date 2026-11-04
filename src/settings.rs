use serde_json::{Map, Number, Value, json};

use crate::error::Error;

#[derive(Debug, Clone, PartialEq)]
pub enum Setting {
    Set(String, Value),
    Add(String, f64),
}

impl Setting {
    pub fn key(&self) -> &str {
        match self {
            Setting::Set(key, _) | Setting::Add(key, _) => key,
        }
    }
}

fn number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        json!(value as i64)
    } else {
        Number::from_f64(value).map_or(Value::Null, Value::Number)
    }
}

fn parse_value(raw: &str) -> Value {
    if let Ok(whole) = raw.parse::<i64>() {
        return json!(whole);
    }
    if let Ok(float) = raw.parse::<f64>()
        && float.is_finite()
    {
        return number(float);
    }
    match raw {
        "true" => json!(true),
        "false" => json!(false),
        _ => json!(raw),
    }
}

/// Accepts `Key=Value` (number, true/false or string), `Key+=N` and `Key-=N`.
pub fn parse_setting(text: &str) -> Result<Setting, String> {
    let malformed = || {
        format!(
            "invalid setting \"{text}\". Use Key=Value, Key+=N or Key-=N, for example Exposure2012+=0.5, Contrast2012=15 or WhiteBalance=Auto"
        )
    };
    let (head, raw) = text.split_once('=').ok_or_else(malformed)?;
    let (key, sign) = if let Some(key) = head.strip_suffix('+') {
        (key, Some(1.0))
    } else if let Some(key) = head.strip_suffix('-') {
        (key, Some(-1.0))
    } else {
        (head, None)
    };
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(malformed());
    }
    match sign {
        None if raw.is_empty() => Err(malformed()),
        None => Ok(Setting::Set(key.to_string(), parse_value(raw))),
        Some(sign) => match raw.parse::<f64>() {
            Ok(delta) if delta.is_finite() => Ok(Setting::Add(key.to_string(), sign * delta)),
            _ => Err(format!(
                "invalid setting \"{text}\": the amount after += or -= must be a number"
            )),
        },
    }
}

pub type Args = (Map<String, Value>, Map<String, Value>);

/// Splits settings into the `set` and `add` objects the plugin expects.
pub fn to_args(settings: &[Setting]) -> Result<Args, Error> {
    let (mut set, mut add) = (Map::new(), Map::new());
    for (index, setting) in settings.iter().enumerate() {
        if settings[..index].iter().any(|s| s.key() == setting.key()) {
            return Err(Error::usage(format!(
                "{} is given more than once. Give each setting once.",
                setting.key()
            )));
        }
        match setting {
            Setting::Set(key, value) => set.insert(key.clone(), value.clone()),
            Setting::Add(key, delta) => add.insert(key.clone(), number(*delta)),
        };
    }
    Ok((set, add))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(key: &str, value: Value) -> Setting {
        Setting::Set(key.into(), value)
    }

    #[test]
    fn absolute_values() {
        assert_eq!(
            parse_setting("Contrast2012=15"),
            Ok(set("Contrast2012", json!(15)))
        );
        assert_eq!(
            parse_setting("Exposure2012=-1"),
            Ok(set("Exposure2012", json!(-1)))
        );
        assert_eq!(
            parse_setting("Exposure2012=0.25"),
            Ok(set("Exposure2012", json!(0.25)))
        );
        assert_eq!(
            parse_setting("ConvertToGrayscale=true"),
            Ok(set("ConvertToGrayscale", json!(true)))
        );
        assert_eq!(
            parse_setting("EnableDetail=false"),
            Ok(set("EnableDetail", json!(false)))
        );
        assert_eq!(
            parse_setting("WhiteBalance=Auto"),
            Ok(set("WhiteBalance", json!("Auto")))
        );
        assert_eq!(
            parse_setting("WhiteBalance=As Shot"),
            Ok(set("WhiteBalance", json!("As Shot")))
        );
        assert_eq!(parse_setting("Tint=nan"), Ok(set("Tint", json!("nan"))));
    }

    #[test]
    fn relative_values() {
        assert_eq!(
            parse_setting("Exposure2012+=0.5"),
            Ok(Setting::Add("Exposure2012".into(), 0.5))
        );
        assert_eq!(
            parse_setting("Key-=0.3"),
            Ok(Setting::Add("Key".into(), -0.3))
        );
        assert_eq!(
            parse_setting("Key+=-2"),
            Ok(Setting::Add("Key".into(), -2.0))
        );
    }

    #[test]
    fn malformed_settings() {
        for text in [
            "",
            "Exposure2012",
            "=1",
            "Key=",
            "Key+=",
            "Key+=abc",
            "Key-=inf",
            "+=1",
            "Bad Key=1",
            "A.B=1",
        ] {
            assert!(parse_setting(text).is_err(), "{text}");
        }
    }

    #[test]
    fn args_split_set_and_add() {
        let settings = [
            parse_setting("A=1").unwrap(),
            parse_setting("B-=2").unwrap(),
            parse_setting("C=x").unwrap(),
        ];
        let (set, add) = to_args(&settings).unwrap();
        assert_eq!(Value::Object(set), json!({ "A": 1, "C": "x" }));
        assert_eq!(Value::Object(add), json!({ "B": -2 }));
    }

    #[test]
    fn repeated_keys_are_usage_errors() {
        let settings = [
            parse_setting("A=1").unwrap(),
            parse_setting("A+=2").unwrap(),
        ];
        assert_eq!(to_args(&settings).unwrap_err().exit_code(), 2);
    }
}
