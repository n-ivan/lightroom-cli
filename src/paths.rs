use std::fs::DirBuilder;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;

use crate::error::Error;

#[derive(Debug, Clone)]
pub struct Paths {
    dir: PathBuf,
}

pub fn home_dir() -> Result<PathBuf, Error> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| Error::failed("no_home", "HOME is not set. Set HOME or LRCLI_HOME."))
}

impl Paths {
    pub fn from_env() -> Result<Self, Error> {
        match std::env::var_os("LRCLI_HOME").filter(|h| !h.is_empty()) {
            Some(dir) => Ok(Self::at(dir.into())),
            None => Ok(Self::at(home_dir()?.join(".lrcli"))),
        }
    }

    pub fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &PathBuf {
        &self.dir
    }

    pub fn ports(&self) -> PathBuf {
        self.dir.join("ports.json")
    }

    pub fn socket(&self) -> PathBuf {
        self.dir.join("broker.sock")
    }

    pub fn log(&self) -> PathBuf {
        self.dir.join("broker.log")
    }

    pub fn plugin_log(&self) -> PathBuf {
        self.dir.join("plugin.log")
    }

    pub fn history(&self) -> PathBuf {
        self.dir.join("history.jsonl")
    }

    pub fn config(&self) -> PathBuf {
        self.dir.join("config.json")
    }

    pub fn ensure_dir(&self) -> Result<(), Error> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)
            .map_err(|e| {
                Error::io(
                    "io_error",
                    format!("Cannot create {}", self.dir.display()),
                    e,
                )
            })
    }
}
