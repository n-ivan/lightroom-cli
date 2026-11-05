#![cfg(target_os = "macos")]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

const ARCHIVE: &str = "lrcli-macos-universal.tar.gz";

fn sh(dir: &Path, script: &str) {
    let status = Command::new("sh")
        .args(["-c", script])
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "{script}");
}

/// A release layout where the "universal" binary is just the test build.
fn fake_release(dir: &Path) {
    fs::copy(env!("CARGO_BIN_EXE_lrcli"), dir.join("lrcli")).unwrap();
    sh(
        dir,
        &format!("tar -czf {ARCHIVE} lrcli && shasum -a 256 {ARCHIVE} > {ARCHIVE}.sha256"),
    );
}

struct Sandbox {
    release: TempDir,
    install_dir: TempDir,
    home: TempDir,
    modules: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let release = TempDir::new().unwrap();
        fake_release(release.path());
        Self {
            release,
            install_dir: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            modules: TempDir::new().unwrap(),
        }
    }

    fn run(&self, base: &Path) -> Output {
        Command::new("sh")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh"))
            .env("LRCLI_DOWNLOAD_BASE", format!("file://{}", base.display()))
            .env("LRCLI_VERSION", "1.2.3")
            .env("LRCLI_INSTALL_DIR", self.install_dir.path())
            .env("LRCLI_HOME", self.home.path())
            .env("LRCLI_MODULES_DIR", self.modules.path())
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap()
    }

    fn versioned_base(&self) -> std::path::PathBuf {
        let base = self.release.path().join("base");
        let versioned = base.join("download/v1.2.3");
        fs::create_dir_all(&versioned).unwrap();
        for name in [ARCHIVE.to_string(), format!("{ARCHIVE}.sha256")] {
            fs::copy(self.release.path().join(&name), versioned.join(&name)).unwrap();
        }
        base
    }
}

#[test]
fn install_script_installs_the_binary_and_the_plugin() {
    let sandbox = Sandbox::new();
    let out = sandbox.run(&sandbox.versioned_base());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(sandbox.install_dir.path().join("lrcli").is_file());
    assert!(
        sandbox
            .modules
            .path()
            .join("lrcli.lrplugin/bin/lrcli")
            .is_file()
    );
    assert!(stdout.contains("lrcli restart"), "{stdout}");
    assert!(stdout.contains("not on your PATH"), "{stdout}");

    let again = sandbox.run(&sandbox.versioned_base());
    assert!(again.status.success());
}

#[test]
fn install_script_rejects_a_corrupt_download() {
    let sandbox = Sandbox::new();
    let base = sandbox.versioned_base();
    fs::write(base.join("download/v1.2.3").join(ARCHIVE), "tampered").unwrap();
    let out = sandbox.run(&base);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("checksum"));
    assert!(!sandbox.install_dir.path().join("lrcli").exists());
    assert!(
        fs::read_dir(sandbox.modules.path())
            .unwrap()
            .next()
            .is_none()
    );
}
