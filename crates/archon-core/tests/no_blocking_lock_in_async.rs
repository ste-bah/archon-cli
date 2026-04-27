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

#[test]
fn no_blocking_lock_outside_allowlist() {
    use std::process::Command;
    let output = Command::new("grep")
        .args([
            "-rn",
            "--include=*.rs",
            "blocking_lock\\|blocking_read\\|blocking_write",
            "crates/",
            "src/",
        ])
        .output()
        .expect("grep");
    let stdout = String::from_utf8_lossy(&output.stdout);

    const ALLOWLIST: &[&str] = &[
        // Example shape (do NOT add without proof):
        // "crates/foo/src/bar.rs:42",  // Proof: only called from spawn_blocking at baz.rs:10
    ];

    let offenders: Vec<&str> = stdout
        .lines()
        // Exclude test code — tests run on their own runtimes and are
        // free to use blocking_* as needed.
        .filter(|line| !line.contains("/tests/"))
        .filter(|line| !line.contains("#[cfg(test)]"))
        .filter(|line| !line.contains("mod tests"))
        // Exclude doc-comments (lines starting with //! or ///).
        .filter(|line| {
            let trimmed = line.split(':').nth(2).unwrap_or("").trim_start();
            !trimmed.starts_with("//")
        })
        // Exclude allowlisted sites.
        .filter(|line| !ALLOWLIST.iter().any(|allowed| line.contains(allowed)))
        .collect();

    assert!(
        offenders.is_empty(),
        "Found tokio blocking_lock/read/write outside allowlist:\n{}\n\n\
         These will panic from async context. Either:\n  \
         1. Convert the call to .await (preferred for async paths), OR\n  \
         2. Move the call to a non-async context (spawn_blocking, \
            dedicated OS thread), OR\n  \
         3. Swap the mutex type to std::sync::Mutex if no .await is \
            held inside the critical section, OR\n  \
         4. (LAST RESORT) Add the site to ALLOWLIST with a `// Proof:` \
            comment naming the sync-context guarantee.\n",
        offenders.join("\n")
    );
}
