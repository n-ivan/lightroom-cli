use std::path::{Path, PathBuf};

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

/// `LRCLI_FAKE_APP=running|stopped` replaces every OS call below so tests
/// never touch the real Lightroom.
fn fake_app() -> Option<bool> {
    match std::env::var("LRCLI_FAKE_APP").ok()?.as_str() {
        "running" => Some(true),
        "stopped" => Some(false),
        _ => None,
    }
}

pub fn app_is_running() -> Result<bool, Error> {
    match fake_app() {
        Some(running) => Ok(running),
        None => real_app_is_running(),
    }
}

pub fn launch_app(catalog: Option<&Path>, background: bool) -> Result<(), Error> {
    match fake_app() {
        Some(_) => Ok(()),
        None => real_launch_app(catalog, background),
    }
}

pub fn frontmost_app() -> Option<String> {
    match fake_app() {
        Some(_) => None,
        None => real_frontmost_app(),
    }
}

pub fn blip_app(return_to: Option<&str>) -> Result<(), Error> {
    match fake_app() {
        Some(_) => Ok(()),
        None => real_blip_app(return_to),
    }
}

pub fn activate_app() -> Result<(), Error> {
    match fake_app() {
        Some(_) => Ok(()),
        None => real_activate_app(),
    }
}

pub fn activate_app_for_init(foreground: bool, return_to: Option<&str>) -> Result<(), Error> {
    if foreground {
        activate_app()
    } else {
        blip_app(return_to)
    }
}

pub fn quit_app() -> Result<(), Error> {
    match fake_app() {
        Some(_) => Ok(()),
        None => real_quit_app(),
    }
}

#[cfg(target_os = "macos")]
use macos::{
    activate_app as real_activate_app, app_is_running as real_app_is_running,
    blip_app as real_blip_app, frontmost_app as real_frontmost_app, launch_app as real_launch_app,
    quit_app as real_quit_app,
};

#[cfg(not(target_os = "macos"))]
fn unsupported() -> Error {
    Error::usage(format!(
        "Starting and stopping Lightroom Classic is not supported on {} yet; only macOS is.",
        std::env::consts::OS
    ))
}

#[cfg(not(target_os = "macos"))]
fn real_app_is_running() -> Result<bool, Error> {
    Err(unsupported())
}

#[cfg(not(target_os = "macos"))]
fn real_launch_app(_catalog: Option<&Path>, _background: bool) -> Result<(), Error> {
    Err(unsupported())
}

#[cfg(not(target_os = "macos"))]
fn real_frontmost_app() -> Option<String> {
    None
}

#[cfg(not(target_os = "macos"))]
fn real_blip_app(_return_to: Option<&str>) -> Result<(), Error> {
    Err(unsupported())
}

#[cfg(not(target_os = "macos"))]
fn real_activate_app() -> Result<(), Error> {
    Err(unsupported())
}

#[cfg(not(target_os = "macos"))]
fn real_quit_app() -> Result<(), Error> {
    Err(unsupported())
}
