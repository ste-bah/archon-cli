//! TC-ARCH-05 (REQ-FOR-D3): grep lint for .send().await on agent event producer.
//!
//! Validates that the agent event channel is unbounded: no `.send().await`
//! on `event_tx` in agent.rs or main.rs. Unbounded channels use synchronous
//! `.send()` which never blocks the producer.

use std::process::Command;

#[test]
fn no_send_await_on_event_tx() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

    // Direct grep check — same pattern as arch-lint.sh Rule 2
    let pattern = r"event_tx\.send\([^)]*\)\.await";
    let paths = [
        repo_root.join("crates/archon-core/src/agent.rs"),
        repo_root.join("src/main.rs"),
    ];

    for path in &paths {
        let content = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("TC-ARCH-05: failed to read {path:?}: {e}"));

        let re = regex::Regex::new(pattern).expect("invalid regex");
        let matches: Vec<_> = content
            .lines()
            .enumerate()
            .filter(|(_, line)| re.is_match(line))
            .collect();

        assert!(
            matches.is_empty(),
            "TC-ARCH-05: found .send().await on event_tx in {path:?}: {matches:?}"
        );
    }
}

#[test]
fn arch_lint_rule2_passes() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let lint_script = repo_root.join("scripts/lint/arch-lint.sh");

    let output = Command::new("bash")
        .arg(&lint_script)
        .current_dir(repo_root)
        .output()
        .expect("failed to execute arch-lint.sh");

    assert!(
        output.status.success(),
        "TC-ARCH-05: arch-lint.sh failed (Rule 2 or other): {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
