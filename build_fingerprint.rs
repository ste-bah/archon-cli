//! Content identity for verdict interpretation, independent of Git's dirty flag.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[cfg(not(test))]
pub fn build_fingerprint(root: &Path) -> String {
    let mut hash = Sha256::new();
    let source = source_fingerprint(root);
    let rustflags = std::env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    let profile = std::env::var("PROFILE").unwrap_or_default();
    framed(
        &mut hash,
        combine_build_fingerprint(&source, &rustflags, &profile).as_bytes(),
    );
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

/// Combine source identity with build-script compiler flags and profile.
#[allow(dead_code)]
pub fn combine_build_fingerprint(source: &str, rustflags: &str, profile: &str) -> String {
    let mut hash = Sha256::new();
    framed(&mut hash, source.as_bytes());
    framed(&mut hash, rustflags.as_bytes());
    framed(&mut hash, profile.as_bytes());
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
    paths.extend(embedded_files(root));
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

/// Files referenced by literal `include!`, `include_str!`, and
/// `include_bytes!` expressions in Rust source and build scripts.
pub fn embedded_files(root: &Path) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    for dir in ["src", "crates"] {
        collect_rust_sources(&root.join(dir), &mut sources);
    }
    sources.push(root.join("build.rs"));

    let mut embedded = Vec::new();
    for source in sources {
        let Ok(contents) = std::fs::read_to_string(&source) else {
            continue;
        };
        for macro_name in ["include_str!", "include_bytes!", "include!"] {
            for (offset, _) in contents.match_indices(macro_name) {
                let rest = contents[offset + macro_name.len()..].trim_start();
                let Some(rest) = rest.strip_prefix('(').map(str::trim_start) else {
                    continue;
                };
                let Some(target) =
                    included_path(rest, source.parent().expect("source parent"), root)
                else {
                    continue;
                };
                if target.is_file() && target.starts_with(root) {
                    embedded.push(target);
                }
            }
        }
    }
    embedded.sort();
    embedded.dedup();
    embedded
}

fn included_path(expression: &str, source_parent: &Path, root: &Path) -> Option<PathBuf> {
    if let Some((literal, _)) = rust_string_literal(expression) {
        return Some(source_parent.join(literal));
    }

    let manifest_dir = "env!(\"CARGO_MANIFEST_DIR\")";
    let expression = expression.strip_prefix("concat!(")?;
    let suffix = expression.strip_prefix(manifest_dir)?.trim_start();
    let suffix = suffix.strip_prefix(',')?.trim_start();
    let (literal, _) = rust_string_literal(suffix)?;
    Some(root.join(literal.trim_start_matches('/')))
}

fn rust_string_literal(source: &str) -> Option<(String, usize)> {
    let mut chars = source.char_indices();
    if chars.next()?.1 != '"' {
        return None;
    }
    let mut value = String::new();
    let mut escaped = false;
    for (index, character) in chars {
        if escaped {
            value.push(match character {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            });
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            return Some((value, index + character.len_utf8()));
        } else {
            value.push(character);
        }
    }
    None
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

fn collect_rust_sources(dir: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !path
                .file_name()
                .is_some_and(|name| name == "target" || name == ".git")
            {
                collect_rust_sources(&path, paths);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            paths.push(path);
        }
    }
}
