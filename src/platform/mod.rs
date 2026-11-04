use std::path::PathBuf;

use crate::error::Error;

#[cfg(target_os = "macos")]
mod macos;

pub fn modules_dir() -> Result<PathBuf, Error> {
    if let Some(dir) = std::env::var_os("LRCLI_MODULES_DIR").filter(|d| !d.is_empty()) {
        return Ok(dir.into());
    }
    default_modules_dir()
}

#[cfg(target_os = "macos")]
fn default_modules_dir() -> Result<PathBuf, Error> {
    macos::modules_dir()
}

#[cfg(not(target_os = "macos"))]
fn default_modules_dir() -> Result<PathBuf, Error> {
    Err(Error::usage(format!(
        "Plugin installation is not supported on {} yet; only macOS is. Copy plugin/lrcli.lrdevplugin into Lightroom's Modules directory manually.",
        std::env::consts::OS
    )))
}
