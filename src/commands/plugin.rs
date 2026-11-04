use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::error::Error;
use crate::paths::Paths;
use crate::platform;

pub const PLUGIN_DIR_NAME: &str = "lrcli.lrdevplugin";

fn current_exe() -> Result<PathBuf, Error> {
    std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|e| {
            Error::io(
                "no_exe_path",
                "Cannot determine the lrcli executable path",
                e,
            )
        })
}

/// Packaged layout first, then a source checkout (target/<profile>/lrcli).
pub fn find_source(exe: &Path) -> Option<PathBuf> {
    let exe_dir = exe.parent()?;
    let packaged = exe_dir.join("../share/lrcli").join(PLUGIN_DIR_NAME);
    if packaged.is_dir() {
        return fs::canonicalize(packaged).ok();
    }
    exe_dir
        .ancestors()
        .map(|dir| dir.join("plugin").join(PLUGIN_DIR_NAME))
        .find(|candidate| candidate.is_dir())
        .and_then(|found| fs::canonicalize(found).ok())
}

fn require_source(exe: &Path) -> Result<PathBuf, Error> {
    find_source(exe).ok_or_else(|| {
        Error::failed(
            "plugin_source_missing",
            format!(
                "Cannot find {PLUGIN_DIR_NAME} next to {}. Run lrcli from the repository checkout, or reinstall it so the plugin directory is present.",
                exe.display()
            ),
        )
    })
}

fn target_path() -> Result<PathBuf, Error> {
    Ok(platform::modules_dir()?.join(PLUGIN_DIR_NAME))
}

fn lstat(path: &Path) -> Result<Option<fs::Metadata>, Error> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(
            "io_error",
            format!("Cannot inspect {}", path.display()),
            e,
        )),
    }
}

fn links_to(target: &Path, source: &Path) -> bool {
    match (fs::canonicalize(target), fs::canonicalize(source)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

pub struct InstallPlan<'a> {
    pub source: &'a Path,
    pub target: &'a Path,
    pub config: &'a Path,
    pub broker: &'a Path,
}

pub fn install_at(plan: &InstallPlan, force: bool) -> Result<&'static str, Error> {
    let InstallPlan {
        source,
        target,
        config,
        broker,
    } = *plan;
    let action = match lstat(target)? {
        None => "installed",
        Some(meta) if meta.is_symlink() && links_to(target, source) => "unchanged",
        Some(_) if !force => {
            return Err(Error::failed(
                "plugin_target_exists",
                format!(
                    "{} already exists and is not the lrcli plugin symlink. Re-run with --force to replace it, or remove it yourself.",
                    target.display()
                ),
            ));
        }
        Some(meta) => {
            let removed = if meta.is_dir() {
                fs::remove_dir_all(target)
            } else {
                fs::remove_file(target)
            };
            removed.map_err(|e| {
                Error::io("io_error", format!("Cannot remove {}", target.display()), e)
            })?;
            "replaced"
        }
    };
    if action != "unchanged" {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                Error::io("io_error", format!("Cannot create {}", parent.display()), e)
            })?;
        }
        std::os::unix::fs::symlink(source, target)
            .map_err(|e| Error::io("io_error", format!("Cannot link {}", target.display()), e))?;
    }
    let body = serde_json::to_vec_pretty(&json!({ "version": 1, "brokerPath": broker }))
        .expect("serializing a Value cannot fail");
    fs::write(config, body)
        .map_err(|e| Error::io("io_error", format!("Cannot write {}", config.display()), e))?;
    Ok(action)
}

pub fn uninstall_at(target: &Path, config: &Path) -> Result<&'static str, Error> {
    let action = match lstat(target)? {
        None => "absent",
        Some(meta) if meta.is_symlink() && is_plugin_link(target) => {
            fs::remove_file(target).map_err(|e| {
                Error::io("io_error", format!("Cannot remove {}", target.display()), e)
            })?;
            "removed"
        }
        Some(_) => {
            return Err(Error::failed(
                "plugin_target_not_ours",
                format!(
                    "{} is not a symlink to the lrcli plugin, so it was left alone. Remove it manually if you are sure.",
                    target.display()
                ),
            ));
        }
    };
    match fs::remove_file(config) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => {
            return Err(Error::io(
                "io_error",
                format!("Cannot remove {}", config.display()),
                e,
            ));
        }
    }
    Ok(action)
}

fn is_plugin_link(target: &Path) -> bool {
    fs::read_link(target).is_ok_and(|dest| dest.file_name().is_some_and(|n| n == PLUGIN_DIR_NAME))
}

pub fn install(paths: &Paths, force: bool) -> Result<Value, Error> {
    let exe = current_exe()?;
    let source = require_source(&exe)?;
    let target = target_path()?;
    paths.ensure_dir()?;
    let config = paths.config();
    let action = install_at(
        &InstallPlan {
            source: &source,
            target: &target,
            config: &config,
            broker: &exe,
        },
        force,
    )?;
    Ok(json!({
        "ok": true, "action": action, "source": source, "target": target,
        "config": config, "brokerPath": exe,
    }))
}

pub fn uninstall(paths: &Paths) -> Result<Value, Error> {
    let target = target_path()?;
    let config = paths.config();
    let action = uninstall_at(&target, &config)?;
    Ok(json!({ "ok": true, "action": action, "target": target, "config": config }))
}

pub fn path(paths: &Paths) -> Result<Value, Error> {
    let source = require_source(&current_exe()?)?;
    Ok(json!({ "ok": true, "source": source, "target": target_path()?, "config": paths.config() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn plan_dirs() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let dir = tempdir().unwrap();
        let source = dir.path().join("src").join(PLUGIN_DIR_NAME);
        fs::create_dir_all(&source).unwrap();
        let target = dir.path().join("modules").join(PLUGIN_DIR_NAME);
        let config = dir.path().join("config.json");
        (dir, source, target, config)
    }

    #[test]
    fn find_source_prefers_the_packaged_layout() {
        let dir = tempdir().unwrap();
        let packaged = dir.path().join("share/lrcli").join(PLUGIN_DIR_NAME);
        let checkout = dir.path().join("plugin").join(PLUGIN_DIR_NAME);
        fs::create_dir_all(&packaged).unwrap();
        fs::create_dir_all(&checkout).unwrap();
        fs::create_dir_all(dir.path().join("bin")).unwrap();
        let found = find_source(&dir.path().join("bin/lrcli")).unwrap();
        assert_eq!(found, fs::canonicalize(packaged).unwrap());
    }

    #[test]
    fn find_source_walks_up_to_a_checkout() {
        let dir = tempdir().unwrap();
        let checkout = dir.path().join("plugin").join(PLUGIN_DIR_NAME);
        fs::create_dir_all(&checkout).unwrap();
        fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        let found = find_source(&dir.path().join("target/debug/lrcli")).unwrap();
        assert_eq!(found, fs::canonicalize(checkout).unwrap());
        assert!(find_source(Path::new("/nonexistent/bin/lrcli")).is_none());
    }

    #[test]
    fn install_is_idempotent_and_writes_config() {
        let (_dir, source, target, config) = plan_dirs();
        let broker = Path::new("/opt/lrcli");
        let plan = InstallPlan {
            source: &source,
            target: &target,
            config: &config,
            broker,
        };
        assert_eq!(install_at(&plan, false).unwrap(), "installed");
        assert_eq!(install_at(&plan, false).unwrap(), "unchanged");
        let written: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(written, json!({ "version": 1, "brokerPath": "/opt/lrcli" }));
    }

    #[test]
    fn install_refuses_foreign_targets_without_force() {
        let (_dir, source, target, config) = plan_dirs();
        fs::create_dir_all(&target).unwrap();
        let plan = InstallPlan {
            source: &source,
            target: &target,
            config: &config,
            broker: Path::new("/x"),
        };
        assert_eq!(
            install_at(&plan, false).unwrap_err().code,
            "plugin_target_exists"
        );
        assert!(!config.exists());
        assert_eq!(install_at(&plan, true).unwrap(), "replaced");
        assert!(fs::symlink_metadata(&target).unwrap().is_symlink());
    }

    #[test]
    fn uninstall_only_removes_our_symlink() {
        let (_dir, source, target, config) = plan_dirs();
        assert_eq!(uninstall_at(&target, &config).unwrap(), "absent");
        let plan = InstallPlan {
            source: &source,
            target: &target,
            config: &config,
            broker: Path::new("/x"),
        };
        install_at(&plan, false).unwrap();
        assert_eq!(uninstall_at(&target, &config).unwrap(), "removed");
        assert!(!target.exists() && !config.exists());
        fs::create_dir_all(&target).unwrap();
        assert_eq!(
            uninstall_at(&target, &config).unwrap_err().code,
            "plugin_target_not_ours"
        );
    }
}
