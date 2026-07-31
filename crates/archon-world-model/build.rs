//! Bakes the git commit into the binary so every trace row can name the exact
//! build that produced it.
//!
//! `CARGO_PKG_VERSION` alone is too coarse: it changes only on release, so an
//! entire development period collapses to one label and a corpus collected
//! across it cannot be segmented. The commit is what actually identifies the
//! behaviour that generated a trace.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let sha = git_short_sha().unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=ARCHON_BUILD_SHA={sha}");

    // Rebuild when the checked-out commit moves. HEAD alone is not enough: on a
    // branch it is a symref, so committing rewrites the ref file rather than
    // HEAD itself. Watch both.
    if let Some(git_dir) = git_dir() {
        println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
        if let Some(ref_path) = head_ref_path(&git_dir) {
            println!("cargo:rerun-if-changed={}", ref_path.display());
        }
        // Covers refs that live in packed-refs rather than as loose files.
        let packed = git_dir.join("packed-refs");
        if packed.exists() {
            println!("cargo:rerun-if-changed={}", packed.display());
        }
    }
}

fn git_short_sha() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

fn git_dir() -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--absolute-git-dir"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let dir = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!dir.is_empty()).then(|| PathBuf::from(dir))
}

/// Resolve `HEAD` to the ref file it points at, when HEAD is a symref.
fn head_ref_path(git_dir: &Path) -> Option<PathBuf> {
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let reference = head.trim().strip_prefix("ref: ")?;
    Some(git_dir.join(reference))
}
