#![cfg(unix)]
use archon_core::hooks::{HookConfig, HookEvent, HookRegistry};
use std::time::Duration;

struct PidGuard(i32);
impl Drop for PidGuard {
    fn drop(&mut self) {
        unsafe {
            libc::kill(self.0, libc::SIGKILL);
        }
    }
}
async fn redirected_descendant(policy: &str, parent_code: u8, count: usize) {
    let dir = tempfile::tempdir().unwrap();
    let command = format!(
        "for n in $(seq 1 {count}); do sleep 600 </dev/null >/dev/null 2>&1 & echo $! >> descendants; done; exit {parent_code}"
    );
    let config: HookConfig = serde_json::from_value(serde_json::json!({
        "type":"command", "command":command, "timeout":1, "on_failure":policy
    }))
    .unwrap();
    let registry = HookRegistry::new();
    registry.register_session_hook("r3", HookEvent::PostToolUse, config);
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        registry.execute_hooks(
            HookEvent::PostToolUse,
            serde_json::json!({}),
            dir.path(),
            "r3",
        ),
    )
    .await
    .unwrap();
    let pids: Vec<_> = std::fs::read_to_string(dir.path().join("descendants"))
        .unwrap()
        .lines()
        .map(|s| PidGuard(s.parse().unwrap()))
        .collect();
    let errors = if policy == "block" {
        &result.blocking_errors
    } else {
        &result.nonblocking_errors
    };
    assert!(
        errors.iter().any(|s| s.contains("no progress")),
        "{result:?}"
    );
    for pid in &pids {
        tokio::time::timeout(Duration::from_secs(3), async {
            while unsafe { libc::kill(pid.0, 0) } == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("descendant killed");
    }
}
#[tokio::test]
async fn redirected_descendant_allow_reports_stop() {
    redirected_descendant("allow", 0, 1).await;
}
#[tokio::test]
async fn redirected_descendants_block_reports_stop() {
    redirected_descendant("block", 0, 3).await;
}
#[tokio::test]
async fn redirected_descendant_nonzero_parent_reports_stop() {
    redirected_descendant("allow", 7, 1).await;
}
