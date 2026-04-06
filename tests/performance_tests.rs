//! Performance regression tests for Archon CLI.
//!
//! These tests verify that key performance characteristics remain
//! within acceptable bounds.

use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

/// Resolve the compiled binary via `CARGO_BIN_EXE_archon` (set by Cargo
/// for integration tests that depend on a `[[bin]]` in the same package).
fn archon_bin() -> Option<PathBuf> {
    std::env::var_os("CARGO_BIN_EXE_archon").map(PathBuf::from)
}

/// Verify that `archon --version` completes within 500ms.
///
/// The PRD target is 200ms, but CI runners can be significantly
/// slower, so we use a generous 500ms bound.
#[test]
fn startup_under_500ms() {
    let Some(bin) = archon_bin() else {
        eprintln!("skipping: CARGO_BIN_EXE_archon not set");
        return;
    };

    let start = Instant::now();
    let output = Command::new(&bin)
        .arg("--version")
        .output()
        .expect("failed to execute archon binary");
    let elapsed = start.elapsed();

    assert!(
        output.status.success(),
        "archon --version failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        elapsed.as_millis() < 500,
        "startup took {}ms, expected < 500ms",
        elapsed.as_millis(),
    );
}

/// Verify that the binary stays within a reasonable size.
///
/// Debug builds are much larger than release builds due to debug info,
/// so we use different thresholds: 100 MB for release, 500 MB for debug.
#[test]
fn binary_size_check() {
    let Some(bin) = archon_bin() else {
        eprintln!("skipping: CARGO_BIN_EXE_archon not set");
        return;
    };

    let meta = std::fs::metadata(&bin).expect("failed to stat archon binary");
    let size_mb = meta.len() as f64 / (1024.0 * 1024.0);

    let is_release = bin
        .to_string_lossy()
        .contains("target/release");
    let limit_mb = if is_release { 100.0 } else { 500.0 };
    let label = if is_release { "release" } else { "debug" };

    assert!(
        size_mb < limit_mb,
        "{label} binary is {size_mb:.1}MB, expected < {limit_mb}MB",
    );
}
