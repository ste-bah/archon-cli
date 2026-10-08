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
    //
    // HEAD is per worktree (`--absolute-git-dir`), but branch refs and
    // packed-refs live in the common dir (`--git-common-dir`). In a normal
    // checkout the two are the same `.git`. Never watch a path that does not
    // exist: Cargo treats a missing path as always changed, which reruns this
    // script and relinks every dependent binary on every build.
    if let Some(git_dir) = git_dir() {
        println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
        let common_dir = git_common_dir().unwrap_or_else(|| git_dir.clone());
        if let Some(ref_path) = head_ref_path(&git_dir, &common_dir) {
            if ref_path.exists() {
                println!("cargo:rerun-if-changed={}", ref_path.display());
            } else if let Some(parent) = existing_ancestor(&ref_path, &common_dir) {
                // The branch is only in packed-refs. The next commit writes the
                // loose ref file, which shows as a change in this directory.
                println!("cargo:rerun-if-changed={}", parent.display());
            }
        }
        // Covers refs that live in packed-refs rather than as loose files.
        let packed = common_dir.join("packed-refs");
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

/// The repository's common dir (shared by all worktrees), as an absolute path.
fn git_common_dir() -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let dir = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if dir.is_empty() {
        return None;
    }
    let dir = PathBuf::from(dir);
    // Older git prints this relative to the current directory (e.g. `.git`).
    if dir.is_absolute() {
        Some(dir)
    } else {
        Some(std::env::current_dir().ok()?.join(dir))
    }
}

/// Resolve `HEAD` (read from the per-worktree git dir) to the ref file it
/// points at in the common dir, when HEAD is a symref. `None` when detached.
fn head_ref_path(git_dir: &Path, common_dir: &Path) -> Option<PathBuf> {
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let reference = head.trim().strip_prefix("ref: ")?;
    Some(common_dir.join(reference))
}

/// The nearest existing directory above `path`, not above `stop`.
fn existing_ancestor(path: &Path, stop: &Path) -> Option<PathBuf> {
    let mut dir = path.parent()?;
    while !dir.is_dir() {
        if dir == stop {
            return None;
        }
        dir = dir.parent()?;
    }
    Some(dir.to_path_buf())
}
