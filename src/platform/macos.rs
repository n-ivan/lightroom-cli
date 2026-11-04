use std::path::PathBuf;

use crate::error::Error;
use crate::paths::home_dir;

pub fn modules_dir() -> Result<PathBuf, Error> {
    Ok(home_dir()?.join("Library/Application Support/Adobe/Lightroom/Modules"))
}
