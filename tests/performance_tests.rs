//! Performance regression tests for Archon CLI.
//!
//! These tests verify that key performance characteristics remain
//! within acceptable bounds.

use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

/// Resolve the compiled binary via `CARGO_BIN_EXE_archon`.
///
/// Cargo always sets this for an integration test in a package that declares a
/// `[[bin]]`, and this package declares `archon`. It used to be treated as
/// optional — `let Some(bin) = archon_bin() else { eprintln!("skipping"); return }`
/// in both tests below — which turned a renamed binary, a moved test, or a
/// runner that does not set the variable into a silent pass. A test that reports
/// success without running is worse than one that is absent, so the absence is
/// now a failure that names what went missing.
fn archon_bin() -> PathBuf {
    let raw = std::env::var_os("CARGO_BIN_EXE_archon").unwrap_or_else(|| {
        panic!(
            "CARGO_BIN_EXE_archon is unset. Cargo sets it for every integration test \
             in a package with a `[[bin]]`; if it is missing, this test is not \
             measuring the archon binary and must not report success."
        )
    });
    PathBuf::from(raw)
}

/// Verify that `archon --version` completes within 500ms.
///
/// The PRD target is 200ms, but CI runners can be significantly
/// slower, so we use a generous 500ms bound.
#[test]
fn startup_under_500ms() {
    let bin = archon_bin();

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
/// so we use different thresholds: 100 MB for release, 650 MB for debug.
#[test]
fn binary_size_check() {
    let bin = archon_bin();

    let meta = std::fs::metadata(&bin).expect("failed to stat archon binary");
    let size_mb = meta.len() as f64 / (1024.0 * 1024.0);

    // Normalise separators before matching. `contains("target/release")` never
    // matched on Windows, where the path is `target\release`, so every Windows
    // run — CI included — checked a release binary against the 650MB debug
    // ceiling and would have passed a 6x regression without noticing.
    let normalised = bin.to_string_lossy().replace('\\', "/");
    let is_release = normalised.contains("/release/");
    let limit_mb = if is_release { 100.0 } else { 650.0 };
    let label = if is_release { "release" } else { "debug" };

    assert!(
        size_mb < limit_mb,
        "{label} binary is {size_mb:.1}MB, expected < {limit_mb}MB",
    );
}
