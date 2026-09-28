//! Batch I: a remediation agent sees the failure its check printed -- the end
//! of the output and every failure line -- never a head-slice of warnings.
use super::tests::{ACCEPTANCE_TAIL, acceptance_reply, run_scripted, script, view};
use crate::failure_evidence::failure_evidence;

/// A `cargo run` check's stderr: pages of warnings, then the program's error.
fn warnings_then(error: &str) -> String {
    let mut out = String::new();
    for n in 0..300 {
        out.push_str(&format!("warning: unused variable `v{n}` in src/m{n}.rs\n"));
    }
    out.push_str(error);
    out.push('\n');
    out
}

/// A failure mid-output followed by many lines.
fn error_then_lines(error: &str) -> String {
    let mut out = String::from("starting the ingest\n");
    out.push_str(error);
    out.push('\n');
    for n in 0..400 {
        out.push_str(&format!("teardown step {n} completed\n"));
    }
    out
}

#[tokio::test]
async fn the_remediation_prompt_carries_every_checks_failure_line() {
    let checks = serde_json::json!([
        { "check_id": "REQ-2", "criterion": "two is done", "kind": "command", "status": "failed", "exit_code": 1,
          "owning_tasks": ["TASK-Q-002"],
          "stderr_tail": failure_evidence(warnings_then("Error: X").as_bytes(), 4000) },
        { "check_id": "REQ-3", "criterion": "three is done", "kind": "command", "status": "failed", "exit_code": 1,
          "owning_tasks": ["TASK-Q-002"],
          "stderr_tail": failure_evidence(warnings_then("Error: second X").as_bytes(), 4000),
          "stdout_tail": failure_evidence(error_then_lines("AssertionError: middle Y").as_bytes(), 4000) }
    ]);
    let (calls, _) = run_scripted(
        &script("schema: 2, ", ACCEPTANCE_TAIL),
        move |_, payload| {
            let id = payload["id"].as_str().unwrap_or_default();
            if id == "acceptance-contract-run-1" {
                return acceptance_reply(1, checks.clone(), false);
            }
            if id.starts_with("acceptance-contract-run-") {
                return acceptance_reply(2, serde_json::json!([]), true);
            }
            view(
                serde_json::json!({ "items": [], "outcomes": [] }),
                "accepted",
            )
        },
    )
    .await;
    let prompt = calls
        .iter()
        .find(|(method, p)| {
            method == "fanout"
                && p["id"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("review-remediate-task-q-002")
        })
        .and_then(|(_, p)| p["source"][0]["task"].as_str().map(str::to_string))
        .unwrap_or_else(|| panic!("a remediation write for TASK-Q-002: {calls:#?}"));
    for line in [
        "Error: X",
        "Error: second X",
        "AssertionError: middle Y",
        "teardown step 399 completed",
    ] {
        assert!(prompt.contains(line), "missing {line:?}: {prompt}");
    }
}
