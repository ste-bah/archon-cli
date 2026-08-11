//! Probes, at build time, whether this host will let us create a symbolic link,
//! and exposes the answer as `cfg(archon_can_symlink)`.
//!
//! `capability.rs` has two tests that build a symlink escape and assert the
//! checker refuses to follow it. Creating the symlink is *setup*, and on Windows
//! it needs `SeCreateSymbolicLinkPrivilege`, which an ordinary account does not
//! hold unless Developer Mode is on. Without it the setup fails with
//! `ERROR_PRIVILEGE_NOT_HELD` (1314) and the assertion never runs.
//!
//! The obvious alternative — detect that at runtime and `return` — is worse than
//! useless: Rust's harness classifies a test purely on panic/no-panic, so an
//! early return is reported as a PASS. A test that verified nothing would then be
//! indistinguishable from one that verified everything, which is the failure mode
//! this codebase keeps having to dig out of. Rust has no runtime skip (the 2021
//! "Skippable tests" pre-RFC never landed), so the decision has to be made before
//! the harness sees the test at all. That means build time.
//!
//! `#[cfg_attr(not(archon_can_symlink), ignore = "...")]` is the result: on a host
//! without the privilege the tests are *ignored, with the reason attached*, under
//! both `cargo test` and `cargo nextest`, with no runner-side coordination. On a
//! host that has it, nothing changes and the tests run exactly as before.
//!
//! The probe attempts the real operation. It does not infer from `cfg!(windows)`,
//! from Developer Mode registry keys, or from an environment variable, because
//! none of those is the thing the test needs: a machine can be a privileged
//! Windows one, and a Linux one can still refuse on an exotic filesystem. The only
//! honest question is "does `symlink` return `Ok`", so that is the question asked,
//! using the same call the tests use.
//!
//! Caveat, deliberately not worked around: Cargo caches build-script output, so
//! granting the privilege after a build does not re-probe on its own. Touching
//! `ARCHON_RECHECK_SYMLINK_PRIVILEGE` forces it, which is cheaper and more
//! predictable than making the script rerun on every invocation.

use std::path::{Path, PathBuf};

fn main() {
    // Declared so `#[cfg(archon_can_symlink)]` is a known name rather than an
    // `unexpected_cfgs` warning on modern rustc.
    println!("cargo:rustc-check-cfg=cfg(archon_can_symlink)");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=ARCHON_RECHECK_SYMLINK_PRIVILEGE");

    if host_can_create_symlinks() {
        println!("cargo:rustc-cfg=archon_can_symlink");
        return;
    }

    // `cargo test` prints "ignored, <reason>" and needs nothing more. nextest is
    // what CI runs, and it only reports a count of "skipped" — libtest's `--list`
    // output does not carry the reason, so nextest cannot show it (its JSON says
    // `"ignored": true` and nothing else). Without this line a CI reader sees two
    // tests vanish with no explanation anywhere in the log.
    println!(
        "cargo:warning=symlink creation is not permitted for this build, so the two \
         capability symlink-escape tests are marked #[ignore] and will be reported as \
         skipped. On Windows this means SeCreateSymbolicLinkPrivilege is not held: turn \
         on Developer Mode or build from an elevated shell to exercise them."
    );
}

/// Create a directory symlink in a scratch directory, then remove it.
///
/// Returns `false` for *any* failure, not just a privilege refusal. A probe that
/// tried to classify the error would be guessing; the tests need the operation to
/// work, and "it did not work" is the whole answer. A false negative costs two
/// ignored tests, which is visible in the output. A false positive costs a red
/// build on a host that was never going to pass.
fn host_can_create_symlinks() -> bool {
    let Some(scratch) = scratch_dir() else {
        return false;
    };

    let target = scratch.join("target");
    let link = scratch.join("link");
    let created = std::fs::create_dir_all(&target).is_ok() && symlink_dir(&target, &link).is_ok();

    // Remove the link before the tree, so a host that *did* get a symlink does
    // not have it traversed on the way out.
    let _ = remove_symlink_dir(&link);
    let _ = std::fs::remove_dir_all(&scratch);

    created
}

/// A private, uniquely named directory under `OUT_DIR`.
///
/// `OUT_DIR` rather than `std::env::temp_dir()`: Cargo guarantees it exists and is
/// writable for this package, and it is the directory a build script is supposed
/// to scribble in. The process id keeps concurrent builds of the workspace from
/// colliding on the same probe path.
fn scratch_dir() -> Option<PathBuf> {
    let out_dir = std::env::var_os("OUT_DIR")?;
    let dir = PathBuf::from(out_dir).join(format!("symlink-probe-{}", std::process::id()));
    // A leftover from an interrupted build would make `create_dir_all` succeed
    // while `symlink` then fails with AlreadyExists, reporting a privileged host
    // as unprivileged.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// `symlink_dir`, not `symlink_file`: it is the call the tests make, and on
/// Windows the two are different APIs with different flags.
#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

/// Removing a directory symlink is the one place the two platforms disagree:
/// Unix unlinks it as a file (`remove_dir` gives `ENOTDIR`), Windows removes the
/// reparse point as a directory (`remove_file` gives "access denied"). Getting
/// this wrong only leaves litter in `OUT_DIR`, but it leaves it every build.
#[cfg(unix)]
fn remove_symlink_dir(link: &Path) -> std::io::Result<()> {
    std::fs::remove_file(link)
}

#[cfg(windows)]
fn remove_symlink_dir(link: &Path) -> std::io::Result<()> {
    std::fs::remove_dir(link)
}
