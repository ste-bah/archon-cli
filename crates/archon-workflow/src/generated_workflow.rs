//! Typed records for generated V2 workflow scripts.
//!
//! These are intentionally separate from the legacy generated-`WorkflowSpec`
//! normalizers. Generated V2 workflows are script-first: `workflow.js` owns
//! orchestration, while Rust records the scaffold, host-call manifest, and
//! sanitized terminal learning evidence.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::v2::{WorkflowV2HostCall, WorkflowV2Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneratedWorkflowKind {
    DecomposedPrdScriptScaffold,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowGeneratedScaffold {
    pub schema_version: String,
    pub kind: GeneratedWorkflowKind,
    pub workflow_js: String,
    pub task_universe: serde_json::Value,
    pub scaffold_hash: String,
    #[serde(default)]
    pub prompt_slots: BTreeMap<String, String>,
    #[serde(default)]
    pub host_call_manifest: Vec<WorkflowV2HostCall>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub governed_learning_context: Vec<GeneratedWorkflowLearningContext>,
}

impl WorkflowGeneratedScaffold {
    pub fn decomposed_prd(
        workflow_js: impl Into<String>,
        task_universe: serde_json::Value,
        prompt_slots: BTreeMap<String, String>,
        host_call_manifest: Vec<WorkflowV2HostCall>,
        governed_learning_context: Vec<GeneratedWorkflowLearningContext>,
    ) -> Self {
        let workflow_js = workflow_js.into();
        Self {
            schema_version: "workflow-generated-scaffold-v1".to_string(),
            kind: GeneratedWorkflowKind::DecomposedPrdScriptScaffold,
            scaffold_hash: workflow_scaffold_hash(&workflow_js),
            workflow_js,
            task_universe,
            prompt_slots,
            host_call_manifest,
            governed_learning_context,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowLearningEvidenceRef {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

impl WorkflowLearningEvidenceRef {
    pub fn path(kind: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            call_id: None,
            path: Some(path.into()),
            hash: None,
        }
    }

    pub fn call(kind: impl Into<String>, call_id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            call_id: Some(call_id.into()),
            path: None,
            hash: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowLearningEvent {
    pub schema_version: String,
    pub run_id: String,
    pub scaffold_hash: String,
    pub terminal_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<String>,
    pub prevented_false_completion: bool,
    #[serde(default)]
    pub evidence_refs: Vec<WorkflowLearningEvidenceRef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub call_status_counts: BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub branch_status_counts: BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub failure_class_counts: BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repair_decisions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_gap_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canary_result: Option<String>,
}

impl WorkflowLearningEvent {
    pub fn generated_run(
        run_id: impl Into<String>,
        scaffold_hash: impl Into<String>,
        terminal_status: WorkflowV2Status,
        failure_class: Option<String>,
        prevented_false_completion: bool,
        evidence_refs: Vec<WorkflowLearningEvidenceRef>,
    ) -> Self {
        Self {
            schema_version: "workflow-generated-learning-event-v1".to_string(),
            run_id: run_id.into(),
            scaffold_hash: scaffold_hash.into(),
            terminal_status: status_label(terminal_status).to_string(),
            failure_class,
            prevented_false_completion,
            evidence_refs,
            call_status_counts: BTreeMap::new(),
            branch_status_counts: BTreeMap::new(),
            failure_class_counts: BTreeMap::new(),
            repair_decisions: Vec::new(),
            evidence_gap_refs: Vec::new(),
            canary_result: None,
        }
    }

    pub fn with_runtime_summary(
        mut self,
        call_status_counts: BTreeMap<String, usize>,
        branch_status_counts: BTreeMap<String, usize>,
        failure_class_counts: BTreeMap<String, usize>,
        repair_decisions: Vec<String>,
        evidence_gap_refs: Vec<String>,
        canary_result: Option<String>,
    ) -> Self {
        self.call_status_counts = call_status_counts;
        self.branch_status_counts = branch_status_counts;
        self.failure_class_counts = failure_class_counts;
        self.repair_decisions = repair_decisions;
        self.evidence_gap_refs = evidence_gap_refs;
        self.canary_result = canary_result;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedWorkflowLearningContext {
    pub schema_version: String,
    pub source_run_id: String,
    pub terminal_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<String>,
    pub prevented_false_completion: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<WorkflowLearningEvidenceRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repair_decisions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_gap_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canary_result: Option<String>,
}

impl GeneratedWorkflowLearningContext {
    pub fn from_event(event: &WorkflowLearningEvent) -> Self {
        Self {
            schema_version: "workflow-generated-learning-context-v1".to_string(),
            source_run_id: event.run_id.clone(),
            terminal_status: event.terminal_status.clone(),
            failure_class: event.failure_class.clone(),
            prevented_false_completion: event.prevented_false_completion,
            evidence_refs: event.evidence_refs.clone(),
            repair_decisions: event.repair_decisions.clone(),
            evidence_gap_refs: event.evidence_gap_refs.clone(),
            canary_result: event.canary_result.clone(),
        }
    }
}

pub fn workflow_scaffold_hash(harness_source: &str) -> String {
    use sha2::{Digest, Sha256};

    hex::encode(Sha256::digest(harness_source.trim().as_bytes()))
}

fn status_label(status: WorkflowV2Status) -> &'static str {
    match status {
        WorkflowV2Status::Pending => "pending",
        WorkflowV2Status::Running => "running",
        WorkflowV2Status::Accepted => "accepted",
        WorkflowV2Status::Noop => "noop",
        WorkflowV2Status::NeedsReview => "needs_review",
        WorkflowV2Status::Blocked => "blocked",
        WorkflowV2Status::Failed => "failed",
        WorkflowV2Status::Cancelled => "cancelled",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::v2::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2Status};

    use super::{
        GeneratedWorkflowKind, WorkflowGeneratedScaffold, WorkflowLearningEvent,
        WorkflowLearningEvidenceRef, workflow_scaffold_hash,
    };

    #[test]
    fn generated_scaffold_records_hash_kind_prompt_slots_and_manifest() {
        let workflow_js =
            "export default async function workflow(w) { await w.agent('discover'); }";
        let mut prompt_slots = std::collections::BTreeMap::new();
        prompt_slots.insert(
            "implementation_wave".to_string(),
            "Implement safely.".to_string(),
        );
        let manifest = vec![WorkflowV2HostCall {
            id: "discover".to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        }];

        let scaffold = WorkflowGeneratedScaffold::decomposed_prd(
            workflow_js,
            json!({"tasks": [{"canonical_task_id": "TASK-TDL-001"}]}),
            prompt_slots,
            manifest,
            Vec::new(),
        );

        assert_eq!(
            scaffold.kind,
            GeneratedWorkflowKind::DecomposedPrdScriptScaffold
        );
        assert_eq!(scaffold.scaffold_hash, workflow_scaffold_hash(workflow_js));
        assert_eq!(
            scaffold.prompt_slots.get("implementation_wave"),
            Some(&"Implement safely.".to_string())
        );
        assert_eq!(scaffold.host_call_manifest.len(), 1);
        assert_eq!(scaffold.host_call_manifest[0].id, "discover");
    }

    #[test]
    fn generated_learning_event_serializes_snake_case_terminal_status() {
        let event = WorkflowLearningEvent::generated_run(
            "wf-test",
            "hash-test",
            WorkflowV2Status::NeedsReview,
            Some("final_evidence_gap".to_string()),
            true,
            vec![WorkflowLearningEvidenceRef::call(
                "failed_call",
                "final-audit",
            )],
        );
        let value = serde_json::to_value(&event).expect("learning event serializes");

        assert_eq!(
            value["schema_version"],
            "workflow-generated-learning-event-v1"
        );
        assert_eq!(value["terminal_status"], "needs_review");
        assert_eq!(value["failure_class"], "final_evidence_gap");
        assert_eq!(value["prevented_false_completion"], true);
        assert_eq!(value["evidence_refs"][0]["call_id"], "final-audit");
    }
}
