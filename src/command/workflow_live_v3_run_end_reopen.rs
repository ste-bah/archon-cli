//! Re-entry of the authored run's acceptance stage from its run end (ACC-A9).
//!
//! A pre-commit run-end observation that failed reopens acceptance: the stage
//! runs again in this session, through its own host entry point, as the next
//! attempt of the last acceptance call the run made. The stage holds the
//! chain the pin names now to the launch pin through the recorded lineage
//! path (`acceptance_chain::verify_launch_chain`), so a chain that moved by
//! sanctioned re-authoring is adopted there and one that did not is recorded
//! as the round's operational error. The outcome is then held to the round
//! the re-entry wrote, exactly as the last in-run round was. A round that
//! reads back damaged or unreadable pauses the run (Issue 326).

use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::acceptance_stage::{AcceptanceRoundRecordV1, progress};
use archon_workflow::v2::script::is_acceptance_stage_call;
use archon_workflow::{
    WorkflowError, WorkflowLlmClient, WorkflowResult, WorkflowStore, WorkflowV2CallExecution,
    WorkflowV2ResultStore,
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
    /// The executor's launch generation, when the finalizer was given it.
    generation: Option<u64>,
    v2_store: &'a WorkflowV2ResultStore,
}

impl<'a> AcceptanceReopen<'a> {
    pub(super) fn new(
        store: &'a WorkflowStore,
        run_id: &'a str,
        (runtime, llm, universe): AcceptanceReentry<'a>,
        (generation, v2_store): (Option<u64>, &'a WorkflowV2ResultStore),
    ) -> Self {
        Self {
            store,
            run_id,
            runtime,
            llm,
            universe,
            generation,
            v2_store,
        }
    }

    /// Issue 316: the generation the re-entered round runs under, read once
    /// here, at its dispatch, and only while this executor owns the run (the
    /// finalizer's launch generation, else this session's executor), as the
    /// script host reads a call's. Never the launch generation itself: an
    /// edit that kept the executor moved the run on, and a pause under the
    /// launch generation would be refused and end the run Cancelled.
    fn owned_generation(&self) -> WorkflowResult<u64> {
        let run = self.store.load_state(self.run_id)?;
        match self.generation {
            Some(launch) => archon_workflow::control_pause::require_executor(&run, launch)?,
            None => self.v2_store.require_session_executor(&run)?,
        }
        Ok(run.generation)
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
            self.owned_generation()?,
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
        let (record, path) = reentered_round(self.store, self.run_id, &relative)?;
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

/// The round the re-entered stage wrote at `relative` (Issue 326). Damage
/// is never an error and never stands in for a verdict: a record that will
/// not parse is quarantined with evidence (the history's own healing load,
/// [`progress::ProgressLedger::load_healing`]), one that is gone or that the
/// file system will not hand over is left as it is, and either way the run
/// PAUSES with the reason (`Err(ControlPaused)`). Nothing is rebuilt from
/// another record (Issue 313, round 2); the resume runs the stage again and
/// its new round heals the run.
pub(super) fn reentered_round(
    store: &WorkflowStore,
    run_id: &str,
    relative: &str,
) -> WorkflowResult<(AcceptanceRoundRecordV1, std::path::PathBuf)> {
    let run_dir = store.run_dir(run_id);
    let path = run_dir.join(relative);
    let pause = |reason: String| super::call::pause(store, run_id, relative, &reason);
    let why = match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(record) => return Ok((record, path)),
            Err(error) => format!("it will not parse ({error})"),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let reason = format!("the re-entered acceptance round {relative} is gone");
            return Err(pause(reason));
        }
        Err(error) => {
            let reason =
                format!("the re-entered acceptance round {relative} cannot be read ({error})");
            return Err(pause(reason));
        }
    };
    let healed = match progress::ProgressLedger::load_healing(&run_dir) {
        Ok(healed) => healed,
        Err(error) => {
            let reason = format!(
                "the re-entered acceptance round {relative} is damaged ({why}) and cannot be quarantined ({error})"
            );
            return Err(pause(reason));
        }
    };
    progress::record_quarantine_events(store, run_id, &healed.quarantined);
    let moved = (healed.quarantined.iter()).find(|q| q.original == relative);
    let kept = moved.map_or_else(
        || "it was not quarantined".to_string(),
        |q| format!("it is quarantined at {}", q.quarantined),
    );
    let reason = format!("the re-entered acceptance round {relative} is damaged ({why}); {kept}");
    let paused = pause(reason);
    // The pause reports this loss: acknowledged only once it is recorded,
    // so the resumed stage does not pause on it a second time.
    let lost: Vec<_> = (healed.unacknowledged.iter())
        .filter(|q| q.original == relative)
        .cloned()
        .collect();
    if matches!(paused, WorkflowError::ControlPaused(_))
        && !lost.is_empty()
        && let Err(error) = progress::acknowledge_quarantined(&run_dir, &lost)
    {
        tracing::warn!(%error, run_id, "the reported loss of the re-entered acceptance round was not acknowledged; the resumed stage pauses on it again");
    }
    Err(paused)
}
