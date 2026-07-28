//! TC-ARCH-05: Agent event emission must await bounded capacity.

use std::process::Command;

#[test]
fn agent_event_send_is_awaited() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let events = std::fs::read_to_string(repo_root.join("crates/archon-core/src/agent/events.rs"))
        .expect("read Agent events source");

    assert!(
        events.contains("self.event_tx.send(timestamped).await"),
        "Agent event send must await bounded channel capacity"
    );
}

#[test]
fn arch_lint_passes() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new("bash")
        .arg(repo_root.join("scripts/lint/arch-lint.sh"))
        .current_dir(repo_root)
        .output()
        .expect("execute arch-lint.sh");

    assert!(
        output.status.success(),
        "arch-lint.sh failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
