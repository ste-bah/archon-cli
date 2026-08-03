// Rust decomposed-PRD lifecycle driver — the faithful port of the generated
// scaffold JS (body_a/body_b + verification/ownership splices). It drives the same
// `WorkflowScriptHost::execute` entry point the QuickJS bridge used, so
// result reuse, source metadata, run control, events, and persistence behave identically.

use super::*;

pub(super) use super::super::super::workflow_live_semantic_preservation as semantic_preservation;
pub(super) use super::workflow_live_v2_lifecycle_prompts as prompts;
pub(super) use archon_workflow::generated_lifecycle_remediation as remediation;
pub(super) use archon_workflow::generated_lifecycle_support as support;
pub(super) use archon_workflow::generated_lifecycle_support::LifecycleContract;

pub(super) const TERMINAL_GATE_REROUTE_MARKER: &str = "workflow terminal gate reroute:";

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
        let host = Arc::new(WorkflowScriptHost {
            scaffold_hash: workflow_scaffold_hash(harness_source),
            runner: self,
            accumulator: Arc::new(Mutex::new(WorkflowScriptAccumulator::default())),
        });
        let driver = LifecycleDriver::new(
            host.clone(),
            task_universe,
            target_repository_root,
            project_artifact_root,
            governed_learning_context,
            resume_completed_ids,
            &generated_config,
        );
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

pub(super) struct LifecycleDriver {
    pub(super) host: Arc<WorkflowScriptHost>,
    pub(super) universe: WorkflowV2TaskUniverse,
    pub(super) task_universe: serde_json::Value,
    pub(super) target_repository_root: Option<String>,
    pub(super) project_artifact_root: Option<String>,
    pub(super) governed_learning_context: serde_json::Value,
    pub(super) max_repair_iterations: usize,
    pub(super) max_investigation_iterations: usize,
    pub(super) max_dependency_waves: usize,
    /// How many ready tasks the write fan-out may dispatch at once, or `None`
    /// for the configured subagent concurrency. See
    /// `archon_core::config::decide_fanout_width` — this value can only ever be
    /// narrower than the cap, and the runtime clamps it again on the way out.
    pub(super) write_wave_width: Option<usize>,
    pub(super) runtime_state:
        std::sync::Mutex<workflow_live_v2_lifecycle_terminal_gate::TerminalGateState>,
}

/// Mutable evidence bundles — the JS lifecycle's top-level arrays.
#[derive(Default)]
pub(super) struct LifecycleEvidence {
    pub(super) implementation: Vec<serde_json::Value>,
    pub(super) verification: Vec<serde_json::Value>,
    pub(super) review: Vec<serde_json::Value>,
    pub(super) artifact: Vec<serde_json::Value>,
    pub(super) repair_attempts: Vec<serde_json::Value>,
    pub(super) final_evidence_repair_attempts: Vec<serde_json::Value>,
}

#[path = "workflow_live_v2_lifecycle_driver_a.rs"]
mod workflow_live_v2_lifecycle_driver_a;
pub(crate) use workflow_live_v2_lifecycle_driver_a::*;
#[path = "workflow_live_v2_lifecycle_driver_b.rs"]
mod workflow_live_v2_lifecycle_driver_b;
pub(crate) use workflow_live_v2_lifecycle_driver_b::*;
#[path = "workflow_live_v2_lifecycle_driver_c.rs"]
mod workflow_live_v2_lifecycle_driver_c;
pub(crate) use workflow_live_v2_lifecycle_driver_c::*;

pub(super) fn normalize_null_report_collections(value: &mut serde_json::Value) {
    const COLLECTION_FIELDS: &[&str] = &[
        "accepted_tasks",
        "actionable",
        "artifact_requirements",
        "artifacts",
        "blocked_tasks",
        "canonical_task_ids",
        "commands_run",
        "completed_ids",
        "dependency_ids",
        "evidence",
        "failed_tasks",
        "files_changed",
        "files_read",
        "focused_verification",
        "items",
        "missing_tasks",
        "noop_tasks",
        "outcomes",
        "remediation_actions",
        "repair_attempts",
        "residual_gaps",
        "retry_items",
        "review_blockers",
        "review_findings",
        "target_files",
        "task_coverage",
        "tests_run",
        "unresolved_issues",
    ];
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                if child.is_null() && COLLECTION_FIELDS.contains(&key.as_str()) {
                    *child = serde_json::Value::Array(Vec::new());
                } else {
                    normalize_null_report_collections(child);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                normalize_null_report_collections(child);
            }
        }
        _ => {}
    }
}

pub(super) fn terminal_marker_requires_report_fallback(status: Option<WorkflowV2Status>) -> bool {
    status == Some(WorkflowV2Status::Failed)
}

pub(super) fn is_terminal_gate_reroute(error: &WorkflowError) -> bool {
    error.to_string().contains(TERMINAL_GATE_REROUTE_MARKER)
}
