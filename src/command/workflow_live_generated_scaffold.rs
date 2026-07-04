use std::path::Path;

use archon_core::config::GeneratedWorkflowConfig;
use archon_workflow::{GeneratedWorkflowLearningContext, WorkflowResult};

use super::workflow_live_generated_contract::generated_prd_contract_js;
use super::workflow_live_task_universe::WorkflowV2TaskUniverse;

#[path = "workflow_live_generated_scaffold_ownership.rs"]
mod workflow_live_generated_scaffold_ownership;
#[path = "workflow_live_generated_scaffold_verification.rs"]
mod workflow_live_generated_scaffold_verification;

use workflow_live_generated_scaffold_ownership::apply_ownership_expansion_lifecycle;
use workflow_live_generated_scaffold_verification::apply_verification_remediation_lifecycle;

pub(super) fn decomposed_prd_scaffold(
    task: &str,
    target_repository_root: Option<&str>,
    task_universe: &WorkflowV2TaskUniverse,
    governed_learning_context: &[GeneratedWorkflowLearningContext],
    generated_config: &GeneratedWorkflowConfig,
) -> WorkflowResult<String> {
    let task_json = serde_json::to_string(task)?;
    let target_repository_root_json = serde_json::to_string(&target_repository_root)?;
    let project_artifact_root_json = serde_json::to_string(&project_artifact_root(task_universe))?;
    let universe_json = serde_json::to_string_pretty(task_universe)?;
    let learning_json = serde_json::to_string_pretty(governed_learning_context)?;
    let max_repair_iterations = generated_config.max_repair_iterations.max(1).min(8);
    let max_investigation_iterations = generated_config.max_investigation_iterations.max(1).min(8);

    let mut source = String::new();
    source.push_str("export default async function workflow(w) {\n");
    source.push_str("  const taskText = ");
    source.push_str(&task_json);
    source.push_str(";\n");
    source.push_str("  const targetRepositoryRoot = ");
    source.push_str(&target_repository_root_json);
    source.push_str(";\n");
    source.push_str("  const projectArtifactRoot = ");
    source.push_str(&project_artifact_root_json);
    source.push_str(";\n");
    source.push_str("  const taskUniverse = ");
    source.push_str(&indent_json_for_js(&universe_json));
    source.push_str(";\n");
    source.push_str("  const governedLearningContext = ");
    source.push_str(&indent_json_for_js(&learning_json));
    source.push_str(";\n");
    source.push_str(&format!(
        "  const maxRepairIterations = {max_repair_iterations};\n  const maxInvestigationIterations = {max_investigation_iterations};\n",
    ));
    source.push_str(
        r#"  const canonicalTaskUniverse = new Set((taskUniverse.tasks || []).map((task) => task.canonical_task_id).filter(Boolean));
  const maxDependencyWaves = Math.max(1, canonicalTaskUniverse.size * 3);
  const implementationEvidence = [];
  const verificationEvidence = [];
  const reviewEvidence = [];
  const artifactEvidence = [];
  const repairAttempts = [];
  const finalEvidenceRepairAttempts = [];
  const discoveryItems = [
    {
      id: "prd-task-review",
      task: "Read the PRD, decomposed task files, implementation slice, and context files. Return structured requirements, dependency evidence, task coverage requirements, verification requirements, artifact requirements, and residual risks. Distinguish repository source paths from project artifact/data paths under projectArtifactRoot.",
      paths: taskUniverse.source_roots || []
    },
    {
      id: "repository-implementation-audit",
      task: "Inspect the target repository for existing implementation relevant to taskText. Return concrete files read, existing evidence, missing work, test commands, and safety concerns. Do not modify files.",
      paths: taskUniverse.source_roots || []
    },
    {
      id: "acceptance-evidence-audit",
      task: "Map every canonical task in taskUniverse to acceptance criteria, required artifacts, provider/data constraints, and focused verification commands. Use governedLearningContext only as sanitized prior-run hints with evidence refs; verify every claim against current PRD/code. Artifact paths must be checked relative to projectArtifactRoot when they are project artifacts. Do not mark implementation tasks accepted from read-only evidence.",
      paths: taskUniverse.source_roots || []
    }
  ];
"#,
    );
    source.push_str(generated_prd_contract_js());
    source.push_str(include_str!("workflow_live_generated_scaffold_noop.js"));
    source.push_str(include_str!(
        "workflow_live_generated_scaffold_remediation.js"
    ));
    source.push_str(include_str!("workflow_live_generated_scaffold_body_a.js"));
    source.push_str(include_str!("workflow_live_generated_scaffold_body_b.js"));
    source = apply_verification_remediation_lifecycle(source);
    source = apply_ownership_expansion_lifecycle(source);
    Ok(source)
}

fn indent_json_for_js(json: &str) -> String {
    json.lines().collect::<Vec<_>>().join("\n  ")
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
