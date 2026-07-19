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
    const PLAN: &[(&str, &str, Option<WorkflowV2WriteMode>)] = &[
        ("initial-readonly-discovery", "parallel", None),
        ("canonical-implementation-inventory", "reduce", None),
        ("inventory-shape-repair", "reduce", None),
        ("task-universe-reconcile", "reduce", None),
        ("dependency-graph-repair", "reduce", None),
        ("target-file-discovery", "reduce", None),
        ("verification-requirements-discovery", "reduce", None),
        ("artifact-requirements-discovery", "reduce", None),
        ("provider-environment-discovery", "reduce", None),
        ("evidence-repair", "reduce", None),
        ("blocked-malformed-inventory", "finalReport", None),
        (
            "blocked-empty-implementation-inventory",
            "finalReport",
            None,
        ),
        ("dependency-graph-repair-deadlock", "reduce", None),
        ("blocked-dependency-deadlock", "finalReport", None),
        ("noop-proof-verification", "parallel", None),
        ("noop-evidence-repair", "reduce", None),
        ("noop-proof-reverification", "parallel", None),
        ("blocked-noop-proof-failed", "finalReport", None),
        ("implementation-wave", "fanout", WORKTREE),
        ("remediation-inventory", "reduce", None),
        ("remediation-empty-inventory-repair", "reduce", None),
        ("blocked-malformed-remediation", "finalReport", None),
        ("remediation-wave", "fanout", WORKTREE),
        ("remediation-outcome-repair", "reduce", None),
        ("ownership-expansion-inventory", "reduce", None),
        ("blocked-remediation-unresolved", "finalReport", None),
        ("verification-plan", "reduce", None),
        ("verification-plan-repair", "reduce", None),
        ("blocked-empty-verification", "finalReport", None),
        ("verification-wave", "parallel", None),
        ("verification-failure-triage", "reduce", None),
        ("verification-remediation-inventory", "reduce", None),
        ("post-remediation-verification-plan", "reduce", None),
        ("post-remediation-verification-plan-repair", "reduce", None),
        ("verification-repair-plan", "reduce", None),
        ("verification-repair-shape-repair", "reduce", None),
        ("blocked-verification-failed", "finalReport", None),
        ("wave-completion-evidence-repair", "reduce", None),
        ("blocked-no-completion", "finalReport", None),
        ("blocked-loop-exhaustion", "finalReport", None),
        ("artifact-inventory", "reduce", None),
        ("save-artifact-inventory", "saveArtifact", None),
        ("adversarial-review", "reduce", None),
        ("review-remediation-inventory", "reduce", None),
        ("review-remediation-inventory-repair", "reduce", None),
        ("blocked-empty-review-remediation", "finalReport", None),
        ("review-remediation-wave", "fanout", WORKTREE),
        ("review-verification-plan", "reduce", None),
        ("blocked-empty-review-verification", "finalReport", None),
        ("review-verification-wave", "parallel", None),
        ("blocked-review-verification-failed", "finalReport", None),
        ("blocked-review-unresolved", "finalReport", None),
        ("blocked-review-not-accepted", "finalReport", None),
        ("final-evidence-reconciliation", "reduce", None),
        ("completion-claim-repair", "reduce", None),
        ("artifact-existence-investigation", "parallel", None),
        ("blocked-final-evidence-reconciliation", "finalReport", None),
        ("require-final-artifacts", "requireArtifact", None),
        ("final-zero-gap-audit", "reduce", None),
        ("final-acceptance-gate", "qualityGate", None),
        ("blocked-final-readiness", "finalReport", None),
        ("final-acceptance-report", "finalReport", None),
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
                method: WorkflowV2HostMethod::parse(method)
                    .unwrap_or_else(|| panic!("static plan method '{method}' must parse")),
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
