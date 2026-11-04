use std::fmt;

pub const UNREACHABLE_MESSAGE: &str = "Can't reach Lightroom. Is Lightroom Classic running with the lrcli plugin installed? Run `lrcli plugin install`, restart Lightroom Classic, then open File > Plug-in Manager and select lrcli once (Lightroom only starts plugins when first used).";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Failed,
    Usage,
    Unreachable,
    Protocol,
    Timeout,
}

#[derive(Debug, Clone)]
pub struct Error {
    pub kind: Kind,
    pub code: String,
    pub message: String,
}

impl Error {
    pub fn new(kind: Kind, code: &str, message: impl Into<String>) -> Self {
        Self {
            kind,
            code: code.to_string(),
            message: message.into(),
        }
    }

    pub fn failed(code: &str, message: impl Into<String>) -> Self {
        Self::new(Kind::Failed, code, message)
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(Kind::Usage, "usage", message)
    }

    pub fn unreachable(code: &str) -> Self {
        Self::new(Kind::Unreachable, code, UNREACHABLE_MESSAGE)
    }

    pub fn timeout(secs: f64) -> Self {
        Self::new(
            Kind::Timeout,
            "timeout",
            format!(
                "No reply within {secs} seconds. Lightroom Classic may be busy; retry, or raise --timeout."
            ),
        )
    }

    pub fn io(code: &str, context: impl fmt::Display, source: std::io::Error) -> Self {
        Self::failed(code, format!("{context}: {source}"))
    }

    /// Maps an `ok:false` reply onto an error. Auth and protocol rejections
    /// get their own exit code so scripts can tell them from command failures.
    pub fn from_reply(code: &str, message: &str) -> Self {
        match code {
            "unauthorized" | "protocol_mismatch" => Self::new(Kind::Protocol, code, message),
            "connection_lost" => Self::unreachable(code),
            _ => Self::new(Kind::Failed, code, message),
        }
    }

    pub fn exit_code(&self) -> u8 {
        match self.kind {
            Kind::Failed => 1,
            Kind::Usage => 2,
            Kind::Unreachable => 3,
            Kind::Protocol => 4,
            Kind::Timeout => 5,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_codes_map_to_exit_codes() {
        assert_eq!(Error::from_reply("unauthorized", "x").exit_code(), 4);
        assert_eq!(Error::from_reply("protocol_mismatch", "x").exit_code(), 4);
        assert_eq!(Error::from_reply("connection_lost", "x").exit_code(), 3);
        assert_eq!(Error::from_reply("unknown_command", "x").exit_code(), 1);
    }

    #[test]
    fn connection_lost_keeps_its_code_but_uses_the_actionable_message() {
        let e = Error::from_reply("connection_lost", "link dropped");
        assert_eq!(e.code, "connection_lost");
        assert_eq!(e.message, UNREACHABLE_MESSAGE);
    }
}
