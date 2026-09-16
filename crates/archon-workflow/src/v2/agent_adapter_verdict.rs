//! An accepted read-only verdict must agree with its own command evidence.
//!
//! Issue-34, live: a verification branch returned `accepted` with two `test`
//! commands at `failed` / exit 1 (`test ! -e <sibling artifacts>`), and said in
//! `output_summary` prose alone that the offenders were other tasks' artifacts —
//! `pre_existing` was never set. The host reads only the flag, so the verdict was
//! demoted after the session was gone and a remediation round (budget 6) was
//! dispatched for a task nothing was wrong with. Raising the contradiction here
//! instead lets the bounded repair loop re-ask the SAME session to reconcile
//! verdict and evidence; `verification::normalize` keeps its rules unchanged
//! for whatever comes back.
use super::super::verification::is_evidenced_pre_existing_failure;
use super::super::{
    WorkflowV2CommandKind, WorkflowV2CommandStatus, WorkflowV2Result, WorkflowV2Status,
};
use super::{WorkflowV2AgentError, WorkflowV2AgentRequest};

/// Reject a NON-write result that says `accepted` while `commands_run` holds a
/// failed `Test` command with no evidenced `pre_existing` attribution. Every
/// such command is named so one re-ask can settle them all. Write-capable calls
/// are left to the write path's own contracts.
pub(super) fn reject_accepted_with_unattributed_failed_tests(
    request: &WorkflowV2AgentRequest,
    result: &WorkflowV2Result,
) -> Result<(), WorkflowV2AgentError> {
    if request.is_write_capable() || result.status != WorkflowV2Status::Accepted {
        return Ok(());
    }
    let unattributed: Vec<String> = result
        .commands_run
        .iter()
        .filter(|command| command.kind == WorkflowV2CommandKind::Test)
        .filter(|command| command.status == WorkflowV2CommandStatus::Failed)
        .filter(|command| !is_evidenced_pre_existing_failure(command))
        .map(|command| command.command.clone())
        .collect();
    if unattributed.is_empty() {
        return Ok(());
    }
    Err(WorkflowV2AgentError::AcceptedWithFailedTestCommands(
        unattributed,
    ))
}

#[cfg(test)]
mod tests {
    use super::super::{WorkflowV2AgentAdapter, WorkflowV2AgentError, WorkflowV2AgentRequest};
    use crate::{
        WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2WriteMode,
    };

    const FAILED_COMMAND: &str = "test ! -e .archon/artifacts/sibling-output.json";

    fn request(write_mode: Option<WorkflowV2WriteMode>) -> WorkflowV2AgentRequest {
        WorkflowV2AgentRequest {
            call: WorkflowV2HostCall {
                id: "verification-wave-1-check-item".to_string(),
                method: WorkflowV2HostMethod::Parallel,
                write_mode,
                options: WorkflowV2HostOptions::default(),
            },
            role: "verifier".to_string(),
            task: "Run focused verification only.".to_string(),
            constraints: Vec::new(),
            input: serde_json::json!({ "item": { "canonical_task_ids": ["TASK-001"] } }),
            repository_root: Some("/repo".to_string()),
            project_artifacts: Default::default(),
            target_files: vec!["crates/example/src/lib.rs".to_string()],
            target_ownership_scopes: Vec::new(),
        }
    }

    fn result_json(status: &str, failed_command: serde_json::Value, write: bool) -> String {
        let files_changed = if write {
            serde_json::json!([{ "path": "crates/example/src/lib.rs" }])
        } else {
            serde_json::json!([])
        };
        serde_json::json!({
            "status": status,
            "summary": "checked the artifact set",
            "files_changed": files_changed,
            "commands_run": [
                { "kind": "test", "command": "cargo test -p example", "status": "succeeded", "exit_code": 0, "output_summary": "ok" },
                failed_command
            ],
            "task_coverage": [{
                "task_id": "TASK-001",
                "status": "accepted",
                "summary": "artifact set verified",
                "evidence": [{ "kind": "inspection", "summary": "listed the artifact directory" }]
            }]
        })
        .to_string()
    }

    fn failed_test(pre_existing: bool, output_summary: &str) -> serde_json::Value {
        serde_json::json!({
            "kind": "test",
            "command": FAILED_COMMAND,
            "status": "failed",
            "exit_code": 1,
            "output_summary": output_summary,
            "pre_existing": pre_existing
        })
    }

    #[test]
    fn read_only_accepted_with_an_unattributed_failed_test_is_re_asked() {
        // The live Issue-34 shape: attribution in prose, flag never set.
        let error = WorkflowV2AgentAdapter::new()
            .parse_agent_output(
                &request(None),
                &result_json(
                    "accepted",
                    failed_test(false, "artifacts belong to a downstream task, not this one"),
                    false,
                ),
            )
            .expect_err("an accepted verdict contradicted by a failed test must be re-asked");
        assert!(
            matches!(
                &error,
                WorkflowV2AgentError::AcceptedWithFailedTestCommands(commands)
                    if commands == &[FAILED_COMMAND.to_string()]
            ),
            "{error}"
        );
        let message = error.to_string();
        assert!(message.contains(FAILED_COMMAND), "{message}");
        assert!(message.contains("pre_existing: true"), "{message}");
    }

    #[test]
    fn read_only_accepted_with_an_evidenced_pre_existing_failure_stands() {
        WorkflowV2AgentAdapter::new()
            .parse_agent_output(
                &request(None),
                &result_json(
                    "accepted",
                    failed_test(true, "path is owned by TASK-002; absent at baseline too"),
                    false,
                ),
            )
            .expect("an evidenced pre_existing attribution is the normaliser's to review");
    }

    #[test]
    fn a_bare_pre_existing_flag_without_evidence_is_still_re_asked() {
        let error = WorkflowV2AgentAdapter::new()
            .parse_agent_output(
                &request(None),
                &result_json("accepted", failed_test(true, "   "), false),
            )
            .expect_err("a flag with no evidence is an ordinary failure");
        assert!(
            matches!(
                error,
                WorkflowV2AgentError::AcceptedWithFailedTestCommands(_)
            ),
            "{error}"
        );
    }

    #[test]
    fn a_non_accepted_verdict_with_a_failed_test_is_not_contradicted() {
        WorkflowV2AgentAdapter::new()
            .parse_agent_output(
                &request(None),
                &result_json(
                    "needs_review",
                    failed_test(false, "artifacts belong to a downstream task"),
                    false,
                ),
            )
            .expect("needs_review agrees with a failed test; nothing to re-ask");
    }

    #[test]
    fn the_write_path_is_not_touched_by_the_verdict_check() {
        let outcome = WorkflowV2AgentAdapter::new().parse_agent_output(
            &request(Some(WorkflowV2WriteMode::Coordinated)),
            &result_json(
                "accepted",
                failed_test(false, "artifacts belong to a downstream task"),
                true,
            ),
        );
        assert!(
            !matches!(
                outcome,
                Err(WorkflowV2AgentError::AcceptedWithFailedTestCommands(_))
            ),
            "the write path keeps its own contracts: {outcome:?}"
        );
        outcome.expect("write-path validation is unchanged by this check");
    }
}
