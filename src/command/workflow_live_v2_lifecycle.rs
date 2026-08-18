// Composition root for the decomposed-PRD lifecycle.
//
// The lifecycle itself is `archon_workflow::v2::lifecycle_driver`. What lives
// here is the only part that touches the concrete host: it builds the
// `WorkflowScriptHost` around this runner, hands it to the driver through
// `archon_workflow::LifecycleHost`, and turns the driver's outcome back into a
// `WorkflowV2ScriptSummary`.
//
// It is spelled as an inherent `impl WorkflowV2ScriptRunner`, so coherence
// pins it to this crate regardless — but it would belong here anyway. The three
// host calls below (`summary`, `emit_terminal_status`, `mark_script_failure`)
// are the ones an earlier survey counted as driver reaches; they are not. The
// driver never makes them.

use super::*;

use archon_workflow::v2::lifecycle_driver::{LifecycleDriver, LifecycleLimits};

// The one adapter this composition root owns that is not the host: the board
// behind `archon_workflow::WorkflowBoardPort`. Declared here because here is the
// only place it is installed.
#[path = "workflow_board_drain.rs"]
mod workflow_board_drain;
use workflow_board_drain::process_board_drain;

impl WorkflowV2ScriptRunner {
    /// Run the decomposed-PRD lifecycle natively. `harness_source` is the
    /// recorded scaffold (hash identity for reuse/metadata); it is NOT
    /// executed.
    pub(in super::super::super) async fn run_decomposed_lifecycle(
        self,
        harness_source: &str,
        governed_learning_context: serde_json::Value,
    ) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
        let harness_source = harness_source.to_string();
        tokio::task::spawn_blocking(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|err| {
                    WorkflowError::SpecInvalid(format!(
                        "decomposed lifecycle local async runtime failed: {err}"
                    ))
                })?;
            runtime.block_on(self.run_decomposed_lifecycle_on_current_thread(
                &harness_source,
                governed_learning_context,
            ))
        })
        .await
        .map_err(|err| {
            WorkflowError::SpecInvalid(format!("decomposed lifecycle task failed: {err}"))
        })?
    }

    pub(super) async fn run_decomposed_lifecycle_on_current_thread(
        self,
        harness_source: &str,
        governed_learning_context: serde_json::Value,
    ) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
        let Some(task_universe) = self.task_universe.clone() else {
            return Err(WorkflowError::SpecInvalid(
                "decomposed lifecycle requires an authoritative task universe".to_string(),
            ));
        };
        let target_repository_root = self.runtime.target_repository_root.clone();
        let project_artifact_root =
            archon_workflow::project_artifact_context_from_v2_root(self.v2_store.root())
                .project_root;
        let resume_completed_ids = self.resume_completed_ids.clone();
        let generated_config = self.runtime.generated_config.clone();
        // Read before `self` moves into the host. This is `WorkflowRun.id` --
        // the same `wf-{uuid}` a subagent inherits as its session id prefix, and
        // therefore the exact partition its board writes landed in.
        let run_id = self.run_id.clone();
        let host = Arc::new(WorkflowScriptHost {
            scaffold_hash: workflow_scaffold_hash(harness_source),
            runner: self,
            accumulator: Arc::new(Mutex::new(WorkflowScriptAccumulator::default())),
            tool_host: std::sync::OnceLock::new(),
            tool_budget: Arc::new(std::sync::Mutex::new(Default::default())),
        });
        let driver = LifecycleDriver::new(
            host.clone(),
            task_universe,
            target_repository_root,
            project_artifact_root,
            governed_learning_context,
            resume_completed_ids,
            LifecycleLimits {
                max_repair_iterations: generated_config.max_repair_iterations,
                max_investigation_iterations: generated_config.max_investigation_iterations,
                implementation_wave_max_parallelism: generated_config
                    .implementation_wave_max_parallelism,
            },
        );
        // The drain gate's only production wiring. Without this line the gate,
        // its policy and its tests are code that never runs -- which is the
        // failure mode the board exists to make visible, so it would be a poor
        // one to reproduce here.
        //
        // Attached unconditionally -- including when this process has no board.
        // `process_board_drain` answers that case with a port that reports why it
        // cannot read rather than with `None`, because `None` means "pass" and a
        // run whose completion could not be checked has not been shown to be
        // complete. Scoping the gate to runs that "look like they use the board"
        // would likewise mean deciding in advance which runs are allowed to leave
        // work behind.
        let driver = driver.with_board_drain(run_id, process_board_drain());
        // v3: ONE persistent orchestrator conversation instead of the v2
        // reducer relay. Opt-in via env until certified as the default.
        let orchestrated = std::env::var("ARCHON_ORCHESTRATED_LIFECYCLE")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let outcome = if orchestrated {
            driver.run_orchestrated().await
        } else {
            driver.run().await
        };
        match outcome {
            Ok(()) => {
                let summary = host.summary().await;
                host.emit_terminal_status(summary.status);
                Ok(summary)
            }
            Err(err) => {
                if matches!(
                    err,
                    WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_)
                ) {
                    return Err(err);
                }
                let error = err.to_string();
                if error.contains(TERMINAL_HOST_CALL_MARKER) {
                    let summary = host.summary().await;
                    host.emit_terminal_status(summary.status);
                    return Ok(summary);
                }
                let summary = host.mark_script_failure(&error, true).await;
                Ok(summary)
            }
        }
    }
}
