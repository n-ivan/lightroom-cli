use std::fs;
use std::path::{Path, PathBuf};

const PLUGIN_DIR: &str = "plugin/lrcli.lrdevplugin";

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let hidden = path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'));
        if hidden {
            continue;
        }
        if path.is_dir() {
            collect(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let plugin = root.join(PLUGIN_DIR);
    println!("cargo:rerun-if-changed={PLUGIN_DIR}");

    let mut files = Vec::new();
    collect(&plugin, &mut files);

    let mut source = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    for file in files {
        let relative = file.strip_prefix(&plugin).unwrap().to_string_lossy();
        source.push_str(&format!(
            "    ({relative:?}, include_bytes!({:?})),\n",
            file.to_string_lossy()
        ));
    }
    source.push_str("];\n");

    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("plugin_files.rs");
    fs::write(out, source).unwrap();
}
