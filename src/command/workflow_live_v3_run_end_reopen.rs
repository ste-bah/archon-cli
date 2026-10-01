//! Re-entry of the authored run's acceptance stage from its run end (ACC-A9).
//!
//! A pre-commit run-end observation that failed reopens acceptance: the stage
//! runs again in this session, through its own host entry point, as the next
//! attempt of the last acceptance call the run made. The stage holds the
//! chain the pin names now to the launch pin through the recorded lineage
//! path (`acceptance_chain::verify_launch_chain`), so a chain that moved by
//! sanctioned re-authoring is adopted there and one that did not is recorded
//! as the round's operational error. The outcome is then held to the round
//! the re-entry wrote, exactly as the last in-run round was.

use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::acceptance_stage::AcceptanceRoundRecordV1;
use archon_workflow::v2::script::is_acceptance_stage_call;
use archon_workflow::{
    WorkflowError, WorkflowLlmClient, WorkflowResult, WorkflowStore, WorkflowV2CallExecution,
};

use super::super::WorkflowV2ScriptRuntime;
use super::super::workflow_live_v2_finalizer::{Reopened, RunEndReopen};
use super::WorkflowV2ScriptSummary;

/// What the run's acceptance stage runs on: the script runtime, the client
/// its contract repair asks, and the run's task universe.
pub(in super::super) type AcceptanceReentry<'a> = (
    &'a WorkflowV2ScriptRuntime,
    Option<&'a dyn WorkflowLlmClient>,
    Option<&'a WorkflowV2TaskUniverse>,
);

pub(super) struct AcceptanceReopen<'a> {
    store: &'a WorkflowStore,
    run_id: &'a str,
    runtime: &'a WorkflowV2ScriptRuntime,
    llm: Option<&'a dyn WorkflowLlmClient>,
    universe: Option<&'a WorkflowV2TaskUniverse>,
}

impl<'a> AcceptanceReopen<'a> {
    pub(super) fn new(
        store: &'a WorkflowStore,
        run_id: &'a str,
        (runtime, llm, universe): AcceptanceReentry<'a>,
    ) -> Self {
        Self {
            store,
            run_id,
            runtime,
            llm,
            universe,
        }
    }
}

#[async_trait::async_trait]
impl RunEndReopen for AcceptanceReopen<'_> {
    async fn reopen(&self, summary: &WorkflowV2ScriptSummary) -> WorkflowResult<Option<Reopened>> {
        let Some(call) = summary
            .calls
            .iter()
            .rev()
            .find(|call| is_acceptance_stage_call(call))
        else {
            return Ok(None);
        };
        let execution = WorkflowV2CallExecution {
            call: call.clone(),
            input: serde_json::json!({}),
            depends_on: Vec::new(),
        };
        let result = super::super::workflow_live_v3_acceptance::run_acceptance_stage(
            self.runtime,
            &execution,
            self.store,
            self.run_id,
            self.universe,
            self.llm,
        )
        .await?;
        let run_dir = self.store.run_dir(self.run_id);
        let relative = result
            .data
            .get("record_path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                WorkflowError::StateCorrupt(format!(
                    "the re-entered acceptance call {} named no round record",
                    call.id
                ))
            })?
            .to_string();
        let path = run_dir.join(&relative);
        let bytes = std::fs::read(&path).map_err(|source| WorkflowError::Io {
            path: path.clone(),
            source,
        })?;
        let record: AcceptanceRoundRecordV1 = serde_json::from_slice(&bytes)?;
        let gate = super::gate_of(&run_dir, &record, &path);
        let (summary, gate) =
            super::hold_to_round(self.run_id, summary.clone(), gate, &record, &path);
        Ok(Some(Reopened {
            summary,
            gate,
            record_path: Some(relative),
        }))
    }
}
