use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::Error;
use crate::paths::home_dir;

pub fn modules_dir() -> Result<PathBuf, Error> {
    Ok(home_dir()?.join("Library/Application Support/Adobe/Lightroom/Modules"))
}

const BUNDLE_ID: &str = "com.adobe.LightroomClassicCC7";

fn osascript(script: &str) -> Result<String, Error> {
    let output = Command::new("osascript")
        .args(["-e", script])
        .output()
        .map_err(|e| Error::io("app_control_failed", "Cannot run osascript", e))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(Error::failed(
            "app_control_failed",
            format!(
                "osascript failed: {}. If macOS asked to let your terminal control Lightroom Classic, allow it in System Settings > Privacy & Security > Automation.",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ))
    }
}

pub fn app_is_running() -> Result<bool, Error> {
    Ok(osascript(&format!("application id \"{BUNDLE_ID}\" is running"))? == "true")
}

pub fn launch_app(catalog: Option<&Path>) -> Result<(), Error> {
    let mut command = Command::new("open");
    command.args(["-b", BUNDLE_ID]);
    command.args(catalog);
    let status = command
        .status()
        .map_err(|e| Error::io("app_control_failed", "Cannot run open", e))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::failed(
            "app_control_failed",
            "Could not launch Lightroom Classic. Is it installed?",
        ))
    }
}

pub fn activate_app() -> Result<(), Error> {
    osascript(&format!("tell application id \"{BUNDLE_ID}\" to activate")).map(drop)
}

pub fn quit_app() -> Result<(), Error> {
    osascript(&format!("tell application id \"{BUNDLE_ID}\" to quit")).map(drop)
}
