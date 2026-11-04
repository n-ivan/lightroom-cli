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

pub fn launch_app(catalog: Option<&Path>, background: bool) -> Result<(), Error> {
    let mut command = Command::new("open");
    if background {
        command.arg("-g");
    }
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

pub fn frontmost_app() -> Option<String> {
    osascript("tell application \"System Events\" to get bundle identifier of first process whose frontmost is true")
        .ok()
        .filter(|id| !id.is_empty() && id != BUNDLE_ID)
}

// Lightroom only runs plugin init once its window has been activated, even
// with LrForceInitPlugin; a brief activation is enough, so focus goes back.
pub fn blip_app(return_to: Option<&str>) -> Result<(), Error> {
    activate_app()?;
    if let Some(id) = return_to {
        std::thread::sleep(std::time::Duration::from_millis(200));
        let _ = osascript(&format!("tell application id \"{id}\" to activate"));
    }
    Ok(())
}

pub fn quit_app() -> Result<(), Error> {
    osascript(&format!("tell application id \"{BUNDLE_ID}\" to quit"))
        .map(drop)
        .map_err(|e| {
            // -128 is how an app declines a quit request, e.g. while a modal window is open.
            if e.message.contains("(-128)") {
                Error::failed(
                    "quit_refused",
                    "Lightroom Classic refused to quit. Close any open dialog or window in Lightroom (for example Plug-in Manager) and retry.",
                )
            } else {
                e
            }
        })
}
