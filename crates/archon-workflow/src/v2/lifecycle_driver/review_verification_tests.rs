use super::review_verification::{
    review_verification_execution_retry_items, review_verification_has_execution_failure,
    review_verification_options,
};
use crate::v2::lifecycle_prompts as prompts;

#[test]
fn review_verification_retry_items_keep_execution_failures_only() {
    let items = vec![
        serde_json::json!({
            "item_id": "verify-a",
            "focused_verification": ["cargo test focused_a"]
        }),
        serde_json::json!({
            "item_id": "verify-b",
            "focused_verification": ["python3 check.py"]
        }),
    ];
    let verification = serde_json::json!({
        "status": "needs_review",
        "outcomes": [
            {
                "item_id": "verify-a",
                "status": "failed",
                "failure_kind": "execution",
                "error": "subagent timed out after 1200s"
            },
            {
                "item_id": "verify-b",
                "status": "failed",
                "failure_kind": "semantic"
            }
        ]
    });

    let retry_items = review_verification_execution_retry_items(&items, &verification);

    assert!(review_verification_has_execution_failure(&verification));
    assert_eq!(retry_items.len(), 1);
    assert_eq!(retry_items[0]["item_id"], "verify-a");
}

#[test]
fn cargo_review_verification_items_are_serialized() {
    let options = review_verification_options(
        &[serde_json::json!({
            "item_id": "verify-cargo",
            "focused_verification": ["cargo test -p archon-cli-workspace focused"]
        })],
        "run review verification",
    );

    assert_eq!(options["maxParallelism"], 1);
}

#[test]
fn non_cargo_review_verification_keeps_default_parallelism() {
    let options = review_verification_options(
        &[serde_json::json!({
            "item_id": "verify-python",
            "focused_verification": ["python3 check.py"]
        })],
        "run review verification",
    );

    assert!(options.get("maxParallelism").is_none());
}

#[test]
fn review_verification_prompt_grounds_run_artifact_markers() {
    // D75: the run-artifact grounding is domain-neutral — a directory counts
    // as a concrete run artifact only via its declared config/manifest or
    // explicit readiness/acceptance evidence, for ANY PRD's artifact layout.
    assert!(prompts::REVIEW_REMEDIATION_WAVE_TASK.contains("workflow run artifact directory"));
    assert!(prompts::REVIEW_VERIFICATION_PLAN_TASK.contains("declared config/manifest file"));
    assert!(prompts::REVIEW_VERIFICATION_PLAN_TASK.contains("hygiene residual gaps"));
    assert!(prompts::REVIEW_VERIFICATION_WAVE_TASK.contains("Declared readiness blockers"));
}
