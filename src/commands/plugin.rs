use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::error::Error;
use crate::paths::Paths;
use crate::platform;
use crate::plugin_files;

pub const DEV_DIR_NAME: &str = "lrcli.lrdevplugin";
pub const RELEASE_DIR_NAME: &str = "lrcli.lrplugin";
const TOOLKIT_MARKER: &str = "LrToolkitIdentifier = 'dev.lrcli.bridge'";

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
    let packaged = exe_dir.join("../share/lrcli").join(DEV_DIR_NAME);
    if packaged.is_dir() {
        return fs::canonicalize(packaged).ok();
    }
    exe_dir
        .ancestors()
        .map(|dir| dir.join("plugin").join(DEV_DIR_NAME))
        .find(|candidate| candidate.is_dir())
        .and_then(|found| fs::canonicalize(found).ok())
}

fn require_source(exe: &Path) -> Result<PathBuf, Error> {
    find_source(exe).ok_or_else(|| {
        Error::failed(
            "plugin_source_missing",
            format!(
                "Cannot find {DEV_DIR_NAME} next to {}. Run lrcli from the repository checkout, or reinstall it so the plugin directory is present.",
                exe.display()
            ),
        )
    })
}

fn dev_target() -> Result<PathBuf, Error> {
    Ok(platform::modules_dir()?.join(DEV_DIR_NAME))
}

fn release_target() -> Result<PathBuf, Error> {
    Ok(platform::modules_dir()?.join(RELEASE_DIR_NAME))
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

#[derive(Debug, PartialEq)]
enum Ownership {
    Absent,
    Ours,
    Foreign,
}

fn classify(path: &Path) -> Result<Ownership, Error> {
    let Some(meta) = lstat(path)? else {
        return Ok(Ownership::Absent);
    };
    let dev_link = meta.is_symlink() && is_plugin_link(path);
    let has_marker =
        fs::read_to_string(path.join("Info.lua")).is_ok_and(|info| info.contains(TOOLKIT_MARKER));
    Ok(if dev_link || has_marker {
        Ownership::Ours
    } else {
        Ownership::Foreign
    })
}

fn remove_path(path: &Path) -> Result<(), Error> {
    let meta = fs::symlink_metadata(path)
        .map_err(|e| Error::io("io_error", format!("Cannot inspect {}", path.display()), e))?;
    let removed = if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    removed.map_err(|e| Error::io("io_error", format!("Cannot remove {}", path.display()), e))
}

fn remove_quietly(path: &Path) {
    let _ = remove_path(path);
}

fn foreign_target(target: &Path) -> Error {
    Error::failed(
        "plugin_target_exists",
        format!(
            "{} already exists and does not look like the lrcli plugin. Re-run with --force to replace it, or remove it yourself.",
            target.display()
        ),
    )
}

/// Removes the other install flavour when it is ours: two plugins with the same
/// toolkit id would conflict in Lightroom. Anything not ours is left alone.
fn remove_other(other: &Path) -> Result<Option<PathBuf>, Error> {
    if classify(other)? == Ownership::Ours {
        remove_path(other)?;
        return Ok(Some(other.to_path_buf()));
    }
    Ok(None)
}

pub struct InstallPlan<'a> {
    pub source: &'a Path,
    pub target: &'a Path,
    pub config: &'a Path,
    pub broker: &'a Path,
}

pub fn install_dev_at(plan: &InstallPlan, force: bool) -> Result<&'static str, Error> {
    let InstallPlan {
        source,
        target,
        config,
        broker,
    } = *plan;
    let action = match lstat(target)? {
        None => "installed",
        Some(meta) if meta.is_symlink() && links_to(target, source) => "unchanged",
        Some(_) => {
            if !force && classify(target)? == Ownership::Foreign {
                return Err(foreign_target(target));
            }
            remove_path(target)?;
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

fn write_staged(stage: &Path, exe: &Path) -> Result<(), Error> {
    let io = |what: &str, path: &Path| {
        let message = format!("Cannot {what} {}", path.display());
        move |e| Error::io("io_error", &message, e)
    };
    for (relative, bytes) in plugin_files::FILES {
        let path = stage.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io("create", parent))?;
        }
        fs::write(&path, bytes).map_err(io("write", &path))?;
    }
    let bin = stage.join("bin");
    fs::create_dir_all(&bin).map_err(io("create", &bin))?;
    let broker = bin.join("lrcli");
    fs::copy(exe, &broker).map_err(io("copy lrcli to", &broker))?;
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o755))
        .map_err(io("make executable", &broker))
}

/// Stages next to `target` and swaps by rename, so Lightroom never sees a
/// half-written plugin. The staging names lack the `.lrplugin` suffix.
pub fn install_release_at(target: &Path, exe: &Path, force: bool) -> Result<&'static str, Error> {
    let existing = classify(target)?;
    if existing == Ownership::Foreign && !force {
        return Err(foreign_target(target));
    }
    let parent = target
        .parent()
        .ok_or_else(|| Error::failed("io_error", format!("{} has no parent", target.display())))?;
    fs::create_dir_all(parent)
        .map_err(|e| Error::io("io_error", format!("Cannot create {}", parent.display()), e))?;

    let pid = std::process::id();
    let stage = parent.join(format!(".lrcli-stage-{pid}"));
    let old = parent.join(format!(".lrcli-old-{pid}"));
    remove_quietly(&stage);
    remove_quietly(&old);
    if let Err(e) = write_staged(&stage, exe) {
        remove_quietly(&stage);
        return Err(e);
    }
    let replacing = existing != Ownership::Absent;
    if replacing && let Err(e) = fs::rename(target, &old) {
        remove_quietly(&stage);
        return Err(Error::io(
            "io_error",
            format!("Cannot move {} aside", target.display()),
            e,
        ));
    }
    if let Err(e) = fs::rename(&stage, target) {
        if replacing {
            let _ = fs::rename(&old, target);
        }
        remove_quietly(&stage);
        return Err(Error::io(
            "io_error",
            format!("Cannot move the new plugin into {}", target.display()),
            e,
        ));
    }
    remove_quietly(&old);
    Ok(if replacing { "upgraded" } else { "installed" })
}

pub fn uninstall_at(release: &Path, dev: &Path, config: &Path) -> Result<Vec<PathBuf>, Error> {
    for target in [release, dev] {
        if classify(target)? == Ownership::Foreign {
            return Err(Error::failed(
                "plugin_target_not_ours",
                format!(
                    "{} is not the lrcli plugin, so it was left alone. Remove it manually if you are sure.",
                    target.display()
                ),
            ));
        }
    }
    let mut removed = Vec::new();
    for target in [release, dev] {
        removed.extend(remove_other(target)?);
    }
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
    Ok(removed)
}

fn is_plugin_link(target: &Path) -> bool {
    fs::read_link(target).is_ok_and(|dest| dest.file_name().is_some_and(|n| n == DEV_DIR_NAME))
}

pub fn install(paths: &Paths, force: bool, dev: bool) -> Result<Value, Error> {
    let exe = current_exe()?;
    if dev {
        install_dev(paths, &exe, force)
    } else {
        install_release(&exe, force)
    }
}

fn install_dev(paths: &Paths, exe: &Path, force: bool) -> Result<Value, Error> {
    let source = require_source(exe)?;
    let target = dev_target()?;
    paths.ensure_dir()?;
    let config = paths.config();
    let action = install_dev_at(
        &InstallPlan {
            source: &source,
            target: &target,
            config: &config,
            broker: exe,
        },
        force,
    )?;
    let removed = remove_other(&release_target()?)?;
    Ok(json!({
        "ok": true, "mode": "dev", "action": action, "source": source,
        "target": target, "config": config, "brokerPath": exe, "removed": removed,
    }))
}

fn install_release(exe: &Path, force: bool) -> Result<Value, Error> {
    let target = release_target()?;
    let action = install_release_at(&target, exe, force)?;
    let removed = remove_other(&dev_target()?)?;
    Ok(json!({
        "ok": true, "mode": "release", "action": action, "target": target,
        "brokerPath": target.join("bin/lrcli"), "removed": removed,
    }))
}

pub fn uninstall(paths: &Paths) -> Result<Value, Error> {
    let config = paths.config();
    let removed = uninstall_at(&release_target()?, &dev_target()?, &config)?;
    Ok(json!({ "ok": true, "removed": removed, "config": config }))
}

fn configured_broker(config: &Path) -> Option<String> {
    let body: Value = serde_json::from_slice(&fs::read(config).ok()?).ok()?;
    body.get("brokerPath")?.as_str().map(str::to_string)
}

pub fn path(paths: &Paths) -> Result<Value, Error> {
    let release = release_target()?;
    let dev = dev_target()?;
    let config = paths.config();
    let (mode, target, broker) = if lstat(&release)?.is_some() {
        let broker = release.join("bin/lrcli");
        ("release", Some(release), broker.exists().then_some(broker))
    } else if lstat(&dev)?.is_some() {
        (
            "dev",
            Some(dev),
            configured_broker(&config).map(PathBuf::from),
        )
    } else {
        ("none", None, None)
    };
    Ok(json!({
        "ok": true, "installed": mode, "target": target, "brokerPath": broker,
        "config": config,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    struct Dirs {
        _dir: tempfile::TempDir,
        modules: PathBuf,
        exe: PathBuf,
        config: PathBuf,
    }

    impl Dirs {
        fn new() -> Self {
            let dir = tempdir().unwrap();
            let modules = dir.path().join("modules");
            let exe = dir.path().join("fake-lrcli");
            fs::write(&exe, "binary").unwrap();
            let config = dir.path().join("config.json");
            Self {
                modules,
                exe,
                config,
                _dir: dir,
            }
        }

        fn release(&self) -> PathBuf {
            self.modules.join(RELEASE_DIR_NAME)
        }

        fn dev(&self) -> PathBuf {
            self.modules.join(DEV_DIR_NAME)
        }

        fn checkout(&self) -> PathBuf {
            let source = self._dir.path().join("src").join(DEV_DIR_NAME);
            fs::create_dir_all(&source).unwrap();
            source
        }
    }

    #[test]
    fn find_source_prefers_the_packaged_layout() {
        let dir = tempdir().unwrap();
        let packaged = dir.path().join("share/lrcli").join(DEV_DIR_NAME);
        let checkout = dir.path().join("plugin").join(DEV_DIR_NAME);
        fs::create_dir_all(&packaged).unwrap();
        fs::create_dir_all(&checkout).unwrap();
        fs::create_dir_all(dir.path().join("bin")).unwrap();
        let found = find_source(&dir.path().join("bin/lrcli")).unwrap();
        assert_eq!(found, fs::canonicalize(packaged).unwrap());
    }

    #[test]
    fn find_source_walks_up_to_a_checkout() {
        let dir = tempdir().unwrap();
        let checkout = dir.path().join("plugin").join(DEV_DIR_NAME);
        fs::create_dir_all(&checkout).unwrap();
        fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        let found = find_source(&dir.path().join("target/debug/lrcli")).unwrap();
        assert_eq!(found, fs::canonicalize(checkout).unwrap());
        assert!(find_source(Path::new("/nonexistent/bin/lrcli")).is_none());
    }

    #[test]
    fn release_install_writes_files_and_binary_then_upgrades_in_place() {
        let d = Dirs::new();
        let target = d.release();
        assert_eq!(
            install_release_at(&target, &d.exe, false).unwrap(),
            "installed"
        );
        assert!(target.join("Info.lua").is_file());
        let bin = target.join("bin/lrcli");
        assert_eq!(fs::read(&bin).unwrap(), b"binary");
        assert_eq!(
            fs::metadata(&bin).unwrap().permissions().mode() & 0o777,
            0o755
        );

        fs::write(target.join("stale.lua"), "x").unwrap();
        assert_eq!(
            install_release_at(&target, &d.exe, false).unwrap(),
            "upgraded"
        );
        assert!(!target.join("stale.lua").exists());
        let leftovers: Vec<_> = fs::read_dir(&d.modules).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "staging dirs must not linger");
    }

    #[test]
    fn release_install_refuses_foreign_dirs_without_force() {
        let d = Dirs::new();
        let target = d.release();
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("Info.lua"), "LrToolkitIdentifier = 'other'").unwrap();
        assert_eq!(
            install_release_at(&target, &d.exe, false).unwrap_err().code,
            "plugin_target_exists"
        );
        assert!(!target.join("bin").exists());
        assert_eq!(
            install_release_at(&target, &d.exe, true).unwrap(),
            "upgraded"
        );
        assert!(target.join("bin/lrcli").exists());
    }

    #[test]
    fn dev_install_is_idempotent_and_writes_config() {
        let d = Dirs::new();
        let source = d.checkout();
        let target = d.dev();
        let plan = InstallPlan {
            source: &source,
            target: &target,
            config: &d.config,
            broker: Path::new("/opt/lrcli"),
        };
        assert_eq!(install_dev_at(&plan, false).unwrap(), "installed");
        assert_eq!(install_dev_at(&plan, false).unwrap(), "unchanged");
        let written: Value = serde_json::from_slice(&fs::read(&d.config).unwrap()).unwrap();
        assert_eq!(written, json!({ "version": 1, "brokerPath": "/opt/lrcli" }));
    }

    #[test]
    fn dev_install_refuses_foreign_targets_without_force() {
        let d = Dirs::new();
        let source = d.checkout();
        let target = d.dev();
        fs::create_dir_all(&target).unwrap();
        let plan = InstallPlan {
            source: &source,
            target: &target,
            config: &d.config,
            broker: Path::new("/x"),
        };
        assert_eq!(
            install_dev_at(&plan, false).unwrap_err().code,
            "plugin_target_exists"
        );
        assert!(!d.config.exists());
        assert_eq!(install_dev_at(&plan, true).unwrap(), "replaced");
        assert!(fs::symlink_metadata(&target).unwrap().is_symlink());
    }

    #[test]
    fn uninstall_removes_both_flavours_but_never_foreign_dirs() {
        let d = Dirs::new();
        assert!(
            uninstall_at(&d.release(), &d.dev(), &d.config)
                .unwrap()
                .is_empty()
        );
        install_release_at(&d.release(), &d.exe, false).unwrap();
        let source = d.checkout();
        std::os::unix::fs::symlink(&source, d.dev()).unwrap();
        fs::write(&d.config, "{}").unwrap();
        let removed = uninstall_at(&d.release(), &d.dev(), &d.config).unwrap();
        assert_eq!(removed, vec![d.release(), d.dev()]);
        assert!(!d.config.exists());

        fs::create_dir_all(d.release()).unwrap();
        assert_eq!(
            uninstall_at(&d.release(), &d.dev(), &d.config)
                .unwrap_err()
                .code,
            "plugin_target_not_ours"
        );
        assert!(d.release().is_dir());
    }
}
