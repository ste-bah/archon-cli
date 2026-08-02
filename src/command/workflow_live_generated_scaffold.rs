use std::path::Path;

use archon_core::config::GeneratedWorkflowConfig;
use archon_workflow::{
    GeneratedWorkflowLearningContext, WorkflowResult, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2HostOptions, WorkflowV2WriteMode,
};

use super::workflow_live_task_universe::WorkflowV2TaskUniverse;

/// The recorded plan document for a decomposed-PRD run.
///
/// The lifecycle executes natively in Rust (`run_decomposed_lifecycle`); this
/// descriptor is the run's durable record and hash identity — what the
/// maintainer approves and what result reuse keys on. It is deterministic for
/// a given task universe, learning context, and configuration.
pub(super) fn decomposed_prd_scaffold(
    task: &str,
    target_repository_root: Option<&str>,
    task_universe: &WorkflowV2TaskUniverse,
    governed_learning_context: &[GeneratedWorkflowLearningContext],
    generated_config: &GeneratedWorkflowConfig,
) -> WorkflowResult<String> {
    let universe_json = serde_json::to_string_pretty(task_universe)?;
    let learning_json = serde_json::to_string_pretty(governed_learning_context)?;
    let max_repair_iterations = generated_config.max_repair_iterations.clamp(1, 8);
    let max_investigation_iterations = generated_config.max_investigation_iterations.clamp(1, 8);
    let max_dependency_waves = task_universe.tasks.len().saturating_mul(3).max(1);

    let mut descriptor = String::new();
    descriptor.push_str("# Archon decomposed-PRD workflow (native lifecycle v1)\n");
    descriptor.push_str(
        "# Executed by the Rust lifecycle driver; this document is the approved plan record.\n\n",
    );
    descriptor.push_str(&format!("task: {}\n", serde_json::to_string(task)?));
    descriptor.push_str(&format!(
        "target_repository_root: {}\n",
        serde_json::to_string(&target_repository_root)?
    ));
    descriptor.push_str(&format!(
        "project_artifact_root: {}\n",
        serde_json::to_string(&project_artifact_root(task_universe))?
    ));
    descriptor.push_str(&format!("max_repair_iterations: {max_repair_iterations}\n"));
    descriptor.push_str(&format!(
        "max_investigation_iterations: {max_investigation_iterations}\n"
    ));
    descriptor.push_str(&format!("max_dependency_waves: {max_dependency_waves}\n\n"));
    descriptor.push_str("stage_families:\n");
    for call in decomposed_prd_plan_calls() {
        descriptor.push_str(&format!(
            "- {} ({}{})\n",
            call.id,
            call.method.as_str(),
            match call.write_mode {
                Some(mode) => format!(", write={mode:?}"),
                None => String::new(),
            },
        ));
    }
    descriptor.push_str("\ntask_universe:\n");
    descriptor.push_str(&universe_json);
    descriptor.push_str("\n\ngoverned_learning_context:\n");
    descriptor.push_str(&learning_json);
    descriptor.push('\n');
    Ok(descriptor)
}

/// The approval-time plan for the deterministic decomposed-PRD lifecycle:
/// one entry per stage family, with the write mode, item kind, and source the
/// lifecycle uses. This is the single declaration the approval surface, the
/// persisted host-call manifest, and restart invalidation consume.
pub(super) fn decomposed_prd_plan_calls() -> Vec<WorkflowV2HostCall> {
    const WORKTREE: Option<WorkflowV2WriteMode> = Some(WorkflowV2WriteMode::Worktree);
    // The plan names its host methods as typed variants rather than strings, so
    // "every static plan method parses" is discharged by the type checker at
    // construction instead of by a runtime `parse` that could only panic.
    const PARALLEL: WorkflowV2HostMethod = WorkflowV2HostMethod::Parallel;
    const REDUCE: WorkflowV2HostMethod = WorkflowV2HostMethod::Reduce;
    const FANOUT: WorkflowV2HostMethod = WorkflowV2HostMethod::Fanout;
    const FINAL_REPORT: WorkflowV2HostMethod = WorkflowV2HostMethod::FinalReport;
    const SAVE_ARTIFACT: WorkflowV2HostMethod = WorkflowV2HostMethod::SaveArtifact;
    const REQUIRE_ARTIFACT: WorkflowV2HostMethod = WorkflowV2HostMethod::RequireArtifact;
    const QUALITY_GATE: WorkflowV2HostMethod = WorkflowV2HostMethod::QualityGate;
    const PLAN: &[(&str, WorkflowV2HostMethod, Option<WorkflowV2WriteMode>)] = &[
        ("initial-readonly-discovery", PARALLEL, None),
        ("canonical-implementation-inventory", REDUCE, None),
        ("inventory-shape-repair", REDUCE, None),
        ("task-universe-reconcile", REDUCE, None),
        ("dependency-graph-repair", REDUCE, None),
        ("target-file-discovery", REDUCE, None),
        ("verification-requirements-discovery", REDUCE, None),
        ("artifact-requirements-discovery", REDUCE, None),
        ("provider-environment-discovery", REDUCE, None),
        ("evidence-repair", REDUCE, None),
        ("blocked-malformed-inventory", FINAL_REPORT, None),
        ("blocked-empty-implementation-inventory", FINAL_REPORT, None),
        ("dependency-graph-repair-deadlock", REDUCE, None),
        ("blocked-dependency-deadlock", FINAL_REPORT, None),
        ("noop-proof-verification", PARALLEL, None),
        ("noop-evidence-repair", REDUCE, None),
        ("noop-proof-reverification", PARALLEL, None),
        ("blocked-noop-proof-failed", FINAL_REPORT, None),
        ("implementation-wave", FANOUT, WORKTREE),
        ("remediation-inventory", REDUCE, None),
        ("remediation-empty-inventory-repair", REDUCE, None),
        ("blocked-malformed-remediation", FINAL_REPORT, None),
        ("remediation-wave", FANOUT, WORKTREE),
        ("remediation-outcome-repair", REDUCE, None),
        ("ownership-expansion-inventory", REDUCE, None),
        ("blocked-remediation-unresolved", FINAL_REPORT, None),
        ("verification-plan", REDUCE, None),
        ("verification-plan-repair", REDUCE, None),
        ("blocked-empty-verification", FINAL_REPORT, None),
        ("verification-wave", PARALLEL, None),
        ("verification-failure-triage", REDUCE, None),
        ("verification-remediation-inventory", REDUCE, None),
        ("post-remediation-verification-plan", REDUCE, None),
        ("post-remediation-verification-plan-repair", REDUCE, None),
        ("verification-repair-plan", REDUCE, None),
        ("verification-repair-shape-repair", REDUCE, None),
        ("blocked-verification-failed", FINAL_REPORT, None),
        // The third point of the per-task diamond. PARALLEL, one reviewer per
        // task, listed here because that is where it runs: after this wave's
        // verification accepted the task, not after every wave. It was a single
        // terminal REDUCE over all tasks, which made attribution inferential
        // (a reducer has no per-item branch to recover a task id from) and made
        // every finding arrive too late to be cheap.
        ("adversarial-review", PARALLEL, None),
        ("wave-completion-evidence-repair", REDUCE, None),
        ("blocked-no-completion", FINAL_REPORT, None),
        ("blocked-loop-exhaustion", FINAL_REPORT, None),
        ("artifact-inventory", REDUCE, None),
        ("save-artifact-inventory", SAVE_ARTIFACT, None),
        // The terminal reduce, narrowed: contradictions BETWEEN tasks, global
        // invariants, PRD-level acceptance. It is handed the task universe and
        // a digest of the per-task findings — never the run's implementation
        // or verification evidence — so it cannot re-review per-task work.
        ("cross-cutting-review", REDUCE, None),
        ("review-remediation-inventory", REDUCE, None),
        ("review-remediation-inventory-repair", REDUCE, None),
        ("blocked-empty-review-remediation", FINAL_REPORT, None),
        ("review-remediation-wave", FANOUT, WORKTREE),
        ("review-verification-plan", REDUCE, None),
        ("blocked-empty-review-verification", FINAL_REPORT, None),
        ("review-verification-wave", PARALLEL, None),
        ("blocked-review-verification-failed", FINAL_REPORT, None),
        ("blocked-review-unresolved", FINAL_REPORT, None),
        ("blocked-review-not-accepted", FINAL_REPORT, None),
        ("final-evidence-reconciliation", REDUCE, None),
        ("completion-claim-repair", REDUCE, None),
        ("artifact-existence-investigation", PARALLEL, None),
        ("blocked-final-evidence-reconciliation", FINAL_REPORT, None),
        ("require-final-artifacts", REQUIRE_ARTIFACT, None),
        ("final-zero-gap-audit", REDUCE, None),
        ("final-acceptance-gate", QUALITY_GATE, None),
        ("blocked-final-readiness", FINAL_REPORT, None),
        ("final-acceptance-report", FINAL_REPORT, None),
    ];
    // Families that occur exactly once keep their literal id; every other
    // family is suffixed at runtime, so the plan records its dynamic prefix.
    const STATIC_IDS: &[&str] = &[
        "blocked-malformed-inventory",
        "blocked-empty-implementation-inventory",
        "artifact-inventory",
        "save-artifact-inventory",
        "blocked-review-unresolved",
        "blocked-review-not-accepted",
        "blocked-final-evidence-reconciliation",
        "require-final-artifacts",
        "final-zero-gap-audit",
        "final-acceptance-gate",
        "blocked-final-readiness",
        "final-acceptance-report",
    ];
    PLAN.iter()
        .map(|(id, method, write_mode)| {
            let item_kind = match *id {
                "implementation-wave" | "remediation-wave" | "review-remediation-wave" => {
                    Some("implementation".to_string())
                }
                "verification-wave" | "review-verification-wave" => {
                    Some("focused_verification".to_string())
                }
                "noop-proof-verification" | "noop-proof-reverification" => {
                    Some("noop_proof".to_string())
                }
                _ => None,
            };
            let source = match *id {
                "implementation-wave" => Some("readyImplementationItems".to_string()),
                "remediation-wave" => Some("remediationInventory.items".to_string()),
                "review-remediation-wave" => Some("reviewRemediationInventory.items".to_string()),
                _ => None,
            };
            let mut options = WorkflowV2HostOptions {
                item_kind,
                source,
                ..Default::default()
            };
            if !STATIC_IDS.contains(id) {
                options.extra.insert(
                    "dynamic_id_prefix".to_string(),
                    serde_json::Value::String(format!("{id}-")),
                );
            }
            WorkflowV2HostCall {
                id: (*id).to_string(),
                method: *method,
                write_mode: *write_mode,
                options,
            }
        })
        .collect()
}

fn project_artifact_root(task_universe: &WorkflowV2TaskUniverse) -> Option<String> {
    task_universe
        .source_roots
        .iter()
        .chain(task_universe.tasks.iter().map(|task| &task.source_path))
        .filter_map(|path| project_root_from_path(path))
        .next()
}

fn project_root_from_path(path: &str) -> Option<String> {
    for ancestor in Path::new(path).ancestors() {
        if ancestor.join(".archon").is_dir() {
            return Some(ancestor.display().to_string());
        }
    }
    None
}
