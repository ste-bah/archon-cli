//! Content identity for verdict interpretation, independent of Git's dirty flag.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[cfg(not(test))]
pub fn build_fingerprint(root: &Path) -> String {
    let mut hash = Sha256::new();
    framed(&mut hash, source_fingerprint(root).as_bytes());
    let mut settings: Vec<_> = std::env::vars()
        .filter(|(key, _)| {
            key.starts_with("CARGO_FEATURE_")
                || key.starts_with("CARGO_CFG_TARGET_")
                || matches!(
                    key.as_str(),
                    "PROFILE" | "OPT_LEVEL" | "DEBUG" | "CARGO_CFG_DEBUG_ASSERTIONS"
                )
        })
        .collect();
    settings.sort();
    for (key, value) in settings {
        framed(&mut hash, key.as_bytes());
        framed(&mut hash, value.as_bytes());
    }
    let rustc = std::process::Command::new(std::env::var_os("RUSTC").expect("RUSTC"))
        .arg("--version")
        .output()
        .expect("compiler version");
    assert!(rustc.status.success(), "compiler version failed");
    framed(&mut hash, &rustc.stdout);
    format!("{:x}", hash.finalize())
}

pub fn source_fingerprint(root: &Path) -> String {
    let mut paths = Vec::new();
    for dir in ["src", "crates"] {
        collect(&root.join(dir), &mut paths);
    }
    for file in [
        "build.rs",
        "build_fingerprint.rs",
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        ".cargo/config.toml",
    ] {
        paths.push(root.join(file));
    }
    paths.sort();
    let mut hash = Sha256::new();
    for path in paths {
        let name = path.strip_prefix(root).expect("relative source path");
        framed(
            &mut hash,
            name.to_string_lossy().replace('\\', "/").as_bytes(),
        );
        framed(
            &mut hash,
            &std::fs::read(&path).expect("read build fingerprint input"),
        );
    }
    format!("{:x}", hash.finalize())
}

fn framed(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn collect(dir: &Path, paths: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read source directory") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            if !path
                .file_name()
                .is_some_and(|name| name == "target" || name == ".git")
            {
                collect(&path, paths);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs")
            || path.file_name().is_some_and(|name| name == "Cargo.toml")
        {
            paths.push(path);
        }
    }
}
