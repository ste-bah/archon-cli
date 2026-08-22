//! Regression guard — prohibit tokio Mutex blocking_lock/blocking_read/
//! blocking_write outside of explicitly approved sites.
//!
//! Why this is an allowlist, not a blanket ban:
//! Some `blocking_*` uses ARE legitimate — specifically when the call
//! is made from a guaranteed non-async context. Examples that may be
//! legitimately allowlisted:
//!   - inside a `tokio::task::spawn_blocking(|| { ... })` closure
//!   - inside a function that is ONLY called from `fn main` (sync) or
//!     from a dedicated OS thread
//!   - inside a Drop impl that runs on shutdown (no async context)
//!
//! NEVER add a site to ALLOWLIST as a workaround for a panic. Fix the
//! panic. Allowlist is for legitimate sync-context uses only.
//!
//! ## Why this no longer shells out to `grep`
//!
//! It used to run `grep -rn --include=*.rs ... crates/ src/` as a subprocess.
//! Both of those are *relative* paths, and a Cargo integration-test binary runs
//! with its working directory set to the package root — here
//! `crates/archon-core`. So `crates/` did not exist, and `src/` meant
//! `crates/archon-core/src`. The scan looked at zero matching lines anywhere in
//! the workspace and reported success, every run, on every platform, for as long
//! as the guard had existed.
//!
//! Measured: appending
//! `fn _probe(m: &tokio::sync::Mutex<u8>) { let _g = m.blocking_lock(); }` to the
//! workspace's own `src/main.rs` — exactly the thing this guard forbids — left
//! the test reporting `ok` in 0.10s. A guard that cannot see a violation planted
//! in the file it is supposed to be watching is not a guard.
//!
//! Two things changed. The scan is now pure Rust rooted at the workspace via
//! `CARGO_MANIFEST_DIR`, so it depends neither on the process working directory
//! nor on `grep` being installed (it is not, on a default Windows box). And it
//! declares what it inspected: a scan that reaches no files, or that cannot find
//! the pattern anywhere in a tree that demonstrably contains it, fails instead of
//! passing quietly. That is the vacuity rule `scripts/lint/arch-lint.sh` applies
//! to its own rules, and the guard shape `tests/no_hardcoded_tmp_path_gate.rs`
//! already uses.

use std::path::{Path, PathBuf};

/// Sites where a `blocking_*` call is legitimate. Each entry is a substring
/// matched against `path:line`.
const ALLOWLIST: &[&str] = &[
    // Example shape (do NOT add without proof):
    // "crates/foo/src/bar.rs:42",  // Proof: only called from spawn_blocking at baz.rs:10
];

const PATTERNS: &[&str] = &["blocking_lock", "blocking_read", "blocking_write"];

/// The workspace root, resolved from this crate rather than from the process
/// working directory — which for an integration-test binary is the *package*
/// root, not the workspace root.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            // Build output and vendored trees are not this workspace's source.
            if matches!(name.as_ref(), "target" | "node_modules" | ".git") {
                continue;
            }
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_blocking_lock_outside_allowlist() {
    let root = workspace_root();

    let mut files = Vec::new();
    for scan_root in ["crates", "src"] {
        collect_rs_files(&root.join(scan_root), &mut files);
    }
    files.sort();

    // Vacuity guard 1: the scan roots must exist and hold sources. A walk that
    // finds nothing is the exact failure this test was rewritten to stop.
    assert!(
        files.len() > 100,
        "walk found only {} .rs files under {}/{{crates,src}} — the scan roots are \
         wrong, and a guard that inspects nothing must never report success",
        files.len(),
        root.display()
    );

    let mut raw_hits = 0usize;
    let mut offenders: Vec<String> = Vec::new();

    for path in &files {
        let Ok(body) = std::fs::read_to_string(path) else {
            continue;
        };
        // Forward slashes so the filters below read the same on both platforms.
        let relative = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        for (index, line) in body.lines().enumerate() {
            if !PATTERNS.iter().any(|pattern| line.contains(pattern)) {
                continue;
            }
            raw_hits += 1;

            let site = format!("{relative}:{}", index + 1);

            // Tests run on their own runtimes and are free to use blocking_*.
            if relative.contains("/tests/") || relative.ends_with("_tests.rs") {
                continue;
            }
            // Doc comments and ordinary comments are prose, not calls.
            if line.trim_start().starts_with("//") {
                continue;
            }
            if ALLOWLIST.iter().any(|allowed| site.contains(allowed)) {
                continue;
            }
            offenders.push(format!("{site}: {}", line.trim()));
        }
    }

    // Vacuity guard 2: this very file names all three patterns above, so a scan
    // that reached the workspace cannot come back with zero raw hits. Zero means
    // the matcher stopped working, not that the tree is clean.
    assert!(
        raw_hits > 0,
        "scanned {} files and matched none of {PATTERNS:?} anywhere — including in \
         this test's own source, which contains all three. The scan is broken.",
        files.len()
    );

    assert!(
        offenders.is_empty(),
        "Found tokio blocking_lock/read/write outside allowlist \
         ({} file(s) scanned, {raw_hits} raw hit(s)):\n{}\n\n\
         These will panic from async context. Either:\n  \
         1. Convert the call to .await (preferred for async paths), OR\n  \
         2. Move the call to a non-async context (spawn_blocking, \
            dedicated OS thread), OR\n  \
         3. Swap the mutex type to std::sync::Mutex if no .await is \
            held inside the critical section, OR\n  \
         4. (LAST RESORT) Add the site to ALLOWLIST with a `// Proof:` \
            comment naming the sync-context guarantee.\n",
        files.len(),
        offenders.join("\n")
    );
}
