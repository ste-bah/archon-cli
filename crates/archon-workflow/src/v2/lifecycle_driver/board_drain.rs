// The drain gate's one board read.
//
// `lifecycle_policy::drain_gate` decides what counts as drained; this performs
// the single read that feeds it and turns a refusal into a report. It is called
// from `run_final_gates`, after the final acceptance gate has accepted and
// immediately before the accepted report — the last barrier in the run, where
// every wave, fan-out and review round has already completed and no branch is
// still writing to the board.

use crate::v2::lifecycle_policy::drain_gate;
use crate::v2::lifecycle_prompts as prompts;

use super::{LifecycleDriver, LifecycleEvidence};

impl LifecycleDriver {
    /// Refuse acceptance while this run still owns undrained board issues.
    ///
    /// Returns `true` when it reported and the caller must return.
    ///
    /// A run with no board configured passes: the gate asserts that a board
    /// which exists was drained, and inventing a failure for a runtime that
    /// never had one would make the board mandatory by accident.
    ///
    /// A board that cannot be READ, however, fails. "The board is unreachable"
    /// and "the board is empty" are the same silence from here, and treating
    /// that silence as success is precisely the failure mode this gate exists
    /// to remove.
    pub(crate) async fn block_undrained_board(
        &self,
        evidence: &LifecycleEvidence,
        final_inputs: &serde_json::Value,
    ) -> crate::WorkflowResult<bool> {
        let Some((run_id, board)) = self.board_drain.as_ref() else {
            return Ok(false);
        };
        let outcome = match board.drain_items_for_run(run_id) {
            Ok(items) => drain_gate::evaluate(run_id, &items),
            Err(error) => {
                return self
                    .report_undrained(
                        serde_json::json!({
                            "runId": run_id,
                            "boardDrainError": error.to_string(),
                            "message": format!(
                                "board drain gate: run {run_id} could not read its task board: {error}"
                            ),
                        }),
                        evidence,
                        final_inputs,
                    )
                    .await
                    .map(|()| true);
            }
        };
        if outcome.passed() {
            return Ok(false);
        }
        self.report_undrained(
            serde_json::json!({
                "runId": outcome.run_id,
                "message": outcome.failure_message(),
                "boardItemsInspected": outcome.inspected,
                "boardIssues": outcome.issues,
                "undrainedBoardItems": outcome
                    .undrained
                    .iter()
                    .map(|item| serde_json::json!({
                        "id": item.id,
                        "title": item.title,
                        "status": item.status.as_str(),
                        "reason": item.reason,
                    }))
                    .collect::<Vec<_>>(),
            }),
            evidence,
            final_inputs,
        )
        .await
        .map(|()| true)
    }

    async fn report_undrained(
        &self,
        drain: serde_json::Value,
        evidence: &LifecycleEvidence,
        final_inputs: &serde_json::Value,
    ) -> crate::WorkflowResult<()> {
        let mut inputs = final_inputs.clone();
        if let Some(object) = inputs.as_object_mut() {
            object.insert("boardDrain".to_string(), drain);
            object.insert(
                "artifactEvidence".to_string(),
                serde_json::json!(evidence.artifact),
            );
        }
        self.final_report(
            "blocked-board-drain",
            None,
            "needs_review",
            inputs,
            prompts::BLOCKED_BOARD_DRAIN_TASK,
        )
        .await
    }
}
